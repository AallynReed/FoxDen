// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Backend, Color};

use super::common::{Driver, Io, Proto, stick};
use super::spd::Slot;
use crate::i2c::SharedBus;

const ADDRESSES: [u8; 13] = [
    0x70, 0x71, 0x72, 0x73, 0x74, 0x75, 0x76, 0x78, 0x39, 0x3A, 0x3B, 0x3C, 0x3D,
];
const LED_COUNT: usize = 15;
const REG_DIRECT: u16 = 0xE020;
const REG_APPLY: u16 = 0xE02F;
const REG_SLOT_INDEX: u16 = 0xE0F8;
const REG_I2C_ADDRESS: u16 = 0xE0F9;
const REG_COLORS_DIRECT: u16 = 0xE100;
const APPLY_VAL: u8 = 0x01;

/// Probe.
fn test(io: &Io, a: u8) -> bool {
    let mut res = io.read_byte(a);
    if res < 0 {
        res = io.read_byte_data(a, 0x00);
    }
    if res < 0 {
        return false;
    }
    (0x90..0xA1).all(|i| io.read_byte_data(a, i) == i as i32 - 0x80)
}

/// Detection: move each module off 0x77, then probe the address list.
pub fn detect(
    bus: &SharedBus,
    io: &Io,
    slots: &mut [&mut Slot],
    _name: &'static str,
) -> Vec<Box<dyn Backend>> {
    for slot in slots.iter() {
        let mut idx = slot.index() as isize - 1;
        if !test(io, 0x77) {
            break;
        }
        loop {
            idx += 1;
            if idx as usize >= ADDRESSES.len() || io.quick(ADDRESSES[idx as usize]) < 0 {
                break;
            }
        }
        if (idx as usize) < ADDRESSES.len() {
            io.ene_write(0x77, REG_SLOT_INDEX, slot.index());
            io.ene_write(0x77, REG_I2C_ADDRESS, ADDRESSES[idx as usize] << 1);
        }
    }
    let mut found: Vec<Box<dyn Backend>> = Vec::new();
    for (i, &a) in ADDRESSES.iter().enumerate() {
        if test(io, a) {
            let device = stick(
                format!("smbus:tforce-xtreem-{a:02x}"),
                "T-Force Xtreem RGB".into(),
                i,
                LED_COUNT,
            );
            found.push(Box::new(Driver::new(
                bus,
                "t-force xtreem",
                vec![device],
                Xtreem { dev: a },
            )));
        }
    }
    found
}

/// XTREEM_LED_OFFSET: LEDs alternate between the two ends of the stick.
fn led_offset(x: usize) -> usize {
    if x & 1 != 0 {
        LED_COUNT - 1 - (x >> 1)
    } else {
        x >> 1
    }
}

pub struct Xtreem {
    dev: u8,
}

impl Proto for Xtreem {
    /// SetDirect(true)
    fn start(&mut self, io: &Io, _zones: &[Vec<Color>]) {
        io.ene_write(self.dev, REG_DIRECT, 1);
        io.ene_write(self.dev, REG_APPLY, APPLY_VAL);
    }

    /// SetAllColorsDirect: R, B, G per LED, sent three bytes at a time.
    fn frame(&mut self, io: &Io, zones: &[Vec<Color>], _zone: usize) {
        let mut buf = [0u8; LED_COUNT * 3];
        for (i, c) in zones[0].iter().enumerate() {
            let o = 3 * led_offset(i);
            buf[o..o + 3].copy_from_slice(&[c.0, c.2, c.1]);
        }
        for (k, chunk) in buf.chunks(3).enumerate() {
            let reg = REG_COLORS_DIRECT + 3 * k as u16;
            io.write_word_data(self.dev, 0x00, reg.swap_bytes());
            io.write_block_data(self.dev, 0x03, chunk);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Op::*, *};
    use super::super::spd;
    use super::*;

    /// A bus where ENE modules sit at 0x77 until remapped.
    fn sim_with_remap(modules: usize) -> Sim {
        let mut sim = Sim::default();
        sim.ddr4.insert(0x50, ddr4_spd([0x04, 0xEF], &[]));
        sim.ddr4.insert(0x51, ddr4_spd([0x04, 0xEF], &[]));
        let mut moved: Vec<u8> = Vec::new();
        let mut ptr = 0u16;
        sim.hook = Some(Box::new(move |op| {
            let ene =
                |a: u8, moved: &Vec<u8>| (a == 0x77 && moved.len() < modules) || moved.contains(&a);
            match *op {
                Ww(0x77, 0, p) => {
                    ptr = p;
                    Some(Ok(0))
                }
                Wbd(0x77, 1, v) => {
                    if ptr == 0xF9E0 {
                        moved.push(v >> 1);
                    }
                    Some(Ok(0))
                }
                Rb(a) | Quick(a) if !(0x50..0x58).contains(&a) => Some(if ene(a, &moved) {
                    Ok(0)
                } else {
                    Err("nack".into())
                }),
                Rbd(a, i) if !(0x50..0x58).contains(&a) => Some(if ene(a, &moved) {
                    Ok(i.wrapping_sub(0x80) as u16)
                } else {
                    Err("nack".into())
                }),
                _ => None,
            }
        }));
        sim
    }

    fn test_ops(a: u8) -> Vec<Op> {
        let mut ops = vec![Rb(a)];
        ops.extend((0x90..0xA1).map(|i| Rbd(a, i)));
        ops
    }

    // Detection: remap slots 0 and 1 to 0x70 and 0x71, then probe the list
    #[test]
    fn remap_then_probe() {
        let bus = bus(sim_with_remap(2));
        let io = Io::new(&bus);
        let (mut slots, _) = spd::scan(&io);
        assert_eq!(slots[0].jedec, spd::JEDEC_TEAMGROUP);
        take();
        let mut refs: Vec<&mut Slot> = slots.iter_mut().collect();
        let found = detect(&bus, &io, &mut refs, "T-Force Xtreem DDR4 DRAM");
        let mut want = test_ops(0x77);
        want.extend([
            Quick(0x70),
            Ww(0x77, 0, 0xF8E0),
            Wbd(0x77, 1, 0),
            Ww(0x77, 0, 0xF9E0),
            Wbd(0x77, 1, 0xE0),
        ]);
        want.extend(test_ops(0x77));
        want.extend([
            Quick(0x71),
            Ww(0x77, 0, 0xF8E0),
            Wbd(0x77, 1, 1),
            Ww(0x77, 0, 0xF9E0),
            Wbd(0x77, 1, 0xE2),
        ]);
        want.extend(test_ops(0x70));
        want.extend(test_ops(0x71));
        for a in &ADDRESSES[2..] {
            want.extend([Rb(*a), Rbd(*a, 0)]);
        }
        assert_eq!(take(), want);
        assert_eq!(found.len(), 2);
        assert_eq!(found[1].devices()[0].id, "smbus:tforce-xtreem-71");
    }

    // The 0x77 test stops the remap loop for the remaining slots
    #[test]
    fn no_module_at_77() {
        let bus = bus(sim_with_remap(0));
        let io = Io::new(&bus);
        let (mut slots, _) = spd::scan(&io);
        take();
        let mut refs: Vec<&mut Slot> = slots.iter_mut().collect();
        assert!(detect(&bus, &io, &mut refs, "x").is_empty());
        let ops = take();
        assert_eq!(ops[..2], [Rb(0x77), Rbd(0x77, 0)]);
        assert_eq!(ops[2], Rb(0x70));
    }

    // SetDirect(true), then SetAllColorsDirect with the XTREEM_LED_OFFSET interleave
    #[test]
    fn direct_frame() {
        let mut sim = Sim::default();
        sim.present.insert(0x70);
        let bus = bus(sim);
        let dev = stick("t".into(), "t".into(), 0, LED_COUNT);
        let mut d = Driver::new(&bus, "t", vec![dev], Xtreem { dev: 0x70 });
        let colors: Vec<Color> = (0..15).map(|i| Color(i, 100 + i, 200 + i)).collect();
        d.push("t", &colors).unwrap();
        let mut want = vec![
            Ww(0x70, 0, 0x20E0),
            Wbd(0x70, 1, 1),
            Ww(0x70, 0, 0x2FE0),
            Wbd(0x70, 1, 1),
        ];
        let order = [0u8, 2, 4, 6, 8, 10, 12, 14, 13, 11, 9, 7, 5, 3, 1];
        for (k, &i) in order.iter().enumerate() {
            want.push(Ww(0x70, 0, (0xE100 + 3 * k as u16).swap_bytes()));
            want.push(Blk(0x70, 3, vec![i, 200 + i, 100 + i]));
        }
        assert_eq!(take(), want);
    }
}
