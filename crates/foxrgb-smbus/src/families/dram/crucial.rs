//! Ported from OpenRGB Controllers/CrucialController (GPL-2.0-or-later).

use foxrgb_core::{Backend, Color};

use super::common::{Driver, Io, Proto, delay, is_dram_smbus, stick};
use crate::i2c::{SharedBus, lock};

const ADDRESSES: [u8; 8] = [0x39, 0x3A, 0x3B, 0x3C, 0x20, 0x21, 0x22, 0x23];
const REG_MICRON_CHECK_1: u16 = 0x1025;
const REG_MICRON_CHECK_2: u16 = 0x1030;
const MODE_STATIC: u8 = 0xCF;

/// strcmp(buf, "Micron") == 0 on a 16-byte buffer.
fn is_micron(buf: &[u8; 16]) -> bool {
    buf[..7] == *b"Micron\0"
}

fn read_string(io: &Io, a: u8, reg: u16) -> [u8; 16] {
    let mut buf = [0u8; 16];
    for (i, b) in buf.iter_mut().enumerate() {
        *b = io.ene_read(a, reg + i as u16);
    }
    buf
}

/// TestForCrucialController
fn test(io: &Io, a: u8) -> bool {
    if io.read_byte(a) < 0 {
        return false;
    }
    let mut pass = true;
    for i in 0xA0..0xB0 {
        if io.read_byte_data(a, i) != i as i32 - 0xA0 {
            pass = false;
        }
    }
    pass && (is_micron(&read_string(io, a, REG_MICRON_CHECK_1))
        || is_micron(&read_string(io, a, REG_MICRON_CHECK_2)))
}

/// DetectCrucialControllers for one bus: move modules off 0x27, then probe the address list.
pub fn detect(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    if !is_dram_smbus(lock(bus).info()) {
        return Vec::new();
    }
    let io = Io::new(bus);
    let mut idx: isize = -1;
    for slot in 0..4u8 {
        if io.read_byte(0x27) < 0 {
            break;
        }
        loop {
            idx += 1;
            if idx as usize >= ADDRESSES.len() || io.read_byte(ADDRESSES[idx as usize]) < 0 {
                break;
            }
        }
        if (idx as usize) < ADDRESSES.len() {
            io.ene_write(0x27, 0x82EE, slot);
            io.ene_write(0x27, 0x82EF, ADDRESSES[idx as usize] << 1);
            io.ene_write(0x27, 0x82F0, 0xF0);
        }
        delay(1);
    }
    let mut found: Vec<Box<dyn Backend>> = Vec::new();
    for (i, &a) in ADDRESSES.iter().enumerate() {
        if test(&io, a) {
            let device = stick(
                format!("smbus:crucial-{a:02x}"),
                "Crucial DRAM".into(),
                i,
                8,
            );
            found.push(Box::new(Driver::new(
                bus,
                "crucial",
                vec![device],
                Crucial { dev: a },
            )));
        }
        delay(1);
    }
    found
}

pub struct Crucial {
    dev: u8,
}

impl Crucial {
    fn write_block(&self, io: &Io, reg: u16, data: &[u8]) {
        io.write_word_data(self.dev, 0x00, reg.swap_bytes());
        if io
            .raw(|b| b.write_block_data(self.dev, 0x03, data))
            .is_err()
        {
            for &b in data {
                io.write_byte_data(self.dev, 0x01, b);
            }
        }
    }
}

impl Proto for Crucial {
    /// DeviceUpdateMode for Direct: SetMode(STATIC), then SetAllColorsEffect.
    fn start(&mut self, io: &Io, zones: &[Vec<Color>]) {
        let d = self.dev;
        io.ene_write(d, 0x820F, MODE_STATIC);
        io.ene_write(d, 0x82EE, 0x00);
        io.ene_write(d, 0x82EF, 0x10);
        io.ene_write(d, 0x82F0, 0x84);
        for (led, c) in zones[0].iter().enumerate() {
            io.ene_write(d, 0x82E9, 1 << led);
            io.ene_write(d, 0x82EA, 0x00);
            io.ene_write(d, 0x82EB, 0x00);
            io.ene_write(d, 0x82EC, 0x00);
            io.ene_write(d, 0x82ED, c.0);
            io.ene_write(d, 0x82EE, c.1);
            io.ene_write(d, 0x82EF, c.2);
            io.ene_write(d, 0x82F0, 0x01);
        }
    }

    /// SendDirectColors: red, green and blue channels as three 8-byte blocks.
    fn frame(&mut self, io: &Io, zones: &[Vec<Color>], _zone: usize) {
        let z = &zones[0];
        let reds: Vec<u8> = z.iter().map(|c| c.0).collect();
        let greens: Vec<u8> = z.iter().map(|c| c.1).collect();
        let blues: Vec<u8> = z.iter().map(|c| c.2).collect();
        self.write_block(io, 0x8300, &reds);
        self.write_block(io, 0x8340, &greens);
        self.write_block(io, 0x8380, &blues);
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Op::*, *};
    use super::*;

    /// One Crucial module at 0x27 that moves when remapped, with "Micron" at `micron_at`.
    fn sim(micron_at: u16) -> Sim {
        let mut sim = Sim::default();
        let mut home = 0x27u8;
        let mut ptr = 0u16;
        let mut pending = 0u8;
        sim.hook = Some(Box::new(move |op| match *op {
            Ww(a, 0, p) if a == home => {
                ptr = p.swap_bytes();
                Some(Ok(0))
            }
            Wbd(a, 1, v) if a == home => {
                match ptr {
                    0x82EF => pending = v >> 1,
                    0x82F0 if v == 0xF0 => home = pending,
                    _ => {}
                }
                Some(Ok(0))
            }
            Rb(a) | Wbd(a, ..) | Ww(a, ..) | Blk(a, ..) => {
                Some(if a == home { Ok(0) } else { Err("nack".into()) })
            }
            Rbd(a, 0x81) if a == home => {
                let i = ptr.wrapping_sub(micron_at) as usize;
                Some(Ok(*b"Micron\0".get(i).unwrap_or(&0x55) as u16))
            }
            Rbd(a, i) if a == home => Some(Ok(i.wrapping_sub(0xA0) as u16)),
            _ => Some(Err("nack".into())),
        }));
        sim
    }

    fn string_ops(a: u8, reg: u16) -> Vec<Op> {
        (0..16)
            .flat_map(|i| [Ww(a, 0, (reg + i).swap_bytes()), Rbd(a, 0x81)])
            .collect()
    }

    // DetectCrucialControllers: remap the module at 0x27 to 0x39, then TestForCrucialController
    #[test]
    fn remap_then_probe() {
        let bus = bus(sim(REG_MICRON_CHECK_1));
        let found = detect(&bus);
        let mut want = vec![
            Rb(0x27),
            Rb(0x39),
            Ww(0x27, 0, 0xEE82),
            Wbd(0x27, 1, 0),
            Ww(0x27, 0, 0xEF82),
            Wbd(0x27, 1, 0x72),
            Ww(0x27, 0, 0xF082),
            Wbd(0x27, 1, 0xF0),
            Sleep(1),
            Rb(0x27),
            Rb(0x39),
        ];
        want.extend((0xA0..0xB0).map(|i| Rbd(0x39, i)));
        want.extend(string_ops(0x39, 0x1025));
        want.push(Sleep(1));
        for a in &ADDRESSES[1..] {
            want.extend([Rb(*a), Sleep(1)]);
        }
        assert_eq!(take(), want);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].devices()[0].id, "smbus:crucial-39");
    }

    // TestForCrucialController falls back to the second "Micron" location
    #[test]
    fn micron_at_second_location() {
        let mut s = sim(REG_MICRON_CHECK_2);
        s.hook.as_mut().unwrap()(&Ww(0x27, 0, 0xEF82));
        s.hook.as_mut().unwrap()(&Wbd(0x27, 1, 0x72));
        s.hook.as_mut().unwrap()(&Ww(0x27, 0, 0xF082));
        s.hook.as_mut().unwrap()(&Wbd(0x27, 1, 0xF0));
        let bus = bus(s);
        assert!(test(&Io::new(&bus), 0x39));
        let ops = take();
        assert_eq!(ops[17..49], string_ops(0x39, 0x1025)[..]);
        assert_eq!(ops[49..], string_ops(0x39, 0x1030)[..]);
        let s = sim(0x2000);
        let bus = super::super::fake::bus(s);
        assert!(!test(&Io::new(&bus), 0x27));
        take();
    }

    fn crucial_at(a: u8, fail_block: bool) -> Driver<Crucial> {
        let mut s = Sim::default();
        s.present.insert(a);
        s.fail_block = fail_block;
        let bus = bus(s);
        let dev = stick("c".into(), "c".into(), 0, 8);
        Driver::new(&bus, "crucial", vec![dev], Crucial { dev: a })
    }

    fn ene(a: u8, reg: u16, v: u8) -> [Op; 2] {
        [Ww(a, 0, reg.swap_bytes()), Wbd(a, 1, v)]
    }

    // DeviceUpdateMode(Direct) then DeviceUpdateLEDs -> SendDirectColors
    #[test]
    fn direct_setup_and_frame() {
        let mut d = crucial_at(0x39, false);
        let colors: Vec<Color> = (0..8).map(|i| Color(i, 10 + i, 20 + i)).collect();
        d.push("c", &colors).unwrap();
        let mut want: Vec<Op> = [
            ene(0x39, 0x820F, 0xCF),
            ene(0x39, 0x82EE, 0),
            ene(0x39, 0x82EF, 0x10),
            ene(0x39, 0x82F0, 0x84),
        ]
        .concat();
        for i in 0..8u8 {
            want.extend(ene(0x39, 0x82E9, 1 << i));
            want.extend(ene(0x39, 0x82EA, 0));
            want.extend(ene(0x39, 0x82EB, 0));
            want.extend(ene(0x39, 0x82EC, 0));
            want.extend(ene(0x39, 0x82ED, i));
            want.extend(ene(0x39, 0x82EE, 10 + i));
            want.extend(ene(0x39, 0x82EF, 20 + i));
            want.extend(ene(0x39, 0x82F0, 1));
        }
        want.extend([
            Ww(0x39, 0, 0x0083),
            Blk(0x39, 3, (0..8).collect()),
            Ww(0x39, 0, 0x4083),
            Blk(0x39, 3, (10..18).collect()),
            Ww(0x39, 0, 0x8083),
            Blk(0x39, 3, (20..28).collect()),
        ]);
        assert_eq!(take(), want);
    }

    // CrucialRegisterWriteBlock falls back to byte writes when the block write fails
    #[test]
    fn block_fallback() {
        let mut d = crucial_at(0x3A, true);
        let a = d.proto.dev;
        let colors = vec![Color(1, 2, 3); 8];
        d.push("c", &colors).unwrap();
        let ops = take();
        let tail = &ops[ops.len() - 3 * 10..];
        assert_eq!(tail[0], Ww(a, 0, 0x0083));
        assert_eq!(tail[1], Blk(a, 3, vec![1; 8]));
        assert_eq!(tail[2..10], vec![Wbd(a, 1, 1); 8][..]);
        assert_eq!(tail[28..], vec![Wbd(a, 1, 3); 2][..]);
    }
}
