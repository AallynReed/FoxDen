// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Backend, Color};
use foxden_smbus::i2c::SharedBus;

use super::cards::{
    FUSION2_BLACKWELL_AORUS_WATERFORCE, FUSION2_BLACKWELL_GAMING, FUSION2_BLACKWELL_MASTER_5080,
    FUSION2_BLACKWELL_MASTER_5090D_ICE, FUSION2_BLACKWELL_SINGLE, FUSION2_BLACKWELL_WATERFORCE,
};
use super::common::{
    AMD_GPU_VEN, Card, Driver, Gpu, Io, detect_each, device_id, info, is_amd_gpu_i2c_bus, row,
};

const REG_MODE: u8 = 0x12;
const REG_COLOR: u8 = 0x16;
const MODE_DIRECT: u8 = 0x00;
const MODE_STATIC: u8 = 0x01;
const BRIGHTNESS_MAX: u8 = 0x0A;
const HW_ZONES: usize = 6;
const FAN_LEDS: usize = 8;

#[derive(Clone, Copy, PartialEq)]
enum Layout {
    Single,
    Gaming,
    Waterforce,
    AorusWaterforce,
    Master5080,
    Master5090dIce,
}

impl Layout {
    /// LEDs per UI zone, from SetupZones.
    fn zones(self) -> &'static [usize] {
        match self {
            Layout::Single => &[1],
            Layout::Gaming => &[1, 1, 1, 1],
            Layout::Waterforce => &[1, 1],
            Layout::AorusWaterforce => &[1, 1, 1, 1],
            Layout::Master5080 => &[FAN_LEDS, FAN_LEDS, FAN_LEDS, 1, 1],
            Layout::Master5090dIce => &[1, 1, 1, 1, 1],
        }
    }

    /// Hardware zones the LED update sends, more than the card shows, as GCC does.
    fn hw_zones(self) -> u8 {
        match self {
            Layout::Single => 1,
            Layout::Waterforce => 3,
            Layout::AorusWaterforce => 5,
            _ => 6,
        }
    }

    /// The UI zone a hardware zone takes its colour from; None to skip it.
    fn ui_zone(self, hw: u8) -> Option<usize> {
        match (self, hw) {
            (Layout::AorusWaterforce, 0) => None,
            (Layout::AorusWaterforce, _) => Some(hw as usize - 1),
            (Layout::Master5080 | Layout::Master5090dIce, 5) => Some(4),
            _ => Some(hw as usize),
        }
    }
}

/// Probe.
fn test_for(io: &Io, addr: u8) -> bool {
    let mut probe = [0u8; 64];
    probe[..2].copy_from_slice(&[0x10, 0x01]);
    io.raw_write(addr, &probe);
    match io.raw_read(addr, 4) {
        Some(d) => d[0] == 0x01 && matches!(d[1], 0x01 | 0x02) && d[2] == 0x01,
        None => false,
    }
}

fn detect_layout(bus: &SharedBus, table: &[Card], layout: Layout) -> Vec<Box<dyn Backend>> {
    let i = info(bus);
    if i.pci_vendor == AMD_GPU_VEN && !is_amd_gpu_i2c_bus(&i) {
        return Vec::new();
    }
    detect_each(bus, table, |bus, card| {
        if !test_for(&Io::new(bus), card.addr) {
            return None;
        }
        let leds = layout.zones().iter().sum::<usize>() as u32;
        let dev = row(
            device_id("gigabyte-blackwell", bus, card.addr),
            card.name,
            leds,
        );
        let driver = Blackwell {
            addr: card.addr,
            layout,
            zone_color: [Color::BLACK; HW_ZONES],
        };
        Some(Gpu::boxed(bus, dev, driver))
    })
}

pub fn detect_single(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    detect_layout(bus, FUSION2_BLACKWELL_SINGLE, Layout::Single)
}

pub fn detect_gaming(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    detect_layout(bus, FUSION2_BLACKWELL_GAMING, Layout::Gaming)
}

pub fn detect_waterforce(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    detect_layout(bus, FUSION2_BLACKWELL_WATERFORCE, Layout::Waterforce)
}

pub fn detect_aorus_waterforce(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    detect_layout(
        bus,
        FUSION2_BLACKWELL_AORUS_WATERFORCE,
        Layout::AorusWaterforce,
    )
}

pub fn detect_master_5080(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    detect_layout(bus, FUSION2_BLACKWELL_MASTER_5080, Layout::Master5080)
}

pub fn detect_master_5090d_ice(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    detect_layout(
        bus,
        FUSION2_BLACKWELL_MASTER_5090D_ICE,
        Layout::Master5090dIce,
    )
}

struct Blackwell {
    addr: u8,
    layout: Layout,
    /// Zone colours, never cleared.
    zone_color: [Color; HW_ZONES],
}

impl Blackwell {
    // Zone and mode writes; the speed is never set, sent as 0.
    fn zone(&mut self, io: &Io, hw: u8, colors: &[Color; FAN_LEDS], count: u8) {
        let mode = match self.layout {
            Layout::Master5090dIce => MODE_STATIC,
            _ => MODE_DIRECT,
        };
        let kind = if mode == MODE_DIRECT {
            REG_COLOR
        } else {
            REG_MODE
        };
        if count == 0 {
            self.zone_color[hw as usize] = colors[0];
        }
        let Color(r, g, b) = self.zone_color[hw as usize];
        let mut pkt = [0u8; 64];
        pkt[..11].copy_from_slice(&[kind, 0x01, mode, 0, BRIGHTNESS_MAX, r, g, b, 0, hw, count]);
        let mut at = match self.layout {
            Layout::Master5080 | Layout::Master5090dIce => 11,
            _ => 12,
        };
        for c in &colors[..count as usize] {
            pkt[at..at + 3].copy_from_slice(&[c.0, c.1, c.2]);
            at += 3;
        }
        io.raw_write(self.addr, &pkt);
    }
}

impl Driver for Blackwell {
    fn setup(&mut self, _io: &Io, _colors: &[Color]) {}

    // LED update; colours and count carry across zones
    fn update(&mut self, io: &Io, colors: &[Color], _prev: Option<&[Color]>) {
        let zones = self.layout.zones();
        let mut cfg = [Color::BLACK; FAN_LEDS];
        let mut count = 0u8;
        for hw in 0..self.layout.hw_zones() {
            let Some(ui) = self.layout.ui_zone(hw) else {
                continue;
            };
            if ui >= zones.len() {
                cfg[0] = *colors.last().unwrap();
            } else {
                let start: usize = zones[..ui].iter().sum();
                let n = zones[ui];
                cfg[..n].copy_from_slice(&colors[start..start + n]);
                if cfg[..n].iter().any(|c| *c != cfg[0]) {
                    count = n as u8;
                }
            }
            self.zone(io, hw, &cfg, count);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Fake, ids, push, run, take};
    use super::*;

    const ANSWER: &[u8] = &[0x01, 0x01, 0x01, 0x00];

    fn found(table: &[Card], detect: fn(&SharedBus) -> Vec<Box<dyn Backend>>) -> Box<dyn Backend> {
        let bus = Fake::new("NV", ids(table, table[0].name))
            .raw(0x75, &[ANSWER])
            .shared();
        run(&[detect], &bus).remove(0)
    }

    /// One 64-byte packet as the fake logs it.
    fn pkt(head: &[u8]) -> String {
        let mut p = [0u8; 64];
        p[..head.len()].copy_from_slice(head);
        let hex: Vec<String> = p.iter().map(|b| format!("{b:02x}")).collect();
        format!("wraw 75 {}", hex.join(" "))
    }

    // Probe: 01, 01 or 02, 01.
    #[test]
    fn detect_reply() {
        let t = FUSION2_BLACKWELL_GAMING;
        let bus = |reply: &'static [u8]| {
            Fake::new("NV", ids(t, t[0].name))
                .raw(0x75, &[reply])
                .shared()
        };
        assert_eq!(run(&[detect_gaming], &bus(&[1, 2, 1, 0])).len(), 1);
        assert_eq!(take(), vec![pkt(&[0x10, 0x01]), "rraw 75 4".into()]);
        assert!(run(&[detect_gaming], &bus(&[1, 3, 1, 0])).is_empty());
        assert!(run(&[detect_single], &bus(&[1, 1, 1, 0])).is_empty());
    }

    // Gaming layout: four LEDs, six hardware zones, the last colour fills zones 4 and 5.
    #[test]
    fn gaming_direct() {
        let mut b = found(FUSION2_BLACKWELL_GAMING, detect_gaming);
        let c = [
            Color(1, 2, 3),
            Color(4, 5, 6),
            Color(7, 8, 9),
            Color(10, 11, 12),
        ];
        assert_eq!(
            push(&mut b, &c),
            vec![
                pkt(&[0x16, 1, 0, 0, 0x0A, 1, 2, 3, 0, 0, 0]),
                pkt(&[0x16, 1, 0, 0, 0x0A, 4, 5, 6, 0, 1, 0]),
                pkt(&[0x16, 1, 0, 0, 0x0A, 7, 8, 9, 0, 2, 0]),
                pkt(&[0x16, 1, 0, 0, 0x0A, 10, 11, 12, 0, 3, 0]),
                pkt(&[0x16, 1, 0, 0, 0x0A, 10, 11, 12, 0, 4, 0]),
                pkt(&[0x16, 1, 0, 0, 0x0A, 10, 11, 12, 0, 5, 0]),
            ]
        );
    }

    // AORUS WATERFORCE skips hardware zone 0; 5090 D ICE sends Direct as Static on 0x12.
    #[test]
    fn mapped_layouts() {
        let mut b = found(FUSION2_BLACKWELL_AORUS_WATERFORCE, detect_aorus_waterforce);
        let c = [
            Color(1, 1, 1),
            Color(2, 2, 2),
            Color(3, 3, 3),
            Color(4, 4, 4),
        ];
        let log = push(&mut b, &c);
        assert_eq!(log.len(), 4);
        assert_eq!(log[0], pkt(&[0x16, 1, 0, 0, 0x0A, 1, 1, 1, 0, 1, 0]));
        let mut b = found(FUSION2_BLACKWELL_MASTER_5090D_ICE, detect_master_5090d_ice);
        let c = [
            Color(1, 1, 1),
            Color(2, 2, 2),
            Color(3, 3, 3),
            Color(4, 4, 4),
            Color(5, 5, 5),
        ];
        let log = push(&mut b, &c);
        assert_eq!(log.len(), 6);
        assert_eq!(log[0], pkt(&[0x12, 1, 1, 0, 0x0A, 1, 1, 1, 0, 0, 0]));
        assert_eq!(log[5], pkt(&[0x12, 1, 1, 0, 0x0A, 5, 5, 5, 0, 5, 0]));
    }

    // 5080 MASTER: a fan with mixed colours sends all eight from byte 11, and the count carries on.
    #[test]
    fn master_5080_fans() {
        let mut b = found(FUSION2_BLACKWELL_MASTER_5080, detect_master_5080);
        let mut c = vec![Color(9, 9, 9); 26];
        c[1] = Color(1, 2, 3);
        let log = push(&mut b, &c);
        assert_eq!(log.len(), 6);
        let mut fan0 = vec![0x16, 1, 0, 0, 0x0A, 0, 0, 0, 0, 0, 8];
        for led in &c[..8] {
            fan0.extend([led.0, led.1, led.2]);
        }
        assert_eq!(log[0], pkt(&fan0));
        let mut fan1 = vec![0x16, 1, 0, 0, 0x0A, 0, 0, 0, 0, 1, 8];
        fan1.extend([9; 24]);
        assert_eq!(log[1], pkt(&fan1));
        let mut logo = vec![0x16, 1, 0, 0, 0x0A, 0, 0, 0, 0, 3, 8];
        logo.extend([9; 24]);
        assert_eq!(log[3], pkt(&logo));
    }
}
