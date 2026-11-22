// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Backend, Color};

use super::common::{Driver, Io, Proto, stick};
use super::spd::Slot;
use crate::i2c::SharedBus;

const CMD_FADE_TIME: u8 = 0xA4;
const CMD_MODE: u8 = 0xA6;
const CMD_RED: u8 = 0xB0;
const CMD_GREEN: u8 = 0xB1;
const CMD_BLUE: u8 = 0xB2;
const MODE_SINGLE: u8 = 0x00;

/// Probe: every register 0xA0..0xAF reads 0xBA.
fn test(io: &Io, a: u8) -> bool {
    if io.quick(a) < 0 {
        return false;
    }
    let mut pass = true;
    for i in 0xA0..0xB0 {
        if io.read_byte_data(a, i) != 0xBA {
            pass = false;
        }
    }
    pass
}

/// Detection: each slot at SPD + 8 and SPD - 0x40 + 8.
pub fn detect(
    bus: &SharedBus,
    io: &Io,
    slots: &mut [&mut Slot],
    _name: &'static str,
) -> Vec<Box<dyn Backend>> {
    let mut found: Vec<Box<dyn Backend>> = Vec::new();
    for slot in slots.iter() {
        for a in [slot.addr + 8, slot.addr - 0x40 + 8] {
            if test(io, a) {
                let device = stick(
                    format!("smbus:corsair-vengeance-{a:02x}"),
                    "Corsair Vengeance RGB".into(),
                    slot.index() as usize,
                    1,
                );
                found.push(Box::new(Driver::new(
                    bus,
                    "corsair vengeance",
                    vec![device],
                    Vengeance { dev: a },
                )));
            }
        }
    }
    found
}

pub struct Vengeance {
    dev: u8,
}

impl Proto for Vengeance {
    /// Static mode, the per-LED mode.
    fn start(&mut self, io: &Io, _zones: &[Vec<Color>]) {
        io.write_byte_data(self.dev, CMD_MODE, MODE_SINGLE);
    }

    /// SetLEDColor
    fn frame(&mut self, io: &Io, zones: &[Vec<Color>], _zone: usize) {
        let c = zones[0][0];
        io.write_byte_data(self.dev, CMD_FADE_TIME, 0x00);
        io.write_byte_data(self.dev, CMD_RED, c.0);
        io.write_byte_data(self.dev, CMD_GREEN, c.1);
        io.write_byte_data(self.dev, CMD_BLUE, c.2);
        io.write_byte_data(self.dev, CMD_MODE, MODE_SINGLE);
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Op::*, *};
    use super::super::spd;
    use super::*;

    // Detection on slot 1: 0x59 answers, 0x19 does not
    #[test]
    fn detection_and_frame() {
        let mut sim = Sim::default();
        sim.ddr4.insert(0x51, ddr4_spd([0x02, 0x9E], &[]));
        sim.present.insert(0x59);
        for i in 0xA0..0xB0 {
            sim.bytes.insert((0x59, i), 0xBA);
        }
        let bus = bus(sim);
        let io = Io::new(&bus);
        let (mut slots, _) = spd::scan(&io);
        assert_eq!(slots[0].jedec, spd::JEDEC_CORSAIR);
        take();
        let mut refs: Vec<&mut Slot> = slots.iter_mut().collect();
        let mut found = detect(&bus, &io, &mut refs, "Corsair Vengeance RGB DRAM");
        let mut want = vec![Quick(0x59)];
        want.extend((0xA0..0xB0).map(|i| Rbd(0x59, i)));
        want.push(Quick(0x19));
        assert_eq!(take(), want);
        assert_eq!(found.len(), 1);

        found[0]
            .push("smbus:corsair-vengeance-59", &[Color(7, 8, 9)])
            .unwrap();
        assert_eq!(
            take(),
            vec![
                Wbd(0x59, 0xA6, 0x00),
                Wbd(0x59, 0xA4, 0x00),
                Wbd(0x59, 0xB0, 7),
                Wbd(0x59, 0xB1, 8),
                Wbd(0x59, 0xB2, 9),
                Wbd(0x59, 0xA6, 0x00),
            ]
        );
    }

    // The probe keeps reading after a mismatch
    #[test]
    fn mismatch_reads_all() {
        let mut sim = Sim::default();
        sim.present.insert(0x58);
        let bus = bus(sim);
        assert!(!test(&Io::new(&bus), 0x58));
        assert_eq!(take().len(), 17);
    }
}
