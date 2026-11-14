//! Corsair DRAM lighting (Vengeance RGB, Dominator), ported from OpenRGB's CorsairDRAMController
//! (GPL-2.0-or-later, Adam Honse and contributors).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use foxrgb_core::{Backend, Color, Device, DeviceKind};

use crate::pawnio::Bus;

const REG_COLOR_BLOCK_1: u8 = 0x31;
const REG_COLOR_BLOCK_2: u8 = 0x32;
const REG_BINARY_START: u8 = 0x21;
const REG_GET_BINARY_DATA: u8 = 0x40;
const REG_GET_DEVICE_INFO: u8 = 0x61;

/// Name, product ids, LED count and whether the LEDs run in reverse, from CorsairDRAMDevices.cpp.
const MODELS: &[(&str, &[u16], usize, bool)] = &[
    (
        "Corsair Vengeance RGB Pro DDR4",
        &[0x0100, 0x0101],
        10,
        false,
    ),
    (
        "Corsair Dominator Platinum RGB DDR4",
        &[0x0200, 0x0201],
        12,
        true,
    ),
    (
        "Corsair Vengeance RGB Pro SL DDR4",
        &[0x0300, 0x0301],
        10,
        false,
    ),
    ("Corsair Vengeance RGB RS DDR4", &[0x0400, 0x0401], 6, false),
    (
        "Corsair Dominator Platinum RGB DDR5",
        &[0x0600, 0x0601],
        12,
        true,
    ),
    (
        "Corsair Dominator Titanium RGB DDR5",
        &[0x0800, 0x0801, 0x0810, 0x0811],
        12,
        true,
    ),
    (
        "Corsair Vengeance RGB DDR5",
        &[0x0700, 0x0701, 0x0900, 0x0901, 0x0910, 0x0911],
        10,
        false,
    ),
    (
        "Corsair Vengeance Shugo Series DDR5",
        &[0x0A00, 0x0A01, 0x0A10, 0x0A11],
        10,
        false,
    ),
    ("Corsair Vengeance RGB RS DDR5", &[0x0B00, 0x0B01], 6, false),
];

/// CRC-8, polynomial 0x07, no reflection, as CRCpp's CRC_8.
pub fn crc8(bytes: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &b in bytes {
        crc ^= b;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x07
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// Direct-mode packet: LED count, RGB per LED, CRC.
pub fn direct_packet(colors: &[Color], reverse: bool) -> Vec<u8> {
    let mut p = vec![colors.len() as u8];
    let mut ordered: Vec<Color> = colors.to_vec();
    if reverse {
        ordered.reverse();
    }
    for c in ordered {
        p.extend([c.0, c.1, c.2]);
    }
    p.push(crc8(&p));
    p
}

fn present(bus: &Bus, addr: u8) -> bool {
    matches!(bus.read_byte(addr, 0x43), Ok(0x1A..=0x1C))
        && matches!(bus.read_byte(addr, 0x44), Ok(0x01 | 0x03 | 0x04))
}

/// Product id and protocol version from the device info buffer.
fn device_info(bus: &Bus, addr: u8) -> Result<(u16, u8), String> {
    bus.write_byte(addr, REG_GET_DEVICE_INFO, 0)?;
    bus.write_byte(addr, REG_BINARY_START, 0)?;
    let mut info = [0u8; 32];
    for b in &mut info {
        *b = bus.read_byte(addr, REG_GET_BINARY_DATA)?;
    }
    Ok((u16::from_le_bytes([info[2], info[3]]), info[28]))
}

struct Stick {
    addr: u8,
    leds: usize,
    reverse: bool,
}

pub struct Dram {
    bus: Arc<Mutex<Bus>>,
    stick: Stick,
    device: Device,
    sent: Option<Vec<Color>>,
}

/// Every Corsair DRAM module on the bus, one backend each.
pub fn probe(bus: Arc<Mutex<Bus>>) -> Vec<Box<dyn Backend>> {
    let mut found: Vec<Box<dyn Backend>> = Vec::new();
    let addresses: Vec<u8> = (0x58..=0x5F).chain(0x18..=0x1F).collect();
    for addr in addresses {
        let info = {
            let b = bus.lock().unwrap_or_else(|e| e.into_inner());
            if !present(&b, addr) {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            device_info(&b, addr)
        };
        let Ok((pid, protocol)) = info else { continue };
        // Direct mode needs protocol 4 or later.
        if protocol < 4 {
            continue;
        }
        let (name, leds, reverse) = MODELS
            .iter()
            .find(|(_, pids, ..)| pids.contains(&pid))
            .map(|(name, _, leds, reverse)| (*name, *leds, *reverse))
            .unwrap_or(("Corsair DRAM", 10, false));
        let n = found.len() as f32;
        let rect = [0.62 + n * 0.035, 0.08, 0.025, 0.3];
        let device = Device::grid(
            &format!("smbus:corsair-{addr:02x}"),
            name,
            DeviceKind::Memory,
            rect,
            1,
            leds as u32,
        );
        found.push(Box::new(Dram {
            bus: bus.clone(),
            stick: Stick {
                addr,
                leds,
                reverse,
            },
            device,
            sent: None,
        }));
    }
    found
}

impl Backend for Dram {
    fn name(&self) -> &str {
        "corsair dram"
    }

    fn devices(&self) -> Vec<Device> {
        vec![self.device.clone()]
    }

    fn push(&mut self, device_id: &str, colors: &[Color]) -> Result<(), String> {
        if device_id != self.device.id {
            return Err(format!("unknown device {device_id}"));
        }
        if self.sent.as_deref() == Some(colors) {
            return Ok(());
        }
        let colors = &colors[..colors.len().min(self.stick.leds)];
        let packet = direct_packet(colors, self.stick.reverse);
        let bus = self.bus.lock().unwrap_or_else(|e| e.into_inner());
        bus.write_block(
            self.stick.addr,
            REG_COLOR_BLOCK_1,
            &packet[..packet.len().min(32)],
        )?;
        if packet.len() > 32 {
            bus.write_block(self.stick.addr, REG_COLOR_BLOCK_2, &packet[32..])?;
        }
        self.sent = Some(colors.to_vec());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc8_matches_the_standard_check_value() {
        assert_eq!(crc8(b"123456789"), 0xF4);
    }

    #[test]
    fn ten_leds_fit_one_block() {
        let p = direct_packet(&[Color(1, 2, 3); 10], false);
        assert_eq!(p.len(), 32);
        assert_eq!(p[0], 10);
        assert_eq!(p[1..4], [1, 2, 3]);
        assert_eq!(p[31], crc8(&p[..31]));
    }

    #[test]
    fn reversed_modules_send_the_last_led_first() {
        let p = direct_packet(&[Color(1, 0, 0), Color(2, 0, 0)], true);
        assert_eq!(p[1], 2);
        assert_eq!(p[4], 1);
    }
}
