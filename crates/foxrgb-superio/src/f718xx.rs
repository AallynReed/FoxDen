//! Fan control for Fintek F718xx Super I/O chips.

use crate::chip::Chip::{self, *};
use crate::monitor::{Monitor, Values};
use crate::port::Lpc;

const INDEX: u16 = 5;
const DATA: u16 = 6;
const TEMP_BASE: u8 = 0x70;
const TEMP_CONFIG: u8 = 0x69;
const PWM_ENABLE: u8 = 0x96;
const FAULT: u8 = 0x9F;
const PROGRAM_SELECT: u8 = 0x80;
const PWM: [u8; 4] = [0xA3, 0xB3, 0xC3, 0xD3];
const TACH: [u8; 4] = [0xA0, 0xB0, 0xC0, 0xD0];
const TARGET: [u8; 4] = [0xA2, 0xB2, 0xC2, 0xD2];

#[derive(Clone, Copy)]
enum Saved {
    Duty(u8),
    Mode { enable: u8, target: u16 },
}

pub(crate) struct F718xx {
    lpc: Lpc,
    chip: Chip,
    base: u16,
    mode_select: bool,
    controls: usize,
    saved: Vec<Option<Saved>>,
    fault: Option<u8>,
}

impl F718xx {
    pub fn new(lpc: Lpc, chip: Chip, base: u16) -> F718xx {
        let mode_select = matches!(chip, F71878AD | F71889AD | F71889ED);
        let controls = match chip {
            F71878AD | F71889AD | F71889ED => 3,
            F71882 => 4,
            _ => 0,
        };
        F718xx {
            lpc,
            chip,
            base,
            mode_select,
            controls,
            saved: vec![None; controls],
            fault: None,
        }
    }

    fn read(&self, reg: u8) -> u8 {
        self.lpc.outb(self.base + INDEX, reg);
        self.lpc.inb(self.base + DATA)
    }

    fn write(&self, reg: u8, value: u8) {
        self.lpc.outb(self.base + INDEX, reg);
        self.lpc.outb(self.base + DATA, value);
    }

    fn read_word(&self, reg: u8) -> u16 {
        (self.read(reg) as u16) << 8 | self.read(reg + 1) as u16
    }

    fn write_word(&self, reg: u8, value: u16) {
        self.write(reg, (value >> 8) as u8);
        self.write(reg + 1, value as u8);
    }

    fn f71858_temperature(&self, i: u8) -> Option<f32> {
        let mode = self.read(TEMP_CONFIG) & 3;
        let high = self.read(TEMP_BASE + 2 * i) as u32;
        let low = self.read(TEMP_BASE + 2 * i + 1) as u32;
        if high == 0xBB || high == 0xCC {
            return None;
        }
        let mut bits = match mode {
            2 => (high & 0x80) << 8,
            3 => (low & 0x01) << 15,
            _ => 0,
        };
        bits |= high << 7;
        bits |= (low & 0xE0) >> 1;
        Some((bits & 0xFFF0) as u16 as i16 as f32 / 128.0)
    }
}

impl Monitor for F718xx {
    fn size(&self) -> (usize, usize, usize) {
        let temps = if self.chip == F71808E { 2 } else { 3 };
        let fans = if matches!(self.chip, F71882 | F71858) {
            4
        } else {
            3
        };
        (temps, fans, self.controls)
    }

    fn update(&mut self, values: &mut Values) {
        for (i, out) in values.temps.iter_mut().enumerate() {
            *out = if self.chip == F71858 {
                self.f71858_temperature(i as u8)
            } else {
                let t = self.read(TEMP_BASE + 2 * (i as u8 + 1)) as i8;
                (t > 0 && t < i8::MAX).then_some(t as f32)
            };
        }
        for (i, out) in values.fans.iter_mut().enumerate() {
            let count = self.read_word(TACH[i]) as u32;
            *out = (count > 0).then(|| {
                if count < 0x0FFF {
                    1.5e6 / count as f32
                } else {
                    0.0
                }
            });
        }
    }

    fn set_control(&mut self, index: usize, value: Option<u8>) -> Result<(), String> {
        let mask = 0x03 << (2 * index);
        match value {
            Some(value) => {
                if self.saved[index].is_none() {
                    self.saved[index] = Some(if self.mode_select {
                        if self.fault.is_none() {
                            let fault = self.read(FAULT);
                            self.write(FAULT, fault & !PROGRAM_SELECT);
                            self.fault = Some(fault);
                        }
                        Saved::Mode {
                            enable: self.read(PWM_ENABLE) & mask,
                            target: self.read_word(TARGET[index]),
                        }
                    } else {
                        Saved::Duty(self.read(PWM[index]))
                    });
                }
                if self.mode_select {
                    let enable = self.read(PWM_ENABLE);
                    self.write(PWM_ENABLE, enable | mask);
                }
                self.write(PWM[index], value);
            }
            None => {
                match self.saved[index].take() {
                    Some(Saved::Mode { enable, target }) => {
                        let now = self.read(PWM_ENABLE);
                        self.write(PWM_ENABLE, (now & !mask) | enable);
                        self.write_word(TARGET[index], target);
                    }
                    Some(Saved::Duty(duty)) => self.write(PWM[index], duty),
                    None => return Ok(()),
                }
                if self.saved.iter().all(Option::is_none)
                    && let Some(fault) = self.fault.take()
                {
                    self.write(FAULT, fault);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testbus::{Bus, Flat, Regs, regs};

    const BASE: u16 = 0xA00;

    fn chip(kind: Chip, values: &[(u16, u8)]) -> (Bus, Regs, F718xx) {
        let bus = Bus::default();
        let regs = regs(values);
        bus.device(BASE, BASE + 7, Flat::new(BASE, &regs));
        (bus.clone(), regs, F718xx::new(bus.lpc(0x2E), kind, BASE))
    }

    fn read(reg: u8) -> Vec<String> {
        vec![format!("pio_outb a05 {reg:02x}"), "pio_inb a06".into()]
    }

    fn write(reg: u8, value: u8) -> Vec<String> {
        vec![
            format!("pio_outb a05 {reg:02x}"),
            format!("pio_outb a06 {value:02x}"),
        ]
    }

    #[test]
    fn temperatures_and_speeds() {
        let values = [
            (0x72, 42),
            (0x74, 0xFF),
            (0xA0, 0x02),
            (0xA1, 0xEE),
            (0xB0, 0x0F),
            (0xB1, 0xFF),
        ];
        let (bus, _, mut f) = chip(F71889ED, &values);
        let mut v = Values {
            temps: vec![None; 3],
            fans: vec![None; 3],
        };
        f.update(&mut v);
        assert_eq!(v.temps, [Some(42.0), None, None]);
        assert_eq!(v.fans, [Some(1.5e6 / 750.0), Some(0.0), None]);
        assert_eq!(
            bus.take()[..6],
            [read(0x72), read(0x74), read(0x76)].concat()
        );
    }

    #[test]
    fn f71858_decodes_its_own_format() {
        let (_, _, mut f) = chip(
            F71858,
            &[(0x69, 0), (0x70, 0x32), (0x71, 0x80), (0x72, 0xBB)],
        );
        let mut v = Values {
            temps: vec![None; 3],
            fans: vec![None; 4],
        };
        f.update(&mut v);
        assert_eq!(v.temps[..2], [Some(50.5), None]);
    }

    #[test]
    fn duty_mode_then_back_to_speed_mode() {
        let values = [(0x9F, 0x81), (0x96, 0x24), (0xB2, 0x01), (0xB3, 0x40)];
        let (bus, regs, mut f) = chip(F71889AD, &values);
        f.set_control(1, Some(0x99)).unwrap();
        let want = [
            read(0x9F),
            write(0x9F, 0x01),
            read(0x96),
            read(0xB2),
            read(0xB3),
            read(0x96),
            write(0x96, 0x2C),
            write(0xB3, 0x99),
        ]
        .concat();
        assert_eq!(bus.take(), want);
        f.set_control(1, None).unwrap();
        let want = [
            read(0x96),
            write(0x96, 0x24),
            write(0xB2, 0x01),
            write(0xB3, 0x40),
            write(0x9F, 0x81),
        ]
        .concat();
        assert_eq!(bus.take(), want);
        assert_eq!(regs.lock().unwrap()[&0xB3], 0x40);
    }

    #[test]
    fn f71882_writes_duty_directly() {
        let (bus, _, mut f) = chip(F71882, &[(0xD3, 0x10)]);
        f.set_control(3, Some(0x80)).unwrap();
        f.set_control(3, None).unwrap();
        let want = [read(0xD3), write(0xD3, 0x80), write(0xD3, 0x10)].concat();
        assert_eq!(bus.take(), want);
        assert_eq!(chip(F71862, &[]).2.size(), (3, 3, 0));
    }
}
