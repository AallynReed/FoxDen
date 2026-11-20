//! Fan control for Winbond W836xx Super I/O chips.

use crate::chip::Chip::{self, *};
use crate::monitor::{Monitor, Values};
use crate::port::Lpc;

const INDEX: u16 = 5;
const DATA: u16 = 6;
const BANK: u8 = 0x4E;
const VENDOR: u16 = 0x5CA3;
const SOURCE_SELECT: u8 = 0x49;
const FAN_BITS: [u8; 5] = [0x47, 0x4B, 0x4C, 0x59, 0x5D];
const DIV0: [u8; 5] = [36, 38, 30, 8, 10];
const DIV1: [u8; 5] = [37, 39, 31, 9, 11];
const DIV2: [u8; 5] = [5, 6, 7, 23, 15];
const TACH_BANK: [u8; 5] = [0, 0, 0, 0, 5];
const TACH: [u8; 5] = [0x28, 0x29, 0x2A, 0x3F, 0x53];
const TEMP_BANK: [u8; 3] = [1, 2, 0];
const TEMP: [u8; 3] = [0x50, 0x50, 0x27];

/// A mode register and the mask that clears its automatic-control bits.
type Gate = (u8, u8);

pub(crate) struct W836xx {
    lpc: Lpc,
    base: u16,
    fans: usize,
    peci: [bool; 3],
    pwm: Vec<u8>,
    /// Up to three mode registers per control, each written once.
    gates: Vec<Vec<Gate>>,
    saved: Vec<Option<(u8, Vec<u8>)>>,
}

impl W836xx {
    /// `None` when the vendor id does not answer as Winbond.
    pub fn new(lpc: Lpc, chip: Chip, base: u16) -> Option<W836xx> {
        let primary: [Gate; 3] = [(0x04, 0xF3), (0x04, 0xCF), (0x12, 0xF9)];
        let (fans, pwm, gates): (usize, Vec<u8>, Vec<Vec<Gate>>) = match chip {
            W83627EHF | W83627DHG | W83627DHGP => (
                5,
                vec![0x01, 0x03, 0x11],
                primary.iter().map(|g| vec![*g]).collect(),
            ),
            W83667HG | W83667HGB => (
                5,
                vec![0x01, 0x03, 0x11],
                vec![
                    vec![primary[0], (0x7C, 0xEF), (0x62, 0xEF)],
                    vec![primary[1], (0x7C, 0xDF)],
                    vec![primary[2], (0x7C, 0xBF), (0x62, 0xDF)],
                ],
            ),
            W83627HF => (3, vec![0x5A, 0x5B], vec![Vec::new(); 2]),
            W83627THF => (
                3,
                vec![0x01, 0x03, 0x11],
                primary.iter().map(|g| vec![*g]).collect(),
            ),
            _ => (3, Vec::new(), Vec::new()),
        };
        let mut w = W836xx {
            lpc,
            base,
            fans,
            peci: [false; 3],
            saved: vec![None; pwm.len()],
            pwm,
            gates,
        };
        if (w.read(0x80, 0x4F) as u16) << 8 | w.read(0, 0x4F) as u16 != VENDOR {
            return None;
        }
        match chip {
            W83667HG | W83667HGB => {
                let flag = w.read(0, SOURCE_SELECT);
                w.peci = [flag & 0x04 != 0, flag & 0x40 != 0, false];
            }
            W83627DHG | W83627DHGP => {
                let flag = w.read(0, SOURCE_SELECT);
                w.peci = [flag & 0x07 != 0, flag & 0x70 != 0, false];
            }
            _ => {}
        }
        Some(w)
    }

    fn select(&self, bank: u8, reg: u8) {
        self.lpc.outb(self.base + INDEX, BANK);
        self.lpc.outb(self.base + DATA, bank);
        self.lpc.outb(self.base + INDEX, reg);
    }

    fn read(&self, bank: u8, reg: u8) -> u8 {
        self.select(bank, reg);
        self.lpc.inb(self.base + DATA)
    }

    fn write(&self, bank: u8, reg: u8, value: u8) {
        self.select(bank, reg);
        self.lpc.outb(self.base + DATA, value);
    }
}

impl Monitor for W836xx {
    fn size(&self) -> (usize, usize, usize) {
        (3, self.fans, self.pwm.len())
    }

    fn update(&mut self, values: &mut Values) {
        for (i, out) in values.temps.iter_mut().enumerate() {
            let mut half = (self.read(TEMP_BANK[i], TEMP[i]) as i8 as i32) << 1;
            if TEMP_BANK[i] > 0 {
                half |= (self.read(TEMP_BANK[i], TEMP[i] + 1) >> 7) as i32;
            }
            let t = half as f32 / 2.0;
            *out = ((-55.0..=125.0).contains(&t) && !self.peci[i]).then_some(t);
        }
        let bits = FAN_BITS
            .iter()
            .fold(0u64, |bits, &reg| bits << 8 | self.read(0, reg) as u64);
        for (i, out) in values.fans.iter_mut().enumerate() {
            let count = self.read(TACH_BANK[i], TACH[i]) as u32;
            let b = |n: u8| ((bits >> n) & 1) as u32;
            let divisor = 1u32 << (b(DIV2[i]) << 2 | b(DIV1[i]) << 1 | b(DIV0[i]));
            *out = match count {
                0 => None,
                0xFF => Some(0.0),
                _ => Some(1.35e6 / (count * divisor) as f32),
            };
        }
    }

    fn set_control(&mut self, index: usize, value: Option<u8>) -> Result<(), String> {
        let gates = self.gates[index].clone();
        match value {
            Some(value) => {
                if self.saved[index].is_none() {
                    let duty = self.read(0, self.pwm[index]);
                    let modes = gates.iter().map(|&(reg, _)| self.read(0, reg)).collect();
                    self.saved[index] = Some((duty, modes));
                }
                for &(reg, mask) in &gates {
                    let now = self.read(0, reg);
                    self.write(0, reg, now & mask);
                }
                self.write(0, self.pwm[index], value);
            }
            None => {
                let Some((duty, modes)) = self.saved[index].take() else {
                    return Ok(());
                };
                for (&(reg, mask), initial) in gates.iter().zip(modes) {
                    let now = self.read(0, reg);
                    self.write(0, reg, (initial & !mask) | now);
                }
                if gates.is_empty() {
                    self.write(0, self.pwm[index], duty);
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

    fn chip(kind: Chip, values: &[(u16, u8)]) -> (Bus, Regs, Option<W836xx>) {
        let bus = Bus::default();
        let regs = regs(values);
        bus.device(BASE, BASE + 7, Banked::new(BASE, &regs));
        let w = W836xx::new(bus.lpc(0x2E), kind, BASE);
        (bus, regs, w)
    }

    fn read(bank: u8, reg: u8) -> Vec<String> {
        vec![
            "pio_outb 295 4e".into(),
            format!("pio_outb 296 {bank:02x}"),
            format!("pio_outb 295 {reg:02x}"),
            "pio_inb 296".into(),
        ]
    }

    fn write(bank: u8, reg: u8, value: u8) -> Vec<String> {
        let mut w = read(bank, reg);
        w[3] = format!("pio_outb 296 {value:02x}");
        w
    }

    const WINBOND: [(u16, u8); 2] = [(0x804F, 0x5C), (0x004F, 0xA3)];

    #[test]
    fn identified_by_vendor_and_peci_inputs() {
        let mut values = WINBOND.to_vec();
        values.push((0x49, 0x40));
        let (bus, _, w) = chip(W83667HG, &values);
        assert_eq!(w.unwrap().peci, [false, true, false]);
        assert_eq!(
            bus.take(),
            [read(0x80, 0x4F), read(0, 0x4F), read(0, 0x49)].concat()
        );
        assert!(chip(W83627EHF, &[]).2.is_none());
    }

    #[test]
    fn temperatures_and_speeds() {
        let mut values = WINBOND.to_vec();
        // CPU at 37.5 in bank 1; fan 1 with divisor bits 0b011 from 0x47 and 0x4B
        values.extend([(0x150, 37), (0x151, 0x80), (0x027, 0xC8)]);
        values.extend([(0x0047, 0x30), (0x004B, 0x00), (0x0028, 0x87)]);
        let (bus, _, w) = chip(W83627DHG, &values);
        let mut w = w.unwrap();
        bus.take();
        let mut v = Values {
            temps: vec![None; 3],
            fans: vec![None; 5],
        };
        w.update(&mut v);
        assert_eq!(v.temps, [Some(37.5), Some(0.0), None]);
        assert_eq!(v.fans[0], Some(1.35e6 / (0x87 as f32 * 8.0)));
        assert_eq!(v.fans[1], None);
        let log = bus.take();
        assert_eq!(log[..8], [read(1, 0x50), read(1, 0x51)].concat());
    }

    #[test]
    fn manual_duty_then_back_to_the_chip() {
        let mut values = WINBOND.to_vec();
        values.extend([
            (0x0012, 0x06),
            (0x007C, 0x40),
            (0x0062, 0x30),
            (0x0011, 0x55),
        ]);
        let (bus, regs, w) = chip(W83667HGB, &values);
        let mut w = w.unwrap();
        bus.take();
        w.set_control(2, Some(0xA0)).unwrap();
        let want = [
            read(0, 0x11),
            read(0, 0x12),
            read(0, 0x7C),
            read(0, 0x62),
            read(0, 0x12),
            write(0, 0x12, 0x00),
            read(0, 0x7C),
            write(0, 0x7C, 0x00),
            read(0, 0x62),
            write(0, 0x62, 0x10),
            write(0, 0x11, 0xA0),
        ]
        .concat();
        assert_eq!(bus.take(), want);
        w.set_control(2, None).unwrap();
        let want = [
            read(0, 0x12),
            write(0, 0x12, 0x06),
            read(0, 0x7C),
            write(0, 0x7C, 0x40),
            read(0, 0x62),
            write(0, 0x62, 0x30),
        ]
        .concat();
        assert_eq!(bus.take(), want);
        assert_eq!(regs.lock().unwrap()[&0x0062], 0x30);
    }

    #[test]
    fn w83627hf_puts_its_duty_back() {
        let mut values = WINBOND.to_vec();
        values.push((0x005A, 0xC0));
        let (bus, _, w) = chip(W83627HF, &values);
        let mut w = w.unwrap();
        bus.take();
        w.set_control(0, Some(0x20)).unwrap();
        w.set_control(0, None).unwrap();
        let want = [read(0, 0x5A), write(0, 0x5A, 0x20), write(0, 0x5A, 0xC0)].concat();
        assert_eq!(bus.take(), want);
    }
}
