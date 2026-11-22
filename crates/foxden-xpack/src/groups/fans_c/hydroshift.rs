//! Lian Li HydroShift II OLED Curve LED AIO.

use super::usb::{self, Shared, Usb, UsbDriver, UsbProto};
use crate::fan::{Channel, FanHub, with_device};
use foxden_core::{Backend, Color, Device, DeviceKind};
use foxden_fan::FanBackend;

pub const NUVOTON: u16 = 0x0416;
pub const PID: u16 = 0x8051;
pub const NAME: &str = "Lian Li HydroShift II OLED Curve LED";
const FAMILY: &str = "lianli-hydroshift2";

pub const EP_OUT: u8 = 0x01;
pub const EP_IN: u8 = 0x81;
const WRITE_MS: u64 = 1000;
const READ_MS: u64 = 2000;
const READ_LEN: usize = 512;
/// Short opcodes take exactly 8 bytes; GetVer and colour pushes 64.
pub const SHORT_LEN: usize = 8;
pub const LONG_LEN: usize = 64;

pub const PUSH_RGB: u8 = 0x11;
pub const GET_TEMPERATURE: u8 = 0x60;
pub const SET_PUMP: u8 = 0x61;
pub const GET_PUMP: u8 = 0x62;
/// SetMBSync, inverted on the wire: 0 follows the motherboard, 1 is software control.
pub const MB_SYNC: u8 = 0x64;

pub const LEDS: usize = 35;
/// LEDs per colour packet, starting at byte 4.
pub const CHUNK: usize = 15;

pub const PUMP_MIN_RPM: f32 = 1600.0;
pub const PUMP_MAX_RPM: f32 = 2400.0;
/// Pump rpm to the firmware's output value (the MCU wants a point on this curve).
pub const PUMP_CURVE: [(u16, u16); 22] = [
    (1577, 250),
    (1608, 300),
    (1663, 400),
    (1725, 500),
    (1783, 600),
    (1838, 700),
    (1895, 800),
    (1950, 900),
    (2013, 1000),
    (2073, 1100),
    (2130, 1200),
    (2188, 1300),
    (2248, 1400),
    (2305, 1500),
    (2365, 1600),
    (2420, 1700),
    (2477, 1800),
    (2537, 1900),
    (2592, 2000),
    (2651, 2100),
    (2703, 2200),
    (2735, 2300),
];

pub fn packet(op: u8, params: &[u8], len: usize) -> Vec<u8> {
    let mut p = vec![0u8; len];
    p[0] = op;
    for (i, b) in params.iter().take(len - 1).enumerate() {
        p[1 + i] = *b;
    }
    p
}

/// Colour packets: `[0x11, first LED, 0, 0, R G B..]`, 15 LEDs each.
pub fn color_packets(colors: &[Color]) -> Vec<Vec<u8>> {
    colors
        .chunks(CHUNK)
        .enumerate()
        .map(|(n, chunk)| {
            let mut p = vec![0u8; LONG_LEN];
            p[0] = PUSH_RGB;
            p[1] = (n * CHUNK) as u8;
            for (i, c) in chunk.iter().enumerate() {
                p[4 + i * 3..7 + i * 3].copy_from_slice(&[c.0, c.1, c.2]);
            }
            p
        })
        .collect()
}

/// Linear interpolation on the pump curve, clamped at both ends.
pub fn pump_output(rpm: u16) -> u16 {
    let (first, last) = (PUMP_CURVE[0], PUMP_CURVE[PUMP_CURVE.len() - 1]);
    if rpm <= first.0 {
        return first.1;
    }
    if rpm >= last.0 {
        return last.1;
    }
    for w in PUMP_CURVE.windows(2) {
        let ((r0, o0), (r1, o1)) = (w[0], w[1]);
        if rpm <= r1 {
            let t = (rpm - r0) as f32 / (r1 - r0) as f32;
            return (o0 as f32 + t * (o1 - o0) as f32).round() as u16;
        }
    }
    last.1
}

/// Duty 0..=1 spans 1600..2400 rpm.
pub fn pump_level(duty: f32) -> u16 {
    let rpm = (PUMP_MIN_RPM + duty.clamp(0.0, 1.0) * (PUMP_MAX_RPM - PUMP_MIN_RPM)).round();
    pump_output(rpm as u16)
}

/// Reported rpm runs high by a load dependent offset.
pub fn corrected_rpm(raw: u16) -> u32 {
    let r = raw as u32;
    match raw {
        0 => 0,
        1..=1800 => r.saturating_sub(40),
        1801..=2500 => r - 50,
        _ => r - 30,
    }
}

/// Write one command and read its reply; a reply for another opcode is a stale one and dropped.
fn exchange(usb: &Usb, p: &[u8]) -> Option<Vec<u8>> {
    usb.bulk_out(EP_OUT, p, WRITE_MS).ok()?;
    for _ in 0..2 {
        let r = usb.bulk_in(EP_IN, READ_LEN, READ_MS).ok()?;
        if r.is_empty() {
            return None;
        }
        if r[0] == p[0] {
            return Some(r);
        }
    }
    None
}

fn open_all() -> Vec<Shared> {
    usb::find(NUVOTON, PID)
        .iter()
        .filter_map(|d| usb::open(d, Some(EP_OUT)).ok())
        .filter(answers)
        .collect()
}

fn answers(s: &Shared) -> bool {
    s.lock()
        .is_ok_and(|u| exchange(&u, &packet(GET_TEMPERATURE, &[], SHORT_LEN)).is_some())
}

struct Lights;

impl UsbProto for Lights {
    fn frame(&mut self, usb: &Usb, colors: &[Color]) -> Result<(), String> {
        for p in color_packets(colors) {
            usb.bulk_out(EP_OUT, &p, WRITE_MS)?;
            usb.bulk_in(EP_IN, READ_LEN, READ_MS)?;
        }
        Ok(())
    }
    fn stop(&mut self, usb: &Usb) {
        let _ = self.frame(usb, &[Color(0, 0, 0); LEDS]);
    }
}

/// The ring, walked clockwise from top centre, evenly spaced.
fn ring(id: String) -> Device {
    let mut d = Device::grid(
        &id,
        NAME,
        DeviceKind::Other,
        [0.62, 0.24, 0.14, 0.24],
        15,
        15,
    );
    d.leds = (0..LEDS)
        .map(|i| {
            let a = i as f32 / LEDS as f32 * std::f32::consts::TAU;
            [0.5 + 0.47 * a.sin(), 0.5 - 0.47 * a.cos()]
        })
        .collect();
    d
}

pub fn lights() -> Vec<Box<dyn Backend>> {
    open_all()
        .into_iter()
        .filter_map(|s| {
            let (id, device) = s
                .lock()
                .map(|u| (u.device_id(FAMILY), u.device.clone()))
                .ok()?;
            let lights = UsbDriver::boxed(s, ring(id), "lian li hydroshift ii", Lights);
            Some(foxden_hid::port::guard(lights, device))
        })
        .collect()
}

struct Hub {
    /// Opened on first use when another process held the device at listing.
    usb: Option<Shared>,
    at: usb::UsbDevice,
    id: String,
    level: Option<u16>,
}

impl Hub {
    fn link(&mut self) -> Result<Shared, String> {
        if self.usb.is_none() {
            self.usb = Some(usb::open(&self.at, Some(EP_OUT))?);
        }
        self.usb.clone().ok_or_else(|| "not open".into())
    }
}

impl FanHub for Hub {
    fn id(&self) -> String {
        self.id.clone()
    }
    fn name(&self) -> String {
        NAME.into()
    }
    fn channels(&self) -> Vec<Channel> {
        vec![Channel::pump("Pump")]
    }
    fn sensors(&self) -> Vec<String> {
        vec!["Liquid Temperature".into()]
    }
    fn rpm(&mut self, _ch: usize) -> Option<u32> {
        let usb = self.link().ok()?;
        let usb = usb.lock().ok()?;
        let r = exchange(&usb, &packet(GET_PUMP, &[], SHORT_LEN))?;
        (r.len() >= 3).then(|| corrected_rpm(u16::from_be_bytes([r[1], r[2]])))
    }
    fn set_duty(&mut self, _ch: usize, duty: f32) -> Result<(), String> {
        let usb = self.link()?;
        let usb = usb.lock().map_err(|e| e.to_string())?;
        // the MCU forgets software mode after a power cycle or L-Connect, so check each time
        let status = exchange(&usb, &packet(GET_TEMPERATURE, &[], SHORT_LEN));
        let synced = status.as_ref().is_none_or(|r| r.get(2) == Some(&0));
        if synced {
            exchange(&usb, &packet(MB_SYNC, &[0x01], SHORT_LEN));
            self.level = None;
        }
        let level = pump_level(duty);
        if self.level == Some(level) {
            return Ok(());
        }
        exchange(&usb, &packet(SET_PUMP, &level.to_be_bytes(), SHORT_LEN))
            .ok_or("pump did not answer")?;
        self.level = Some(level);
        Ok(())
    }
    fn release(&mut self, _ch: usize) -> Result<(), String> {
        Ok(())
    }
    fn temperature(&mut self, _sensor: usize) -> Option<f32> {
        let usb = self.link().ok()?;
        let usb = usb.lock().ok()?;
        let r = exchange(&usb, &packet(GET_TEMPERATURE, &[], SHORT_LEN))?;
        r.get(1).filter(|t| **t > 0).map(|t| *t as f32)
    }
}

/// One hub per WinUSB-bound AIO, kept when another process holds it so it opens later.
pub fn fans() -> Vec<Box<dyn FanBackend>> {
    let mut out = Vec::new();
    for at in usb::find_bound(NUVOTON, PID) {
        let open = usb::open(&at, Some(EP_OUT));
        if open.as_ref().is_ok_and(|s| !answers(s)) {
            continue;
        }
        out.push(with_device(
            Some(usb::lock_for(&at)),
            Hub {
                usb: open.ok(),
                id: at.fan_id(FAMILY),
                at,
                level: None,
            },
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // rpmToOutput / setPumpLevel.
    #[test]
    fn pump_curve() {
        assert_eq!(pump_output(1500), 250);
        assert_eq!(pump_output(3000), 2300);
        assert_eq!(pump_output(2013), 1000);
        assert_eq!(pump_level(0.0), pump_output(1600));
        assert_eq!(pump_level(1.0), pump_output(2400));
        assert_eq!(
            pump_level(0.5),
            900 + ((2000.0 - 1950.0) / 63.0 * 100.0f32).round() as u16
        );
    }

    // readPumpRPM.
    #[test]
    fn rpm_offsets() {
        assert_eq!(corrected_rpm(0), 0);
        assert_eq!(corrected_rpm(1700), 1660);
        assert_eq!(corrected_rpm(2000), 1950);
        assert_eq!(corrected_rpm(2600), 2570);
    }

    // sendColors / buildPacket.
    #[test]
    fn packets() {
        let mut c = vec![Color(0, 0, 0); LEDS];
        c[15] = Color(1, 2, 3);
        let p = color_packets(&c);
        assert_eq!(p.len(), 3);
        assert_eq!(p[1][..7], [0x11, 15, 0, 0, 1, 2, 3]);
        assert_eq!(p[2][1], 30);
        assert_eq!(
            packet(SET_PUMP, &[0x03, 0x84], SHORT_LEN),
            [0x61, 3, 0x84, 0, 0, 0, 0, 0]
        );
    }
}
