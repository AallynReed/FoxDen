// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Backend, Color};

use super::common::{Driver, Io, Proto, stick};
use super::spd::Slot;
use crate::i2c::SharedBus;

const DEV: u8 = 0x77;
const REG_STATIC: u8 = 0x01;
const REG_MODE: u8 = 0x03;
const REG_LED0_EFFECT_COLOR: u8 = 0x3B;
const REG_START: u8 = 0xFF;
const MODE_DIRECT: u8 = 0x07;
const SIGNATURE: [u8; 8] = *b"MICSYS_D";

/// Detection.
pub fn detect(
    bus: &SharedBus,
    io: &Io,
    slots: &mut [&mut Slot],
    _name: &'static str,
) -> Vec<Box<dyn Backend>> {
    if io.quick(DEV) < 0 {
        return Vec::new();
    }
    let mut valid = 0u8;
    for slot in slots.iter_mut() {
        if (0..8).all(|i| slot.manufacturer_data(io, i) == SIGNATURE[i as usize]) {
            valid |= 1 << slot.index();
        }
    }
    if valid == 0 {
        return Vec::new();
    }
    let device = stick(
        format!("smbus:patriot-viper-{DEV:02x}"),
        "Patriot Viper RGB".into(),
        valid.trailing_zeros() as usize,
        5,
    );
    vec![Box::new(Driver::new(
        bus,
        "patriot viper",
        vec![device],
        Viper,
    ))]
}

/// ViperRegisterWrite: the third value is sent as the command of the second write.
fn register_write(io: &Io, reg: u8, v0: u8, v1: u8, v2: u8) {
    io.write_byte_data(DEV, reg, v0);
    io.write_byte_data(DEV, v2, v1);
}

pub struct Viper;

impl Proto for Viper {
    /// SetDirect
    fn start(&mut self, io: &Io, _zones: &[Vec<Color>]) {
        register_write(io, REG_START, 0xFF, 0xFF, 0xFF);
        register_write(io, REG_STATIC, 0x04, 0x00, 0x00);
        register_write(io, REG_MODE, MODE_DIRECT, 0x00, 0x00);
    }

    /// LED update with direct set: the four-argument colour write per LED.
    fn frame(&mut self, io: &Io, zones: &[Vec<Color>], _zone: usize) {
        for (led, c) in zones[0].iter().enumerate() {
            register_write(io, REG_LED0_EFFECT_COLOR + led as u8, c.0, c.2, c.1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Op::*, *};
    use super::super::spd;
    use super::*;

    fn scan_two(sig_page0: bool) -> (SharedBus, Vec<Box<dyn Backend>>) {
        let mut sim = Sim::default();
        let mut e = ddr4_spd([0x85, 0x02], &SIGNATURE);
        if sig_page0 {
            e[0x61..0x69].copy_from_slice(&SIGNATURE);
        }
        sim.ddr4.insert(0x50, e.clone());
        sim.ddr4.insert(0x51, e);
        sim.present.insert(DEV);
        let bus = bus(sim);
        let io = Io::new(&bus);
        let (mut slots, _) = spd::scan(&io);
        assert_eq!(slots[0].jedec, spd::JEDEC_PATRIOT);
        take();
        let mut refs: Vec<&mut Slot> = slots.iter_mut().collect();
        let found = detect(&bus, &io, &mut refs, "Patriot Viper");
        (bus, found)
    }

    // Detection: write_quick 0x77, then manufacturer bytes 0..7 per slot.
    // Each accessor still caches page 1 from its JEDEC read, but the scan of the empty slots left
    // the EE1004 on page 0, so offset 0x61.. is read of page 0 without a page switch.
    #[test]
    fn detection_reads_cached_page() {
        let (_, found) = scan_two(true);
        let mut want = vec![Quick(0x77)];
        for a in [0x50, 0x51] {
            for i in 0..8 {
                want.extend([Rbd(a, 0x61 + i), Sleep(1)]);
            }
        }
        assert_eq!(take(), want);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].devices()[0].leds.len(), 5);
    }

    // The signature checks stop at the first mismatching byte
    #[test]
    fn mismatch_stops_reading() {
        let (_, found) = scan_two(false);
        let ops = take();
        assert_eq!(ops[1..3], [Rbd(0x50, 0x61), Sleep(1)]);
        assert_eq!(ops[3], Rbd(0x51, 0x61));
        assert!(found.is_empty());
    }

    // SetDirect, then SetLEDColor(led, r, g, b) -> ViperRegisterWrite(0x3B + led, r, b, g)
    #[test]
    fn direct_frame() {
        let (_, mut found) = scan_two(true);
        take();
        let colors: Vec<Color> = (0..5).map(|i| Color(i, 10 + i, 20 + i)).collect();
        found[0].push("smbus:patriot-viper-77", &colors).unwrap();
        let mut want = vec![
            Wbd(0x77, 0xFF, 0xFF),
            Wbd(0x77, 0xFF, 0xFF),
            Wbd(0x77, 0x01, 0x04),
            Wbd(0x77, 0x00, 0x00),
            Wbd(0x77, 0x03, 0x07),
            Wbd(0x77, 0x00, 0x00),
        ];
        for i in 0..5u8 {
            want.extend([Wbd(0x77, 0x3B + i, i), Wbd(0x77, 10 + i, 20 + i)]);
        }
        assert_eq!(take(), want);
        found[0].push("smbus:patriot-viper-77", &colors).unwrap();
        assert!(take().is_empty());
    }
}
