// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Backend, Color, Device, DeviceKind};
use foxden_smbus::i2c::SharedBus;

use super::cards::{PNY_EPICX_LARGE, PNY_EPICX_SMALL};
use super::common::{Card, Driver, Gpu, Io, RECT, changed, detect_each, device_id};

const ZONE_ARROW: u8 = 0x02;
const ZONE_FRONT: u8 = 0x04;
const ZONE_LOGO: u8 = 0x0F;
const MODE_DIRECT: u8 = 0x09;

pub fn detect_small(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    detect_each(bus, PNY_EPICX_SMALL, |bus, card| {
        Some(build(bus, card, false))
    })
}

pub fn detect_large(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    detect_each(bus, PNY_EPICX_LARGE, |bus, card| {
        Some(build(bus, card, true))
    })
}

/// SetupZones: (register, index) per LED, with its place on the card.
fn leds(large: bool) -> Vec<(u8, u8, [f32; 2])> {
    let mut v = Vec::new();
    for i in 0..4u8 {
        v.push((ZONE_FRONT, i + 20, [0.02 + i as f32 * 0.04, 0.15]));
    }
    for i in 0..20u8 {
        v.push((ZONE_FRONT, i, [(i as f32 + 0.5) / 20.0, 0.5]));
    }
    let arrow = if large { 19u8 } else { 17 };
    for i in 0..arrow {
        v.push((ZONE_ARROW, i, [(i as f32 + 0.5) / arrow as f32, 0.85]));
    }
    if large {
        v.push((ZONE_LOGO, 0, [0.97, 0.15]));
    }
    v
}

fn build(bus: &SharedBus, card: &Card, large: bool) -> Box<dyn Backend> {
    let map = leds(large);
    let device = Device {
        id: device_id("pny-epicx", bus, card.addr),
        name: card.name.into(),
        kind: DeviceKind::Gpu,
        rect: RECT,
        grid: [20, 3],
        leds: map.iter().map(|l| l.2).collect(),
    };
    let map = map.iter().map(|l| (l.0, l.1)).collect();
    Gpu::boxed(
        bus,
        device,
        EpicX {
            addr: card.addr,
            large,
            map,
        },
    )
}

struct EpicX {
    addr: u8,
    large: bool,
    map: Vec<(u8, u8)>,
}

impl Driver for EpicX {
    // Direct mode: zone mode (9, 0, 0xFF, 0, 0) per zone, then the LEDs.
    fn setup(&mut self, io: &Io, colors: &[Color]) {
        let mut zones = vec![ZONE_FRONT, ZONE_ARROW];
        if self.large {
            zones.push(ZONE_LOGO);
        }
        for z in zones {
            io.write_block(self.addr, z, &[MODE_DIRECT, 0xFF, 0, 0, 0, 0, 0]);
        }
        self.update(io, colors, None);
    }

    fn setup_writes_leds(&self) -> bool {
        true
    }

    // Each changed LED is written on its own.
    fn update(&mut self, io: &Io, colors: &[Color], prev: Option<&[Color]>) {
        for i in changed(colors, prev) {
            let (zone, idx) = self.map[i];
            let c = colors[i];
            io.write_block(
                self.addr,
                zone,
                &[MODE_DIRECT, 0xFF, idx, 0x00, c.0, c.1, c.2],
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Fake, ids, push, run, s, take};
    use super::*;

    // SetupZones: 4 + 20 + 17 LEDs on the small card, 4 + 20 + 19 + 1 on the large one.
    #[test]
    fn layouts() {
        let small = Fake::new(
            "NV",
            ids(PNY_EPICX_SMALL, "PNY GeForce RTX 5070 ARGB Epic-X OC"),
        )
        .shared();
        let found = run(&[detect_small, detect_large], &small);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].devices()[0].leds.len(), 41);
        assert!(take().is_empty());
        let large = Fake::new(
            "NV",
            ids(PNY_EPICX_LARGE, "PNY GeForce RTX 5090 ARGB Epic-X OC"),
        )
        .shared();
        assert_eq!(
            run(&[detect_small, detect_large], &large)[0].devices()[0]
                .leds
                .len(),
            44
        );
    }

    // Direct mode and single-LED writes.
    #[test]
    fn direct_and_frame() {
        let large = Fake::new(
            "NV",
            ids(PNY_EPICX_LARGE, "PNY GeForce RTX 5080 ARGB Epic-X OC"),
        )
        .shared();
        let mut b = run(&[detect_large], &large).remove(0);
        let mut colors = vec![Color::BLACK; 44];
        colors[0] = Color(1, 2, 3);
        colors[4] = Color(4, 5, 6);
        colors[24] = Color(7, 8, 9);
        colors[43] = Color(10, 11, 12);
        let log = push(&mut b, &colors);
        assert_eq!(log.len(), 3 + 44);
        assert_eq!(
            log[..3],
            s(&[
                "wblk 60 04 09 ff 00 00 00 00 00",
                "wblk 60 02 09 ff 00 00 00 00 00",
                "wblk 60 0f 09 ff 00 00 00 00 00",
            ])
        );
        assert_eq!(log[3], "wblk 60 04 09 ff 14 00 01 02 03");
        assert_eq!(log[3 + 4], "wblk 60 04 09 ff 00 00 04 05 06");
        assert_eq!(log[3 + 24], "wblk 60 02 09 ff 00 00 07 08 09");
        assert_eq!(log[3 + 42], "wblk 60 02 09 ff 12 00 00 00 00");
        assert_eq!(log[3 + 43], "wblk 60 0f 09 ff 00 00 0a 0b 0c");
        colors[23] = Color(1, 1, 1);
        assert_eq!(
            push(&mut b, &colors),
            s(&["wblk 60 04 09 ff 13 00 01 01 01"])
        );
    }
}
