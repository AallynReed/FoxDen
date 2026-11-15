//! Ported from OpenRGB Controllers/KingstonFuryDRAMController (GPL-2.0-or-later).

use std::collections::HashMap;

use foxrgb_core::{Backend, Color};

use super::common::{Driver, Io, Proto, delay, stick};
use super::spd::Slot;
use crate::i2c::SharedBus;

const BASE_DDR4: u8 = 0x58;
const BASE_DDR5: u8 = 0x60;
const DELAY: u64 = 10;

const REG_MODEL: u8 = 0x06;
const REG_APPLY: u8 = 0x08;
const REG_MODE: u8 = 0x09;
const REG_INDEX: u8 = 0x0B;
const REG_BRIGHTNESS: u8 = 0x20;
const REG_NUM_SLOTS: u8 = 0x27;
const REG_BASE_RED: u8 = 0x50;
const REG_BASE_GREEN: u8 = 0x51;
const REG_BASE_BLUE: u8 = 0x52;

const BEGIN: u8 = 0x53;
const END: u8 = 0x44;
const MODE_DIRECT: u8 = 0x10;
/// The Direct mode's default brightness in RGBController_KingstonFuryDRAM.
const BRIGHTNESS: u8 = 80;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Test {
    Pass,
    Fail,
    Error,
}

fn ddr4_model(code: u8) -> bool {
    matches!(code, 0x21 | 0x23)
}

fn ddr5_model(code: u8) -> bool {
    matches!(code, 0x10 | 0x15 | 0x11 | 0x12)
}

/// TestDDR5SPD
fn ddr5_spd_model(variant: u8, model: u8) -> bool {
    match variant {
        0x00 => matches!(model, 0x11 | 0x13 | 0x15),
        0x01 => matches!(model, 0x10 | 0x13 | 0x15),
        0x02 | 0x03 => matches!(model, 0x12 | 0x13 | 0x15),
        _ => false,
    }
}

/// TestForFurySignature
fn signature(io: &Io, addr: u8, model_ok: fn(u8) -> bool, spd_identified: bool) -> Test {
    if io.write_byte_data(addr, REG_APPLY, BEGIN) < 0 {
        return Test::Error;
    }
    delay(DELAY);
    let mut result = Test::Pass;
    if !spd_identified {
        for (i, want) in (1u8..=4).zip(*b"FURY") {
            let mut res = -1;
            for _ in 0..3 {
                res = io.read_word_data(addr, i);
                delay(DELAY);
                if (0..0xFFFF).contains(&res) {
                    break;
                }
            }
            if res < 0 || res == 0xFFFF {
                result = Test::Error;
                break;
            }
            if (res >> 8) as u8 != want {
                result = Test::Fail;
                break;
            }
        }
    }
    if !spd_identified && result == Test::Pass {
        let res = io.read_word_data(addr, REG_MODEL);
        delay(DELAY);
        if res < 0 || res == 0xFFFF {
            result = Test::Error;
        } else if !model_ok((res >> 8) as u8) {
            result = Test::Fail;
        }
    }
    if io.write_byte_data(addr, REG_APPLY, END) < 0 {
        return Test::Error;
    }
    delay(DELAY);
    result
}

/// DetectKingstonFuryDRAMControllers: slot indices that answer as Fury modules.
fn find(io: &Io, slots: &mut [&mut Slot], base: u8, model_ok: fn(u8) -> bool) -> Vec<u8> {
    let mut fury = Vec::new();
    for slot in slots.iter_mut() {
        let mut spd_identified = false;
        if base == BASE_DDR5 {
            let variant = slot.manufacturer_data(io, 0x18);
            let model = slot.manufacturer_data(io, 0x19);
            spd_identified = ddr5_spd_model(variant, model);
        }
        let mut result = Test::Error;
        let mut retries = 0;
        while retries < 3 && result == Test::Error {
            result = signature(io, base + slot.index(), model_ok, spd_identified);
            if result == Test::Pass {
                break;
            }
            if result == Test::Error {
                retries += 1;
                delay(DELAY);
            }
        }
        if result == Test::Pass {
            fury.push(slot.index());
        }
    }
    fury
}

fn backend(bus: &SharedBus, name: &'static str, base: u8, slots: Vec<u8>) -> Vec<Box<dyn Backend>> {
    if slots.is_empty() {
        return Vec::new();
    }
    let leds = if base == BASE_DDR4 { 10 } else { 12 };
    let devices = slots
        .iter()
        .enumerate()
        .map(|(i, &s)| {
            stick(
                format!("smbus:kingston-fury-{:02x}", base + s),
                format!("{name}, Fury Slot {}", i + 1),
                s as usize,
                leds,
            )
        })
        .collect();
    let fury = Fury::new(base, slots);
    vec![Box::new(Driver::new(bus, "kingston fury", devices, fury))]
}

pub fn detect_ddr4(
    bus: &SharedBus,
    io: &Io,
    slots: &mut [&mut Slot],
    name: &'static str,
) -> Vec<Box<dyn Backend>> {
    let found = find(io, slots, BASE_DDR4, ddr4_model);
    backend(bus, name, BASE_DDR4, found)
}

pub fn detect_ddr5(
    bus: &SharedBus,
    io: &Io,
    slots: &mut [&mut Slot],
    name: &'static str,
) -> Vec<Box<dyn Backend>> {
    let found = find(io, slots, BASE_DDR5, ddr5_model);
    backend(bus, name, BASE_DDR5, found)
}

/// KingstonFuryDRAMController with its per-slot register cache.
pub struct Fury {
    base: u8,
    slots: Vec<u8>,
    cache: Vec<HashMap<u8, u8>>,
}

impl Fury {
    fn new(base: u8, slots: Vec<u8>) -> Self {
        let cache = vec![HashMap::new(); slots.len()];
        Fury { base, slots, cache }
    }

    fn addr(&self, i: usize) -> u8 {
        self.base + self.slots[i]
    }

    fn per_dimm(&self) -> usize {
        if self.base == BASE_DDR4 { 10 } else { 12 }
    }

    fn smbus_read(&self, io: &Io, i: usize, reg: u8) -> Option<u8> {
        let a = self.addr(i);
        for retries in 1..=5 {
            match io.raw(|b| b.read_word_data(a, reg)) {
                Ok(v) => return Some((v >> 8) as u8),
                Err(_) => delay(3 * retries * DELAY),
            }
        }
        None
    }

    fn smbus_write(&self, io: &Io, i: usize, reg: u8, val: u8) -> bool {
        let a = self.addr(i);
        for retries in 1..=5 {
            if io.raw(|b| b.write_byte_data(a, reg, val)).is_ok() {
                return true;
            }
            delay(3 * retries * DELAY);
        }
        io.note(format!("kingston fury {a:02x} failed to set {reg:02x}"));
        false
    }

    fn cached_read(&mut self, io: &Io, i: usize, reg: u8) -> Option<u8> {
        if let Some(&v) = self.cache[i].get(&reg) {
            return Some(v);
        }
        let v = self.smbus_read(io, i, reg);
        match v {
            Some(v) => {
                self.cache[i].insert(reg, v);
            }
            None => io.note(format!(
                "kingston fury {:02x} failed to get {reg:02x}",
                self.addr(i)
            )),
        }
        v
    }

    fn cached_write(&mut self, io: &Io, i: usize, reg: u8, val: u8) -> bool {
        if self.cache[i].get(&reg) == Some(&val) {
            return false;
        }
        if self.smbus_write(io, i, reg, val) {
            self.cache[i].insert(reg, val);
            return true;
        }
        false
    }

    /// SetRegister with one value per slot.
    fn set_register(&mut self, io: &Io, reg: u8, vals: &[u8]) {
        let mut wrote = false;
        for (i, &v) in vals.iter().enumerate().take(self.slots.len()) {
            wrote = self.cached_write(io, i, reg, v) || wrote;
        }
        if wrote {
            delay(DELAY);
        }
    }

    fn set_all(&mut self, io: &Io, reg: u8, val: u8) {
        let vals = vec![val; self.slots.len()];
        self.set_register(io, reg, &vals);
    }

    fn send_all(&self, io: &Io, reg: u8, val: u8) {
        for i in 0..self.slots.len() {
            self.smbus_write(io, i, reg, val);
        }
    }

    fn send_begin(&self, io: &Io) {
        self.send_all(io, REG_APPLY, BEGIN);
        delay(DELAY);
    }

    fn send_apply(&self, io: &Io) {
        self.send_all(io, REG_APPLY, END);
        delay(DELAY);
    }

    /// SendPreamble; FURY_SYNC is not defined, so every index is 0.
    fn send_preamble(&self, io: &Io) {
        self.send_begin(io);
        self.send_all(io, REG_INDEX, 0);
        delay(DELAY);
        self.send_apply(io);
    }

    fn set_led_colors(&mut self, io: &Io, colors: &[Color]) {
        let per = self.per_dimm();
        if colors.len() != per * self.slots.len() {
            return;
        }
        for led in 0..per {
            let pick = |f: fn(&Color) -> u8| -> Vec<u8> {
                (0..self.slots.len())
                    .map(|s| f(&colors[s * per + led]))
                    .collect()
            };
            let (reds, greens, blues) = (pick(|c| c.0), pick(|c| c.1), pick(|c| c.2));
            let off = 3 * led as u8;
            self.set_register(io, REG_BASE_RED + off, &reds);
            self.set_register(io, REG_BASE_BLUE + off, &blues);
            self.set_register(io, REG_BASE_GREEN + off, &greens);
        }
    }

    /// RGBController_KingstonFuryDRAM::DeviceUpdateLEDs in Direct mode.
    fn update_leds(&mut self, io: &Io, zones: &[Vec<Color>]) {
        self.send_begin(io);
        self.set_all(io, REG_MODE, MODE_DIRECT);
        self.set_led_colors(io, &zones.concat());
        self.set_all(io, REG_BRIGHTNESS, BRIGHTNESS);
        self.set_all(io, REG_NUM_SLOTS, self.slots.len().min(4) as u8);
        self.send_apply(io);
    }
}

impl Proto for Fury {
    const START_SENDS_FRAME: bool = true;

    /// DeviceUpdateMode: preamble only when the stick is not already in Direct.
    fn start(&mut self, io: &Io, zones: &[Vec<Color>]) {
        let mode = self.cached_read(io, 0, REG_MODE).unwrap_or(0);
        if mode != MODE_DIRECT {
            self.send_preamble(io);
        }
        self.update_leds(io, zones);
    }

    fn frame(&mut self, io: &Io, zones: &[Vec<Color>], _zone: usize) {
        self.update_leds(io, zones);
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Op::*, *};
    use super::super::spd;
    use super::*;

    fn fury_ddr4(sim: &mut Sim, a: u8, model: u8) {
        sim.present.insert(a);
        for (i, ch) in (1u8..=4).zip(*b"FURY") {
            sim.words.insert((a, i), (ch as u16) << 8);
        }
        sim.words.insert((a, REG_MODEL), (model as u16) << 8);
    }

    fn signature_ops(a: u8, model: bool) -> Vec<Op> {
        let mut ops = vec![Wbd(a, 0x08, 0x53), Sleep(10)];
        for i in 1..=4 {
            ops.extend([Rw(a, i), Sleep(10)]);
        }
        if model {
            ops.extend([Rw(a, 0x06), Sleep(10)]);
        }
        ops.extend([Wbd(a, 0x08, 0x44), Sleep(10)]);
        ops
    }

    fn two_ddr4() -> (SharedBus, Vec<Box<dyn Backend>>) {
        let mut sim = Sim::default();
        sim.ddr4.insert(0x50, ddr4_spd([0x01, 0x98], &[]));
        sim.ddr4.insert(0x52, ddr4_spd([0x01, 0x98], &[]));
        fury_ddr4(&mut sim, 0x58, 0x23);
        fury_ddr4(&mut sim, 0x5A, 0x21);
        let bus = bus(sim);
        let io = Io::new(&bus);
        let (mut slots, _) = spd::scan(&io);
        take();
        let mut refs: Vec<&mut Slot> = slots.iter_mut().collect();
        let found = detect_ddr4(&bus, &io, &mut refs, "Kingston Fury DDR4 DRAM");
        (bus, found)
    }

    // DetectKingstonFuryDDR4Controllers: signature and model test on each Kingston slot
    #[test]
    fn ddr4_detection() {
        let (_, found) = two_ddr4();
        let ops = take();
        let mut want = signature_ops(0x58, true);
        want.extend(signature_ops(0x5A, true));
        assert_eq!(ops, want);
        assert_eq!(found.len(), 1);
        let ids: Vec<String> = found[0].devices().into_iter().map(|d| d.id).collect();
        assert_eq!(ids, ["smbus:kingston-fury-58", "smbus:kingston-fury-5a"]);
        assert_eq!(found[0].devices()[1].leds.len(), 10);
    }

    // TestForFurySignature: an unknown model code fails without retry
    #[test]
    fn unknown_model_fails() {
        let mut sim = Sim::default();
        fury_ddr4(&mut sim, 0x58, 0x10);
        let bus = bus(sim);
        let io = Io::new(&bus);
        assert_eq!(signature(&io, 0x58, ddr4_model, false), Test::Fail);
        assert_eq!(take(), signature_ops(0x58, true));
    }

    // TestForFurySignature: 0xFFFF is retried three times, then the whole test is retried
    #[test]
    fn ffff_is_retried() {
        let mut sim = Sim::default();
        fury_ddr4(&mut sim, 0x58, 0x23);
        sim.words.insert((0x58, 1), 0xFFFF);
        sim.ddr4.insert(0x50, ddr4_spd([0x01, 0x98], &[]));
        let bus = bus(sim);
        let io = Io::new(&bus);
        let (mut slots, _) = spd::scan(&io);
        take();
        let mut refs: Vec<&mut Slot> = slots.iter_mut().collect();
        assert!(find(&io, &mut refs, BASE_DDR4, ddr4_model).is_empty());
        let one = [
            Wbd(0x58, 0x08, 0x53),
            Sleep(10),
            Rw(0x58, 1),
            Sleep(10),
            Rw(0x58, 1),
            Sleep(10),
            Rw(0x58, 1),
            Sleep(10),
            Wbd(0x58, 0x08, 0x44),
            Sleep(10),
            Sleep(10),
        ];
        assert_eq!(take(), [one.clone(), one.clone(), one].concat());
    }

    // DetectKingstonFuryDDR5Controllers: SPD variant/model at 0x243/0x244 skips the signature
    #[test]
    fn ddr5_spd_identified() {
        let mut manuf = vec![0u8; 0x1A];
        manuf[0x18] = 0x01;
        manuf[0x19] = 0x10;
        let mut sim = Sim::default();
        sim.ddr5.insert(0x51, ddr5_spd([0x01, 0x98], &manuf));
        sim.present.insert(0x61);
        let bus = bus(sim);
        let io = Io::new(&bus);
        let (mut slots, t) = spd::scan(&io);
        assert_eq!((t, slots[0].jedec), (spd::DDR5, spd::JEDEC_KINGSTON));
        take();
        let mut refs: Vec<&mut Slot> = slots.iter_mut().collect();
        let found = detect_ddr5(&bus, &io, &mut refs, "Kingston Fury DDR5 DRAM");
        assert_eq!(
            take(),
            vec![
                Rbd(0x51, 0xC3),
                Sleep(1),
                Rbd(0x51, 0xC4),
                Sleep(1),
                Wbd(0x61, 0x08, 0x53),
                Sleep(10),
                Wbd(0x61, 0x08, 0x44),
                Sleep(10),
            ]
        );
        assert_eq!(found[0].devices()[0].id, "smbus:kingston-fury-61");
        assert_eq!(found[0].devices()[0].leds.len(), 12);
    }

    fn led_ops(colors: &[Color; 2], reds: bool) -> Vec<Op> {
        let mut ops = Vec::new();
        for led in 0..10u8 {
            let mut reg = |r: u8, f: fn(&Color) -> u8| {
                ops.extend([
                    Wbd(0x58, r + 3 * led, f(&colors[0])),
                    Wbd(0x5A, r + 3 * led, f(&colors[1])),
                    Sleep(10),
                ]);
            };
            if reds {
                reg(0x50, |c| c.0);
            }
            reg(0x52, |c| c.2);
            reg(0x51, |c| c.1);
        }
        ops
    }

    // DeviceUpdateMode(Direct): GetMode, SendPreamble, then DeviceUpdateLEDs; later frames are cached
    #[test]
    fn direct_setup_and_frames() {
        let (_, mut found) = two_ddr4();
        take();
        let mut b = found.remove(0);
        let a = [Color(1, 2, 3); 10];
        let c = [Color(4, 5, 6); 10];
        b.push("smbus:kingston-fury-58", &a).unwrap();
        let mut want = vec![
            Rw(0x58, 0x09),
            Wbd(0x58, 0x08, 0x53),
            Wbd(0x5A, 0x08, 0x53),
            Sleep(10),
            Wbd(0x58, 0x0B, 0),
            Wbd(0x5A, 0x0B, 0),
            Sleep(10),
            Wbd(0x58, 0x08, 0x44),
            Wbd(0x5A, 0x08, 0x44),
            Sleep(10),
            Wbd(0x58, 0x08, 0x53),
            Wbd(0x5A, 0x08, 0x53),
            Sleep(10),
            Wbd(0x58, 0x09, 0x10),
            Wbd(0x5A, 0x09, 0x10),
            Sleep(10),
        ];
        want.extend(led_ops(&[Color(1, 2, 3), Color(0, 0, 0)], true));
        want.extend([
            Wbd(0x58, 0x20, 80),
            Wbd(0x5A, 0x20, 80),
            Sleep(10),
            Wbd(0x58, 0x27, 2),
            Wbd(0x5A, 0x27, 2),
            Sleep(10),
            Wbd(0x58, 0x08, 0x44),
            Wbd(0x5A, 0x08, 0x44),
            Sleep(10),
        ]);
        assert_eq!(take(), want);

        b.push("smbus:kingston-fury-58", &a).unwrap();
        assert!(take().is_empty());

        b.push("smbus:kingston-fury-5a", &c).unwrap();
        let mut want = vec![Wbd(0x58, 0x08, 0x53), Wbd(0x5A, 0x08, 0x53), Sleep(10)];
        for led in 0..10u8 {
            want.extend([Wbd(0x5A, 0x50 + 3 * led, 4), Sleep(10)]);
            want.extend([Wbd(0x5A, 0x52 + 3 * led, 6), Sleep(10)]);
            want.extend([Wbd(0x5A, 0x51 + 3 * led, 5), Sleep(10)]);
        }
        want.extend([Wbd(0x58, 0x08, 0x44), Wbd(0x5A, 0x08, 0x44), Sleep(10)]);
        assert_eq!(take(), want);
    }

    // GetMode reading Direct already skips SendPreamble
    #[test]
    fn already_direct_skips_preamble() {
        let (bus, _) = two_ddr4();
        take();
        let mut fury = Fury::new(BASE_DDR4, vec![0, 2]);
        let io = Io::new(&bus);
        fury.cache[0].insert(REG_MODE, MODE_DIRECT);
        fury.start(&io, &[vec![Color(0, 0, 0); 10], vec![Color(0, 0, 0); 10]]);
        let ops = take();
        assert_eq!(ops[0], Wbd(0x58, 0x08, 0x53));
        assert_eq!(ops[3], Wbd(0x5A, 0x09, 0x10));
    }
}
