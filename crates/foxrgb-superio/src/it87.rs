//! Fan control for ITE IT86xx, IT87xx and IT879x Super I/O chips.

use crate::chip::Chip::{self, *};
use crate::monitor::{Monitor, Values};
use crate::port::Lpc;

const INDEX: u16 = 5;
const DATA: u16 = 6;
const CONFIG: u8 = 0x00;
const BANK: u8 = 0x06;
const TACH_16BIT: u8 = 0x0C;
const TACH_16BIT_ALT: u8 = 0x0B;
const TACH_DIVISOR: u8 = 0x0B;
const MAIN_CONTROL: u8 = 0x13;
const TEMP_BASE: u8 = 0x29;
const VENDOR: u8 = 0x58;
const VENDORS: [u8; 2] = [0x90, 0x7F];
const PWM_EXT: [u8; 6] = [0x63, 0x6B, 0x73, 0x7B, 0xA3, 0xAB];
const TACH: [u8; 6] = [0x0D, 0x0E, 0x0F, 0x80, 0x82, 0x4C];
const TACH_EXT: [u8; 6] = [0x18, 0x19, 0x1A, 0x81, 0x83, 0x4D];
const TACH_ALT: [u8; 6] = [0x0D, 0x0E, 0x0F, 0x80, 0x82, 0x93];
const TACH_EXT_ALT: [u8; 6] = [0x18, 0x19, 0x1A, 0x81, 0x83, 0x94];

/// The board firmware's own fan controller on the second chip of some Gigabyte boards. It
/// has to be paused while FoxRGB sets duties, or it writes over them.
pub(crate) struct Controller {
    initial: Option<bool>,
    current: Option<bool>,
}

const ECIO_COMMAND: u16 = 0x3F4;
const ECIO_DATA: u16 = 0x3F0;
const FAN_AREA: u16 = 0x900;
const ENABLE: u16 = 0x47;

/// The second chip's EC memory through its I/O mailbox.
fn wait(lpc: &Lpc, ready: impl Fn(u8) -> bool) -> bool {
    for _ in 0..1000 {
        if ready(lpc.inb(ECIO_COMMAND)) {
            return true;
        }
        lpc.sleep(1);
    }
    false
}

fn ecio_start(lpc: &Lpc, command: u8, offset: u16) -> bool {
    let empty = |s: u8| s & 2 == 0;
    for (port, value) in [
        (ECIO_COMMAND, command),
        (ECIO_DATA, (offset >> 8) as u8),
        (ECIO_DATA, offset as u8),
    ] {
        if !wait(lpc, empty) {
            return false;
        }
        lpc.outb(port, value);
    }
    true
}

pub(crate) fn ecio_read(lpc: &Lpc, offset: u16) -> Option<u8> {
    (ecio_start(lpc, 0xB0, offset) && wait(lpc, |s| s & 1 != 0)).then(|| lpc.inb(ECIO_DATA))
}

pub(crate) fn ecio_write(lpc: &Lpc, offset: u16, value: u8) -> bool {
    if !ecio_start(lpc, 0xB1, offset) || !wait(lpc, |s| s & 2 == 0) {
        return false;
    }
    lpc.outb(ECIO_DATA, value);
    true
}

impl Controller {
    /// The controller answers with version 1 when it is there.
    pub fn find(lpc: &Lpc) -> Option<Controller> {
        (ecio_read(lpc, FAN_AREA)? == 1).then_some(Controller {
            initial: None,
            current: None,
        })
    }

    fn enable(&mut self, lpc: &Lpc, on: bool) -> bool {
        if self.current.is_none() {
            let Some(now) = ecio_read(lpc, FAN_AREA + ENABLE) else {
                return false;
            };
            self.current = Some(now != 0);
            self.initial.get_or_insert(now != 0);
        }
        if self.current != Some(on) {
            if !ecio_write(lpc, FAN_AREA + ENABLE, on as u8) {
                return false;
            }
            lpc.sleep(500);
            self.current = Some(on);
        }
        true
    }

    fn restore(&mut self, lpc: &Lpc) {
        if let Some(initial) = self.initial {
            self.enable(lpc, initial);
        }
    }
}

/// How the second chip's own fan controller stands.
pub(crate) enum Firmware {
    /// Nothing to pause.
    Absent,
    Pausable(Controller),
    /// There, but out of reach: duties would be overwritten, so the headers are read only.
    Unreachable,
}

#[derive(Clone, Copy)]
struct Saved {
    control: u8,
    main: bool,
    ext: u8,
}

pub(crate) struct It87 {
    lpc: Lpc,
    chip: Chip,
    base: u16,
    temps: usize,
    fans: usize,
    pwm: Vec<u8>,
    ext: bool,
    alt_tach: bool,
    wide: bool,
    bank_zero: bool,
    disabled: Vec<bool>,
    saved: Vec<Option<Saved>>,
    firmware: Firmware,
}

impl It87 {
    /// `None` unless the vendor id and configuration register answer as an ITE chip.
    pub fn new(lpc: Lpc, chip: Chip, base: u16, version: u8, firmware: Firmware) -> Option<It87> {
        let pwm: &[u8] = match chip {
            IT8665E | IT8625E => &[0x15, 0x16, 0x17, 0x1E, 0x1F, 0x92],
            IT8792E => &[0x15, 0x16, 0x17],
            _ => &[0x15, 0x16, 0x17, 0x7F, 0xA7, 0xAF],
        };
        let (temps, fans, controls) = match chip {
            IT8613E => (4, 5, 4),
            IT8625E => (3, 6, 6),
            IT8628E | IT8665E | IT8689E | IT8696E => (6, 6, 6),
            IT8631E | IT8638E => (2, 2, 2),
            IT8686E => (7, 6, 5),
            IT8688E => (6, 6, 5),
            IT87952E | IT8792E | IT8705F => (3, 3, 3),
            IT8655E => (6, 3, 3),
            IT8620E => (3, 5, 5),
            _ => (3, 5, 3),
        };
        let ext = matches!(
            chip,
            IT8721F
                | IT8728F
                | IT8665E
                | IT8686E
                | IT8688E
                | IT8689E
                | IT87952E
                | IT8628E
                | IT8625E
                | IT8620E
                | IT8613E
                | IT8792E
                | IT8655E
                | IT8631E
                | IT8638E
                | IT8696E
                | IT8772E
        );
        let mut it = It87 {
            lpc,
            chip,
            base,
            temps,
            fans,
            pwm: pwm[..controls.min(pwm.len())].to_vec(),
            ext,
            alt_tach: matches!(chip, IT8665E | IT8625E),
            wide: (chip != IT8705F || version >= 3) && (chip != IT8712F || version >= 8),
            bank_zero: matches!(chip, IT8665E | IT8655E),
            disabled: vec![false; fans],
            saved: vec![None; controls],
            firmware,
        };
        let vendor = it.read(VENDOR)?;
        if !VENDORS.contains(&vendor) {
            return None;
        }
        let config = it.read(CONFIG)?;
        if config & 0x10 == 0 && !matches!(chip, IT8655E | IT8665E) {
            return None;
        }
        if it.wide {
            let modes = it.read(TACH_16BIT)?;
            if fans >= 5 {
                it.disabled[3] = modes & (1 << 4) == 0;
                it.disabled[4] = modes & (1 << 5) == 0;
            }
            if fans >= 6 {
                if chip == IT8665E {
                    if let Some(alt) = it.read(TACH_16BIT_ALT) {
                        it.disabled[5] = alt & (1 << 3) == 0;
                    }
                } else {
                    it.disabled[5] = modes & (1 << 2) == 0;
                }
            }
        }
        Some(it)
    }

    /// `None` when the index register does not read back, which means the value is not real.
    fn read(&self, reg: u8) -> Option<u8> {
        self.lpc.outb(self.base + INDEX, reg);
        let value = self.lpc.inb(self.base + DATA);
        let echo = self.lpc.inb(self.base + INDEX);
        (echo == reg || self.chip == IT8688E).then_some(value)
    }

    fn write(&self, reg: u8, value: u8) {
        self.lpc.outb(self.base + INDEX, reg);
        self.lpc.outb(self.base + DATA, value);
        self.lpc.inb(self.base + INDEX);
    }
}

impl Monitor for It87 {
    fn size(&self) -> (usize, usize, usize) {
        (self.temps, self.fans, self.pwm.len())
    }

    fn update(&mut self, values: &mut Values) {
        if self.bank_zero
            && let Some(bank) = self.read(BANK)
        {
            self.write(BANK, bank & 0x9F);
        }
        for (i, out) in values.temps.iter_mut().enumerate() {
            if let Some(raw) = self.read(TEMP_BASE + i as u8) {
                let t = raw as i8;
                *out = (t > 0 && t < i8::MAX).then_some(t as f32);
            }
        }
        for (i, out) in values.fans.iter_mut().enumerate() {
            if self.wide {
                if self.disabled[i] {
                    continue;
                }
                let (low, high) = if self.alt_tach {
                    (TACH_ALT[i], TACH_EXT_ALT[i])
                } else {
                    (TACH[i], TACH_EXT[i])
                };
                let Some(low) = self.read(low) else { continue };
                let Some(high) = self.read(high) else {
                    continue;
                };
                let count = (high as u32) << 8 | low as u32;
                *out = (count > 0x3F).then(|| {
                    if count < 0xFFFF {
                        1.35e6 / (count * 2) as f32
                    } else {
                        0.0
                    }
                });
            } else {
                let Some(count) = self.read(TACH[i]) else {
                    continue;
                };
                let mut divisor = 2;
                if i < 2 {
                    let Some(bits) = self.read(TACH_DIVISOR) else {
                        continue;
                    };
                    divisor = 1 << ((bits >> (3 * i)) & 7);
                }
                *out = (count > 0).then(|| {
                    if count < 0xFF {
                        1.35e6 / (count as u32 * divisor) as f32
                    } else {
                        0.0
                    }
                });
            }
        }
    }

    fn set_control(&mut self, index: usize, value: Option<u8>) -> Result<(), String> {
        match value {
            Some(value) => {
                if matches!(self.firmware, Firmware::Unreachable) {
                    return Err("the board's fan controller cannot be paused".into());
                }
                let saved = match self.saved[index] {
                    Some(saved) => saved,
                    None => {
                        let saved = Saved {
                            control: self.read(self.pwm[index]).unwrap_or(0),
                            main: index < 3
                                && self.read(MAIN_CONTROL).unwrap_or(0) & (1 << index) != 0,
                            ext: if self.ext {
                                self.read(PWM_EXT[index]).unwrap_or(0)
                            } else {
                                0
                            },
                        };
                        self.saved[index] = Some(saved);
                        saved
                    }
                };
                if let Firmware::Pausable(controller) = &mut self.firmware
                    && !controller.enable(&self.lpc, false)
                {
                    return Err("the board's fan controller did not pause".into());
                }
                if index < 3 && !saved.main {
                    let main = self.read(MAIN_CONTROL).unwrap_or(0);
                    self.write(MAIN_CONTROL, main | (1 << index));
                }
                if self.ext {
                    let control = if self.chip == IT8689E {
                        0x7F
                    } else {
                        saved.control & 0x7F
                    };
                    self.write(self.pwm[index], control);
                    self.write(PWM_EXT[index], value);
                } else {
                    self.write(self.pwm[index], value >> 1);
                }
            }
            None => {
                let Some(saved) = self.saved[index].take() else {
                    return Ok(());
                };
                self.write(self.pwm[index], saved.control);
                if index < 3 {
                    let main = self.read(MAIN_CONTROL).unwrap_or(0);
                    if (main & (1 << index) != 0) != saved.main {
                        self.write(MAIN_CONTROL, main ^ (1 << index));
                    }
                }
                if self.ext {
                    self.write(PWM_EXT[index], saved.ext);
                }
                if self.saved.iter().all(Option::is_none)
                    && let Firmware::Pausable(controller) = &mut self.firmware
                {
                    controller.restore(&self.lpc);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testbus::{Bus, Device, Flat, Regs, regs};

    const BASE: u16 = 0xA40;

    fn chip(kind: Chip, values: &[(u16, u8)], firmware: Firmware) -> (Bus, Regs, Option<It87>) {
        let bus = Bus::default();
        let regs = regs(values);
        bus.device(BASE, BASE + 7, Flat::new(BASE, &regs));
        let it = It87::new(bus.lpc(0x2E), kind, BASE, 0, firmware);
        (bus, regs, it)
    }

    fn read(reg: u8) -> Vec<String> {
        vec![
            format!("pio_outb a45 {reg:02x}"),
            "pio_inb a46".into(),
            "pio_inb a45".into(),
        ]
    }

    fn write(reg: u8, value: u8) -> Vec<String> {
        vec![
            format!("pio_outb a45 {reg:02x}"),
            format!("pio_outb a46 {value:02x}"),
            "pio_inb a45".into(),
        ]
    }

    const ITE: [(u16, u8); 3] = [(0x58, 0x90), (0x00, 0x10), (0x0C, 0x30)];

    #[test]
    fn identified_by_vendor_and_configuration() {
        let (bus, _, it) = chip(IT8688E, &ITE, Firmware::Absent);
        let it = it.unwrap();
        assert_eq!(bus.take(), [read(0x58), read(0x00), read(0x0C)].concat());
        assert_eq!(it.disabled, [false, false, false, false, false, true]);
        let (_, _, it) = chip(IT8688E, &[(0x58, 0x12), (0x00, 0x10)], Firmware::Absent);
        assert!(it.is_none());
        let (_, _, it) = chip(IT8688E, &[(0x58, 0x90), (0x00, 0x00)], Firmware::Absent);
        assert!(it.is_none());
        let (_, _, it) = chip(IT8665E, &[(0x58, 0x90), (0x00, 0x00)], Firmware::Absent);
        assert!(it.is_some());
    }

    #[test]
    fn index_that_does_not_read_back_is_not_trusted() {
        let bus = Bus::default();
        let regs = regs(&ITE);
        let mut flat = Flat::new(BASE, &regs);
        flat.echo = false;
        bus.device(BASE, BASE + 7, flat);
        assert!(It87::new(bus.lpc(0x2E), IT8686E, BASE, 0, Firmware::Absent).is_none());
        // the IT8688E never echoes its index
        let bus = Bus::default();
        let mut flat = Flat::new(BASE, &regs);
        flat.echo = false;
        bus.device(BASE, BASE + 7, flat);
        assert!(It87::new(bus.lpc(0x2E), IT8688E, BASE, 0, Firmware::Absent).is_some());
    }

    #[test]
    fn temperatures_and_speeds() {
        let mut values = ITE.to_vec();
        values.extend([(0x29, 38), (0x2A, 0x80), (0x2B, 0)]);
        // 0x0546 counts: 1.35e6 / (1350 * 2) = 500 rpm
        values.extend([(0x0D, 0x46), (0x18, 0x05), (0x0E, 0xFF), (0x19, 0xFF)]);
        values.extend([(0x0F, 0x10), (0x1A, 0x00)]);
        let (bus, _, it) = chip(IT8688E, &values, Firmware::Absent);
        let mut it = it.unwrap();
        bus.take();
        let mut v = Values {
            temps: vec![None; 6],
            fans: vec![None; 6],
        };
        it.update(&mut v);
        assert_eq!(v.temps[..3], [Some(38.0), None, None]);
        assert_eq!(v.fans[..3], [Some(500.0), Some(0.0), None]);
        let log = bus.take();
        assert_eq!(log[18..24], [read(0x0D), read(0x18)].concat());
        // the disabled sixth fan is never read
        assert_eq!(log.len(), (6 + 5 * 2) * 3);
    }

    #[test]
    fn eight_bit_counters_use_the_divisor() {
        let values = [(0x58, 0x90), (0x00, 0x10), (0x0D, 0x96), (0x0B, 0x03)];
        let (_, _, it) = chip(IT8705F, &values, Firmware::Absent);
        let mut it = it.unwrap();
        let mut v = Values {
            temps: vec![None; 3],
            fans: vec![None; 3],
        };
        it.update(&mut v);
        assert_eq!(v.fans[0], Some(1.35e6 / (0x96 as f32 * 8.0)));
    }

    #[test]
    fn manual_duty_then_back_to_the_chip() {
        let mut values = ITE.to_vec();
        values.extend([(0x13, 0x70), (0x16, 0x85), (0x6B, 0x20)]);
        let (bus, regs, it) = chip(IT8688E, &values, Firmware::Absent);
        let mut it = it.unwrap();
        bus.take();
        it.set_control(1, Some(0xB0)).unwrap();
        let want = [
            read(0x16),
            read(0x13),
            read(0x6B),
            read(0x13),
            write(0x13, 0x72),
            write(0x16, 0x05),
            write(0x6B, 0xB0),
        ]
        .concat();
        assert_eq!(bus.take(), want);
        it.set_control(1, None).unwrap();
        let want = [
            write(0x16, 0x85),
            read(0x13),
            write(0x13, 0x70),
            write(0x6B, 0x20),
        ]
        .concat();
        assert_eq!(bus.take(), want);
        assert_eq!(regs.lock().unwrap()[&0x13], 0x70);
    }

    #[test]
    fn older_chips_take_seven_bit_duty() {
        let values = [
            (0x58, 0x90),
            (0x00, 0x10),
            (0x0C, 0),
            (0x13, 0x07),
            (0x15, 0x80),
        ];
        let (bus, _, it) = chip(IT8718F, &values, Firmware::Absent);
        let mut it = it.unwrap();
        bus.take();
        it.set_control(0, Some(0xFF)).unwrap();
        assert_eq!(
            bus.take(),
            [read(0x15), read(0x13), write(0x15, 0x7F)].concat()
        );
    }

    /// The mailbox at 0x3F0/0x3F4 in front of EC memory.
    struct Mailbox {
        mem: Regs,
        step: Vec<u8>,
        out: Option<u8>,
    }

    impl Device for Mailbox {
        fn inb(&mut self, port: u16) -> u8 {
            match port {
                ECIO_COMMAND => u8::from(self.out.is_some()),
                _ => self.out.take().unwrap_or(0),
            }
        }
        fn outb(&mut self, port: u16, value: u8) {
            if port == ECIO_COMMAND {
                self.step = vec![value];
                return;
            }
            self.step.push(value);
            let at = (self.step.get(1).copied().unwrap_or(0) as u16) << 8
                | self.step.get(2).copied().unwrap_or(0) as u16;
            match (self.step[0], self.step.len()) {
                (0xB0, 3) => self.out = self.mem.lock().unwrap().get(&at).copied(),
                (0xB1, 4) => {
                    self.mem.lock().unwrap().insert(at, value);
                }
                _ => {}
            }
        }
    }

    #[test]
    fn gigabyte_controller_is_paused_and_restored() {
        let mem = regs(&[(0x900, 1), (0x947, 1)]);
        let bus = Bus::default();
        let values = regs(&[(0x58, 0x90), (0x00, 0x10), (0x0C, 0), (0x13, 0x07)]);
        bus.device(BASE, BASE + 7, Flat::new(BASE, &values));
        bus.device(
            0x3F0,
            0x3F4,
            Mailbox {
                mem: mem.clone(),
                step: Vec::new(),
                out: None,
            },
        );
        let lpc = bus.lpc(0x4E);
        let controller = Controller::find(&lpc).unwrap();
        let want = crate::testbus::log(&[
            "pio_inb 3f4",
            "pio_outb 3f4 b0",
            "pio_inb 3f4",
            "pio_outb 3f0 09",
            "pio_inb 3f4",
            "pio_outb 3f0 00",
            "pio_inb 3f4",
            "pio_inb 3f0",
        ]);
        assert_eq!(bus.take(), want);
        let mut it = It87::new(lpc, IT8792E, BASE, 0, Firmware::Pausable(controller)).unwrap();
        it.set_control(2, Some(0x40)).unwrap();
        assert_eq!(mem.lock().unwrap()[&0x947], 0);
        it.set_control(2, None).unwrap();
        assert_eq!(mem.lock().unwrap()[&0x947], 1);
        let (_, _, it) = chip(
            IT8792E,
            &[(0x58, 0x90), (0x00, 0x10)],
            Firmware::Unreachable,
        );
        assert!(it.unwrap().set_control(0, Some(1)).is_err());
    }
}
