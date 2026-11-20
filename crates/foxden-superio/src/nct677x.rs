//! Fan control for Nuvoton NCT610x, NCT677x, NCT679x, NCT5585D and NCT6701D Super I/O chips.

use crate::chip::Chip::{self, *};
use crate::monitor::{Monitor, Values};
use crate::port::Lpc;

const INDEX: u16 = 5;
const DATA: u16 = 6;
const BANK: u8 = 0x4E;
const VENDOR: u16 = 0x5CA3;

/// One temperature input: which source it reports, where it is, and where its source is chosen.
#[derive(Clone, Copy)]
struct Input {
    source: Option<u8>,
    reg: u16,
    half_reg: u16,
    half_bit: i8,
    source_reg: u16,
    alt: Option<u16>,
}

const fn t(source: u8, reg: u16, half_reg: u16, half_bit: i8, source_reg: u16) -> Input {
    Input {
        source: Some(source),
        reg,
        half_reg,
        half_bit,
        source_reg,
        alt: None,
    }
}

const fn ta(source: u8, reg: u16, half_reg: u16, half_bit: i8, source_reg: u16, alt: u16) -> Input {
    Input {
        alt: Some(alt),
        ..t(source, reg, half_reg, half_bit, source_reg)
    }
}

const fn plain(source: u8, reg: u16) -> Input {
    t(source, reg, 0, -1, 0)
}

const fn unnamed(reg: u16, half_reg: u16, half_bit: i8, source_reg: u16) -> Input {
    Input {
        source: None,
        ..t(0, reg, half_reg, half_bit, source_reg)
    }
}

// Source numbers of the NCT67xx family.
const SYSTIN: u8 = 1;
const CPUTIN: u8 = 2;
const AUXTIN0: u8 = 3;
const AUXTIN1: u8 = 4;
const AUXTIN2: u8 = 5;
const AUXTIN3: u8 = 6;
const AUXTIN4: u8 = 7;
const SMBUS0: u8 = 8;
const SMBUS1: u8 = 9;
const TSENSOR: u8 = 10;
const PECI0: u8 = 16;
const PECI1: u8 = 17;
const PCH_MAX: u8 = 18;
const PCH_CHIP: u8 = 19;
const PCH_CPU: u8 = 20;
const PCH_MCH: u8 = 21;
const DIMM00: u8 = 22;
const DIMM01: u8 = 23;
const DIMM10: u8 = 24;
const DIMM11: u8 = 25;
const BYTE0: u8 = 26;
const BYTE1: u8 = 27;
const PECI0_CAL: u8 = 28;
const PECI1_CAL: u8 = 29;
const VIRTUAL: u8 = 31;
const SPARE: u8 = 32;
const SPARE2: u8 = 33;
const AUXTIN5: u8 = 34;

/// PCH, DIMM, byte and calibrated inputs shared by most of the family.
const COMMON_TAIL: [Input; 16] = [
    plain(PECI1, 0x672).with_source_reg(0xC27),
    ta(PCH_MAX, 0x674, 0, -1, 0xC28, 0x400),
    ta(PCH_CHIP, 0x676, 0, -1, 0xC29, 0x401),
    ta(PCH_CPU, 0x678, 0, -1, 0xC2A, 0x402),
    ta(PCH_MCH, 0x67A, 0, -1, 0xC2B, 0x404),
    plain(DIMM00, 0x405),
    plain(DIMM01, 0x406),
    plain(DIMM10, 0x407),
    plain(DIMM11, 0x408),
    plain(BYTE0, 0x419),
    plain(BYTE1, 0x41A),
    plain(PECI0_CAL, 0x4F4),
    plain(PECI1_CAL, 0x4F5),
    plain(VIRTUAL, 0),
    plain(SPARE, 0),
    plain(SPARE2, 0),
];

impl Input {
    const fn with_source_reg(mut self, source_reg: u16) -> Input {
        self.source_reg = source_reg;
        self
    }
}

const SMBUS: [Input; 2] = [
    t(SMBUS0, 0x150, 0x151, 7, 0x622),
    plain(SMBUS1, 0x670).with_source_reg(0xC26),
];

const HEAD: [Input; 7] = [
    t(PECI0, 0x073, 0x074, 7, 0x100),
    ta(CPUTIN, 0x075, 0x076, 7, 0x200, 0x491),
    ta(SYSTIN, 0x077, 0x078, 7, 0x300, 0x490),
    ta(AUXTIN0, 0x079, 0x07A, 7, 0x800, 0x492),
    ta(AUXTIN1, 0x07B, 0x07C, 7, 0x900, 0x493),
    ta(AUXTIN2, 0x07D, 0x07E, 7, 0xA00, 0x494),
    ta(AUXTIN3, 0x4A0, 0x49E, 6, 0xB00, 0x495),
];

fn inputs(chip: Chip) -> Vec<Input> {
    let auxtin4 = plain(AUXTIN4, 0x027).with_source_reg(0x621);
    match chip {
        NCT6771F | NCT6776F => {
            let (peci, cpu, aux, sys) = if chip == NCT6771F {
                (5, 2, 3, 1)
            } else {
                (12, 2, 3, 1)
            };
            vec![
                plain(peci, 0x027).with_source_reg(0x621),
                t(cpu, 0x073, 0x074, 7, 0x100),
                t(aux, 0x075, 0x076, 7, 0x200),
                t(sys, 0x077, 0x078, 7, 0x300),
                unnamed(0x150, 0x151, 7, 0x622),
                unnamed(0x250, 0x251, 7, 0x623),
                unnamed(0x62B, 0x62E, 0, 0x624),
                unnamed(0x62C, 0x62E, 1, 0x625),
                unnamed(0x62D, 0x62E, 2, 0x626),
            ]
        }
        NCT610XD => vec![
            plain(12, 0x06B).with_source_reg(0x621),
            t(2, 0x010, 0x016, 0, 0),
            t(1, 0x011, 0x01B, 1, 0),
            t(3, 0x012, 0x01B, 2, 0),
            t(4, 0x013, 0x016, 3, 0),
            t(5, 0x014, 0x01B, 4, 0),
            t(6, 0x015, 0x01B, 5, 0),
        ],
        NCT6701D => {
            let mut v = vec![
                plain(PECI0, 0x073).with_source_reg(0x100),
                plain(CPUTIN, 0x491),
                plain(SYSTIN, 0x490),
                plain(AUXTIN0, 0x492),
                plain(AUXTIN1, 0x493),
                plain(AUXTIN2, 0x494),
                plain(AUXTIN3, 0x495),
                auxtin4,
            ];
            v.extend_from_slice(&COMMON_TAIL[..5]);
            v.extend_from_slice(&COMMON_TAIL[5..9]);
            v.push(SMBUS[0].no_half());
            v.push(SMBUS[1]);
            v.extend_from_slice(&COMMON_TAIL[9..14]);
            v.push(plain(SPARE, 0x07B).with_source_reg(0x900));
            v.push(plain(SPARE2, 0));
            v.extend([unnamed(0x409, 0, -1, 0), unnamed(0x4A2, 0, -1, 0)]);
            v.push(unnamed(0xC2F, 0, -1, 0));
            v
        }
        NCT6791D | NCT6792D | NCT6793D | NCT6795D => {
            let mut v = HEAD.to_vec();
            v.push(auxtin4);
            v.extend_from_slice(&COMMON_TAIL[..9]);
            v.extend(SMBUS);
            v.extend_from_slice(&COMMON_TAIL[9..]);
            v
        }
        NCT6796D | NCT6796DR | NCT6797D | NCT6798D | NCT6799D => {
            let mut v = HEAD.to_vec();
            v.push(auxtin4);
            if matches!(chip, NCT6798D | NCT6799D) {
                v.push(ta(TSENSOR, 0x4A2, 0x4A1, 7, 0xC00, 0x496));
            }
            v.extend(SMBUS);
            v.extend_from_slice(&COMMON_TAIL);
            v
        }
        NCT6796DS => vec![
            ta(CPUTIN, 0x073, 0x074, 7, 0x100, 0x491),
            ta(SYSTIN, 0x075, 0x076, 7, 0x200, 0x490),
            ta(AUXTIN0, 0x077, 0x078, 7, 0x300, 0x492),
            ta(AUXTIN1, 0x079, 0x07A, 7, 0x800, 0x493),
            ta(AUXTIN2, 0x07B, 0x07C, 7, 0x900, 0x494),
            ta(AUXTIN3, 0x07D, 0x07E, 7, 0xA00, 0x495),
            ta(AUXTIN4, 0x027, 0, 4, 0xC16, 0x496),
            ta(AUXTIN5, 0x449, 0, 4, 0x100, 0x4A2),
            SMBUS[0],
            plain(PECI0, 0x720),
            plain(VIRTUAL, 0),
        ],
        NCT5585D => vec![
            plain(PECI0, 0x720).with_source_reg(0x100),
            ta(CPUTIN, 0x075, 0x076, 7, 0x000, 0x073),
            ta(AUXTIN1, 0x07B, 0x07C, 7, 0x900, 0x493),
            ta(AUXTIN3, 0x4A0, 0x49E, 6, 0xB00, 0x495),
        ],
        _ => vec![
            plain(PECI0, 0x027).with_source_reg(0x621),
            ta(CPUTIN, 0x073, 0x074, 7, 0x100, 0x491),
            ta(SYSTIN, 0x075, 0x076, 7, 0x200, 0x490),
            ta(AUXTIN0, 0x077, 0x078, 7, 0x300, 0x492),
            ta(AUXTIN1, 0x079, 0x07A, 7, 0x800, 0x493),
            ta(AUXTIN2, 0x07B, 0x07C, 7, 0x900, 0x494),
            ta(AUXTIN3, 0x150, 0x151, 7, 0x622, 0x495),
        ],
    }
}

impl Input {
    fn no_half(mut self) -> Input {
        self.half_reg = 0;
        self.half_bit = -1;
        self.alt = None;
        self
    }
}

/// How a chip turns its inputs into temperatures.
#[derive(Clone, Copy, PartialEq)]
enum Style {
    /// Each input on its own, half degrees from a shared register.
    Fixed,
    /// The chosen source is reported once, the first time it is seen.
    Once,
    /// Every input reports its chosen source, later ones overwrite.
    Chosen,
    /// Whole degrees with reserved codes, fixed inputs read first.
    Whole,
}

fn style(chip: Chip) -> Style {
    match chip {
        NCT610XD => Style::Fixed,
        NCT6796D | NCT6796DR | NCT6796DS | NCT6797D | NCT6798D | NCT6799D | NCT5585D => Style::Once,
        NCT6701D => Style::Whole,
        _ => Style::Chosen,
    }
}

pub(crate) struct Nct677x {
    lpc: Lpc,
    chip: Chip,
    base: u16,
    inputs: Vec<Input>,
    temps: usize,
    fans: usize,
    pwm: Vec<u16>,
    command: Vec<u16>,
    mode: Vec<u16>,
    /// The chip's own mode and duty for each control FoxDen has taken.
    saved: Vec<Option<(u8, u8)>>,
}

impl Nct677x {
    /// `None` when the vendor id does not answer as Nuvoton.
    pub fn new(lpc: Lpc, chip: Chip, base: u16) -> Option<Nct677x> {
        let late = matches!(chip, NCT6797D | NCT6798D | NCT6799D | NCT6796DS | NCT5585D);
        let (pwm, command, mode) = if chip == NCT610XD {
            (
                vec![0x04A, 0x04B, 0x04C],
                vec![0x119, 0x129, 0x139],
                vec![0x113, 0x123, 0x133],
            )
        } else {
            let pwm = if late {
                vec![0x001, 0x003, 0x011, 0x013, 0x015, 0xA09, 0xB09]
            } else {
                vec![0x001, 0x003, 0x011, 0x013, 0x015, 0x017, 0x029]
            };
            (
                pwm,
                vec![0x109, 0x209, 0x309, 0x809, 0x909, 0xA09, 0xB09],
                vec![0x102, 0x202, 0x302, 0x802, 0x902, 0xA02, 0xB02],
            )
        };
        let (fans, controls) = match chip {
            NCT6771F => (4, 3),
            NCT6776F => (5, 3),
            NCT610XD => (3, 3),
            NCT6779D => (5, 5),
            NCT6796DR | NCT6796DS | NCT6797D | NCT6798D | NCT6799D | NCT6701D | NCT5585D => (7, 7),
            _ => (6, 6),
        };
        let inputs = inputs(chip);
        let temps = if matches!(chip, NCT6771F | NCT6776F) {
            4
        } else {
            inputs.len()
        };
        let chip = Nct677x {
            lpc,
            chip,
            base,
            inputs,
            temps,
            fans,
            pwm: pwm[..controls].to_vec(),
            command: command[..controls].to_vec(),
            mode: mode[..controls].to_vec(),
            saved: vec![None; controls],
        };
        chip.is_nuvoton().then_some(chip)
    }

    fn read(&self, at: u16) -> u8 {
        self.lpc.outb(self.base + INDEX, BANK);
        self.lpc.outb(self.base + DATA, (at >> 8) as u8);
        self.lpc.outb(self.base + INDEX, at as u8);
        self.lpc.inb(self.base + DATA)
    }

    fn write(&self, at: u16, value: u8) {
        self.lpc.outb(self.base + INDEX, BANK);
        self.lpc.outb(self.base + DATA, (at >> 8) as u8);
        self.lpc.outb(self.base + INDEX, at as u8);
        self.lpc.outb(self.base + DATA, value);
    }

    fn is_nuvoton(&self) -> bool {
        if self.chip == NCT6701D {
            return true;
        }
        let (high, low) = if self.chip == NCT610XD {
            (0x80FE, 0x00FE)
        } else {
            (0x804F, 0x004F)
        };
        (self.read(high) as u16) << 8 | self.read(low) as u16 == VENDOR
    }

    fn temperatures(&self, out: &mut [Option<f32>]) {
        let style = style(self.chip);
        let mut seen = 0u64;
        let mut order: Vec<usize> = (0..self.inputs.len()).collect();
        if style == Style::Whole {
            order.sort_by_key(|&i| self.inputs[i].source_reg > 0);
        }
        let n = out.len();
        let report = |out: &mut [Option<f32>], source: u8, value: Option<f32>| {
            for (j, input) in self.inputs.iter().enumerate().take(n) {
                if input.source == Some(source) {
                    out[j] = value;
                }
            }
        };
        for i in order {
            let input = self.inputs[i];
            match style {
                Style::Fixed => {
                    let mut value = self.read(input.reg) as i8 as f32;
                    if input.half_bit >= 0 {
                        value += 0.5 * ((self.read(input.half_reg) >> input.half_bit) & 1) as f32;
                    }
                    out[i] = Some(value);
                }
                Style::Whole => {
                    let Some(configured) = input.source else {
                        out[i] = (input.reg != 0)
                            .then(|| whole(self.read(input.reg)))
                            .flatten();
                        continue;
                    };
                    let source = if input.source_reg > 0 {
                        let source = self.read(input.source_reg);
                        if !self.inputs.iter().any(|x| x.source == Some(source)) {
                            continue;
                        }
                        source
                    } else {
                        configured
                    };
                    if seen & bit(source) != 0 || input.reg == 0 {
                        continue;
                    }
                    let Some(value) = whole(self.read(input.reg)) else {
                        continue;
                    };
                    seen |= bit(source);
                    report(out, source, Some(value));
                }
                Style::Once | Style::Chosen => {
                    if input.reg == 0 {
                        continue;
                    }
                    let mut half = (self.read(input.reg) as i8 as i32) << 1;
                    if input.half_bit > 0 {
                        half |= ((self.read(input.half_reg) >> input.half_bit) & 1) as i32;
                    }
                    let source = match (input.source_reg, style) {
                        (0, _) => input.source.unwrap_or(0),
                        (reg, Style::Once) => self.read(reg) & 0x1F,
                        (reg, _) => self.read(reg),
                    };
                    if style == Style::Once && seen & bit(source) != 0 {
                        continue;
                    }
                    let value = 0.5 * half as f32;
                    let value = (-55.0..=125.0).contains(&value).then_some(value);
                    if style == Style::Chosen || value.is_some() {
                        seen |= bit(source);
                    }
                    report(out, source, value);
                }
            }
        }
        for (i, input) in self.inputs.iter().enumerate().take(n) {
            let (Some(alt), Some(source)) = (input.alt, input.source) else {
                continue;
            };
            if seen & bit(source) != 0 {
                continue;
            }
            let value = self.read(alt) as i8 as f32;
            out[i] = (value > 0.0 && value <= 125.0).then_some(value);
        }
    }

    fn speeds(&self, out: &mut [Option<f32>]) {
        for (i, rpm) in out.iter_mut().enumerate() {
            *rpm = match self.chip {
                NCT6771F | NCT6776F | NCT610XD => {
                    let (reg, counter) = match self.chip {
                        NCT610XD => (0x030 + 2 * i as u16, 0x1FFF),
                        NCT6771F => (0x656 + 2 * i as u16, 0xFFFF),
                        _ => (0x656 + 2 * i as u16, 0x1FFF),
                    };
                    let value = (self.read(reg) as u32) << 8 | self.read(reg + 1) as u32;
                    let least = (1.35e6 / counter as f64) as u32;
                    Some(if value > least { value as f32 } else { 0.0 })
                }
                _ => {
                    const COUNT: [u16; 7] = [0x4B0, 0x4B2, 0x4B4, 0x4B6, 0x4B8, 0x4BA, 0x4CC];
                    let high = self.read(COUNT[i]);
                    let low = self.read(COUNT[i] + 1);
                    count13(high, low)
                }
            };
        }
    }
}

/// A 13-bit tachometer count as rpm; `None` when the count is too small to be real.
pub(crate) fn count13(high: u8, low: u8) -> Option<f32> {
    let count = (high as u32) << 5 | (low & 0x1F) as u32;
    if count >= 0x1FFF {
        Some(0.0)
    } else if count >= 0x15 {
        Some(1.35e6 / count as f32)
    } else {
        None
    }
}

fn bit(source: u8) -> u64 {
    1u64.checked_shl(source as u32).unwrap_or(0)
}

/// Whole degrees, with the codes the chip uses for "no reading" left out.
fn whole(raw: u8) -> Option<f32> {
    (!matches!(raw, 0x00 | 0xA0 | 0x7E..=0x80)).then_some(raw as i8 as f32)
}

impl Monitor for Nct677x {
    fn size(&self) -> (usize, usize, usize) {
        (self.temps, self.fans, self.pwm.len())
    }

    fn update(&mut self, values: &mut Values) {
        if self.chip.has_io_lock() && !self.is_nuvoton() {
            self.lpc.nuvoton_enter();
            self.lpc.nuvoton_unlock();
            self.lpc.nuvoton_exit();
        }
        self.temperatures(&mut values.temps);
        self.speeds(&mut values.fans);
    }

    fn set_control(&mut self, index: usize, value: Option<u8>) -> Result<(), String> {
        match value {
            Some(value) => {
                if self.saved[index].is_none() {
                    self.saved[index] =
                        Some((self.read(self.mode[index]), self.read(self.command[index])));
                }
                self.write(self.mode[index], 0);
                self.write(self.command[index], value);
            }
            None => {
                if let Some((mode, command)) = self.saved[index].take() {
                    self.write(self.mode[index], mode);
                    self.write(self.command[index], command);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testbus::{Banked, Bus, Regs, regs};

    const BASE: u16 = 0x290;

    fn chip(chip: Chip, values: &[(u16, u8)]) -> (Bus, Regs, Option<Nct677x>) {
        let bus = Bus::default();
        let regs = regs(values);
        bus.device(BASE, BASE + 7, Banked::new(BASE, &regs));
        let found = Nct677x::new(bus.lpc(0x2E), chip, BASE);
        (bus, regs, found)
    }

    fn read(at: u16) -> Vec<String> {
        vec![
            "pio_outb 295 4e".into(),
            format!("pio_outb 296 {:02x}", at >> 8),
            format!("pio_outb 295 {:02x}", at & 0xFF),
            "pio_inb 296".into(),
        ]
    }

    fn write(at: u16, value: u8) -> Vec<String> {
        vec![
            "pio_outb 295 4e".into(),
            format!("pio_outb 296 {:02x}", at >> 8),
            format!("pio_outb 295 {:02x}", at & 0xFF),
            format!("pio_outb 296 {value:02x}"),
        ]
    }

    const VENDOR_REGS: [(u16, u8); 2] = [(0x804F, 0x5C), (0x004F, 0xA3)];

    #[test]
    fn needs_the_vendor_id() {
        let (bus, _, found) = chip(NCT6798D, &[]);
        assert!(found.is_none());
        assert_eq!(bus.take(), [read(0x804F), read(0x004F)].concat());
        let (_, _, found) = chip(NCT6798D, &VENDOR_REGS);
        assert!(found.is_some());
        let (bus, _, found) = chip(NCT610XD, &[(0x80FE, 0x5C), (0x00FE, 0xA3)]);
        assert!(found.is_some());
        assert_eq!(bus.take(), [read(0x80FE), read(0x00FE)].concat());
    }

    #[test]
    fn speeds_from_13_bit_counts() {
        let mut values = VENDOR_REGS.to_vec();
        // 1.35e6 / 0x2A3 = 2000 rpm; 0x1FFF counts as stopped; tiny counts are not real
        values.extend([(0x4B0, 0x15), (0x4B1, 0x03), (0x4B2, 0xFF), (0x4B3, 0xFF)]);
        values.extend([(0x4B4, 0x00), (0x4B5, 0x01)]);
        let (bus, _, found) = chip(NCT6798D, &values);
        let found = found.unwrap();
        bus.take();
        let mut fans = vec![None; 7];
        found.speeds(&mut fans);
        assert_eq!(fans[0], Some(1.35e6 / 0x2A3 as f32));
        assert_eq!(fans[1], Some(0.0));
        assert_eq!(fans[2], None);
        let log = bus.take();
        assert_eq!(log[..8], [read(0x4B0), read(0x4B1)].concat());
        assert_eq!(log.len(), 7 * 8);
    }

    #[test]
    fn speeds_from_16_bit_rpm() {
        let mut values = vec![(0x804F, 0x5C), (0x004F, 0xA3)];
        values.extend([(0x656, 0x04), (0x657, 0xB0), (0x658, 0x00), (0x659, 0x10)]);
        let (_, _, found) = chip(NCT6776F, &values);
        let mut fans = vec![None; 5];
        found.unwrap().speeds(&mut fans);
        assert_eq!(fans[..2], [Some(1200.0), Some(0.0)]);
    }

    #[test]
    fn temperatures_follow_the_chosen_sources() {
        let mut values = VENDOR_REGS.to_vec();
        // PECI 0 input at 0x073 reports CPUTIN (2) at 45.5; CPUTIN's own input then repeats it
        values.extend([(0x073, 45), (0x074, 0x80), (0x100, 2)]);
        values.extend([(0x075, 30), (0x076, 0x00), (0x200, 2)]);
        // SYSTIN reads 33 from its alternate register; AUXTIN0 is out of range
        values.extend([(0x490, 33), (0x079, 0x81), (0x800, 3)]);
        let (bus, _, found) = chip(NCT6798D, &values);
        let found = found.unwrap();
        bus.take();
        let mut temps = vec![None; found.inputs.len()];
        found.temperatures(&mut temps);
        assert_eq!(temps[0], None);
        assert_eq!(temps[1], Some(45.5));
        assert_eq!(temps[2], Some(33.0));
        assert_eq!(temps[3], None);
        let log = bus.take();
        let first = [read(0x073), read(0x074), read(0x100)].concat();
        assert_eq!(log[..12], first);
        // the CPUTIN input is skipped once its source has been seen
        assert_eq!(
            log[12..24],
            [read(0x075), read(0x076), read(0x200)].concat()
        );
        assert_eq!(log[24..28], read(0x077));
    }

    #[test]
    fn older_chips_overwrite_and_read_their_source_without_a_mask() {
        let mut values = vec![(0x804F, 0x5C), (0x004F, 0xA3)];
        values.extend([(0x027, 50), (0x621, 0x45)]);
        values.extend([(0x073, 40), (0x074, 0x80), (0x100, 2)]);
        let (_, _, found) = chip(NCT6779D, &values);
        let found = found.unwrap();
        let mut temps = vec![None; 7];
        found.temperatures(&mut temps);
        assert_eq!(temps[0], None);
        assert_eq!(temps[1], Some(40.5));
    }

    #[test]
    fn nct6701d_keeps_whole_degrees_and_drops_reserved_codes() {
        let (_, _, found) = chip(NCT6701D, &[(0x491, 41), (0x490, 0x7F), (0x409, 52)]);
        let found = found.unwrap();
        let mut temps = vec![None; found.inputs.len()];
        found.temperatures(&mut temps);
        assert_eq!(temps[1], Some(41.0));
        assert_eq!(temps[2], None);
        assert_eq!(temps[26], Some(52.0));
    }

    #[test]
    fn nct610x_reads_each_input_with_its_half() {
        let values = [(0x80FE, 0x5C), (0x00FE, 0xA3), (0x011, 37), (0x01B, 0b10)];
        let (_, _, found) = chip(NCT610XD, &values);
        let mut temps = vec![None; 7];
        found.unwrap().temperatures(&mut temps);
        assert_eq!(temps[2], Some(37.5));
    }

    #[test]
    fn manual_duty_then_back_to_the_chip() {
        let mut values = VENDOR_REGS.to_vec();
        values.extend([(0x202, 0x40), (0x209, 0x99)]);
        let (bus, regs, found) = chip(NCT6798D, &values);
        let mut found = found.unwrap();
        bus.take();
        found.set_control(1, Some(0x80)).unwrap();
        let want = [
            read(0x202),
            read(0x209),
            write(0x202, 0),
            write(0x209, 0x80),
        ]
        .concat();
        assert_eq!(bus.take(), want);
        found.set_control(1, Some(0x90)).unwrap();
        assert_eq!(bus.take(), [write(0x202, 0), write(0x209, 0x90)].concat());
        found.set_control(1, None).unwrap();
        assert_eq!(
            bus.take(),
            [write(0x202, 0x40), write(0x209, 0x99)].concat()
        );
        assert_eq!(regs.lock().unwrap()[&0x202], 0x40);
        found.set_control(1, None).unwrap();
        assert!(bus.take().is_empty());
    }

    #[test]
    fn locked_io_space_is_opened_through_the_config_port() {
        let mut values = VENDOR_REGS.to_vec();
        values.push((0x4B0, 0));
        let (bus, regs, found) = chip(NCT6798D, &values);
        let mut found = found.unwrap();
        bus.config(0, 0x28, 0x13);
        regs.lock().unwrap().clear();
        bus.take();
        let mut values = Values {
            temps: vec![None; found.temps],
            fans: vec![None; 7],
        };
        found.update(&mut values);
        let log = bus.take();
        let want = [
            read(0x804F),
            read(0x004F),
            crate::testbus::log(&[
                "pio_outb 2e 87",
                "pio_outb 2e 87",
                "superio_inb 28",
                "superio_outb 28 03",
                "pio_outb 2e aa",
            ]),
        ]
        .concat();
        assert_eq!(log[..13], want);
    }
}
