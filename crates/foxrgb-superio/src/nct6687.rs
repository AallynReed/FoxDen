//! Nuvoton NCT6683D, NCT6686D and NCT6687D, whose embedded controller is reached through pages.

use foxrgb_pawnio::SharedLock;

use crate::chip::Chip as Kind;
use crate::monitor::{Monitor, Values};
use crate::port::Lpc;

pub(crate) const ISA_MUTEX: &str = r"Global\Access_ISABUS.HTP.Method";
const EC_PAGE: u16 = 0x04;
const EC_INDEX: u16 = 0x05;
const EC_DATA: u16 = 0x06;
const EC_FREE: u8 = 0xFF;

pub struct Chip {
    lpc: Lpc,
    lock: Option<SharedLock>,
    base: u16,
}

impl Chip {
    pub(crate) fn new(lpc: Lpc, base: u16, lock: Option<SharedLock>) -> Chip {
        Chip { lpc, lock, base }
    }

    pub fn base(&self) -> u16 {
        self.base
    }

    fn locked<T>(&self, f: impl FnOnce() -> T) -> T {
        match &self.lock {
            Some(lock) => lock.run(f),
            None => f(),
        }
    }

    /// One byte of the chip's EC space, as `page << 8 | index`.
    pub fn read(&self, address: u16) -> u8 {
        self.locked(|| self.raw_read(address))
    }

    pub fn write(&self, address: u16, value: u8) {
        self.locked(|| self.raw_write(address, value))
    }

    fn raw_read(&self, address: u16) -> u8 {
        self.claim();
        self.lpc.outb(self.base + EC_PAGE, (address >> 8) as u8);
        self.lpc.outb(self.base + EC_INDEX, address as u8);
        let value = self.lpc.inb(self.base + EC_DATA);
        self.lpc.outb(self.base + EC_PAGE, EC_FREE);
        value
    }

    fn raw_write(&self, address: u16, value: u8) {
        self.claim();
        self.lpc.outb(self.base + EC_PAGE, (address >> 8) as u8);
        self.lpc.outb(self.base + EC_INDEX, address as u8);
        self.lpc.outb(self.base + EC_DATA, value);
        self.lpc.outb(self.base + EC_PAGE, EC_FREE);
    }

    /// Waits up to half a second for the EC page register to be free, then takes it.
    fn claim(&self) {
        if !self
            .lpc
            .until(500, || self.lpc.inb(self.base + EC_PAGE) == EC_FREE)
        {
            self.lpc.outb(self.base + EC_PAGE, EC_FREE);
        }
    }

    /// Temperature at a register pair, in degrees Celsius.
    pub fn temperature(&self, register: u16) -> Option<f32> {
        let whole = self.read(register) as i8 as f32;
        let half = (self.read(register + 1) >> 7) as f32 * 0.5;
        let t = whole + half;
        (t > -55.0 && t < 125.0 && t != 0.0).then_some(t)
    }

    /// Fan speed in rpm at a register pair, 0 when stopped or absent.
    pub fn rpm(&self, register: u16) -> u32 {
        let high = self.read(register) as u32;
        let low = self.read(register + 1) as u32;
        if high == 0xFF && low == 0xF8 {
            return 0;
        }
        // The NCT6687D reports rpm directly as a 16-bit value.
        (high << 8) | low
    }
}

const MODE: u16 = 0xA00;
const REQUEST: u16 = 0xA01;
const COMMAND: u16 = 0xA28;
const CFG_REQUEST: u8 = 0x80;
const CFG_COMPLETE: u8 = 0x40;

impl Chip {
    /// The duty the board itself is using on a header, 0 to 255.
    pub fn duty(&self, header: u8) -> u8 {
        self.read(0x160 + header as u16)
    }

    /// Whether FoxRGB has a header in manual mode.
    pub fn is_manual(&self, header: u8) -> bool {
        self.read(MODE) & (1 << header) != 0
    }

    /// Takes a header into manual mode at `duty` (0 to 255).
    pub fn set(&self, header: u8, duty: u8) {
        self.write(REQUEST, CFG_REQUEST);
        self.lpc.sleep(50);
        let mode = self.read(MODE) | (1 << header);
        self.write(MODE, mode);
        self.write(COMMAND + header as u16, duty);
        self.write(REQUEST, CFG_COMPLETE);
        self.lpc.sleep(50);
    }

    /// Hands a header back to the board's own fan curve, restoring the command it had.
    pub fn release(&self, header: u8, command: u8) {
        self.write(REQUEST, CFG_REQUEST);
        self.lpc.sleep(50);
        let mode = self.read(MODE) & !(1 << header);
        self.write(MODE, mode);
        self.write(COMMAND + header as u16, command);
        self.write(REQUEST, CFG_COMPLETE);
        self.lpc.sleep(50);
    }

    pub fn command(&self, header: u8) -> u8 {
        self.read(COMMAND + header as u16)
    }
}

const ENGINE: u16 = 0xCF8;
const LOCKED: u8 = 1 << 6;
const PHASE: u8 = 1 << 3;
const INVALID: u8 = 1 << 4;
const CHECKED: u8 = 1 << 5;
/// Milliseconds the fan engine gets for each step.
const PATIENCE: u64 = 1000;
const NONE: u16 = 0xFFF;

/// The NCT6683D, NCT6686D, and the NCT6687D on MSI 800-series boards, as a generic monitor.
pub(crate) struct Paged {
    chip: Chip,
    kind: Kind,
    temps: Vec<u16>,
    speeds: Vec<u16>,
    command: Vec<u16>,
    mode: Vec<u16>,
    bits: Vec<i8>,
    saved: Vec<Option<(u8, u8)>>,
}

impl Paged {
    pub fn new(lpc: Lpc, kind: Kind, base: u16) -> Paged {
        let chip = Chip::new(lpc, base, None);
        if kind == Kind::NCT6687DR {
            return Paged {
                chip,
                kind,
                temps: (0..7).map(|i| 0x100 + 2 * i).collect(),
                speeds: vec![
                    0x140, 0x142, 0x144, 0x146, 0x148, 0x14A, 0x14C, 0x14E, 0x150, 0x152, 0x15E,
                    0x15C, 0x15A, 0x158, 0x156, 0x154,
                ],
                command: vec![
                    0xA28, 0xA29, 0xA2A, 0xA2B, NONE, NONE, NONE, NONE, NONE, 0x8E9, 0x265, 0x264,
                    0x263, 0x262, 0x261, 0x260,
                ],
                mode: [vec![0xA00; 9], vec![0x80F; 7]].concat(),
                bits: vec![0, 1, 2, 3, -1, -1, -1, -1, -1, 1, 7, 6, 5, 4, 3, 2],
                saved: vec![None; 16],
            };
        }
        let mut speeds: Vec<u16> = (0..16).map(|i| 0x140 + 2 * i).collect();
        speeds.push(0x852);
        Paged {
            chip,
            kind,
            temps: (0..11).map(|i| 0x100 + 2 * i).collect(),
            speeds,
            command: (0..8).map(|i| COMMAND + i).collect(),
            mode: vec![MODE; 8],
            bits: (0..8).collect(),
            saved: vec![None; 8],
        }
    }

    fn update_byte(&self, at: u16, keep: u8, set: u8) {
        let value = self.chip.raw_read(at);
        self.chip.raw_write(at, (value & keep) | set);
    }

    fn engine(&self) -> u8 {
        self.chip.raw_read(ENGINE)
    }

    fn open(&self) -> bool {
        let ready = |s: u8| s & LOCKED == 0 && s & PHASE != 0;
        if ready(self.engine()) {
            return true;
        }
        self.chip.lpc.until(PATIENCE, || {
            self.engine() & PHASE == 0 && self.chip.raw_read(REQUEST) & CFG_REQUEST == 0
        });
        self.update_byte(REQUEST, 0x7F, CFG_REQUEST);
        self.chip.lpc.sleep(10);
        self.chip.lpc.until(PATIENCE, || ready(self.engine()))
    }

    fn commit(&self) -> bool {
        let status = self.engine();
        if status & LOCKED != 0 || status & PHASE == 0 {
            return false;
        }
        self.update_byte(REQUEST, 0xBF, CFG_COMPLETE);
        self.chip.lpc.sleep(10);
        self.chip
            .lpc
            .until(PATIENCE, || self.engine() & CHECKED != 0);
        self.engine() & INVALID == 0
    }

    /// Sets a header's manual bit and command inside one configuration phase, up to three tries.
    fn phase(&self, index: usize, bit: u8, value: u8) {
        let mask = 1 << self.bits[index];
        for _ in 0..3 {
            if !self.open() {
                break;
            }
            self.update_byte(self.mode[index], !mask, bit);
            self.chip.raw_write(self.command[index], value);
            if self.commit() {
                break;
            }
        }
    }
}

impl Monitor for Paged {
    fn size(&self) -> (usize, usize, usize) {
        (self.temps.len(), self.speeds.len(), self.command.len())
    }

    fn update(&mut self, values: &mut Values) {
        for (out, &reg) in values.temps.iter_mut().zip(&self.temps) {
            let whole = self.chip.raw_read(reg) as i8 as f32;
            let half = ((self.chip.raw_read(reg + 1) >> 7) & 1) as f32 * 0.5;
            *out = Some(whole + half);
        }
        for (out, &reg) in values.fans.iter_mut().zip(&self.speeds) {
            let high = self.chip.raw_read(reg);
            let low = self.chip.raw_read(reg + 1);
            *out = if reg == 0x852 {
                crate::nct677x::count13(high, low)
            } else {
                Some(((high as u16) << 8 | low as u16) as f32)
            };
        }
    }

    fn set_control(&mut self, index: usize, value: Option<u8>) -> Result<(), String> {
        if self.command[index] == NONE || self.bits[index] < 0 {
            return Err("this header has no control".into());
        }
        let mask = 1u8 << self.bits[index];
        let dr = self.kind == Kind::NCT6687DR;
        match value {
            Some(value) => {
                if self.saved[index].is_none() {
                    let mode = self.chip.raw_read(self.mode[index]) & mask;
                    self.saved[index] = Some((mode, self.chip.raw_read(self.command[index])));
                }
                if dr {
                    self.phase(index, mask, value);
                } else {
                    self.chip.raw_write(REQUEST, CFG_REQUEST);
                    self.chip.lpc.sleep(50);
                    let mode = self.chip.raw_read(MODE) | mask;
                    self.chip.raw_write(MODE, mode);
                    self.chip.raw_write(self.command[index], value);
                    self.chip.raw_write(REQUEST, CFG_COMPLETE);
                    self.chip.lpc.sleep(50);
                }
            }
            None => {
                let Some((mode, command)) = self.saved[index].take() else {
                    return Ok(());
                };
                if dr {
                    self.phase(index, mode, command);
                } else {
                    let now = self.chip.raw_read(MODE);
                    self.chip.raw_write(MODE, (now & !mask) | mode);
                    self.chip.raw_write(REQUEST, CFG_REQUEST);
                    self.chip.lpc.sleep(50);
                    self.chip.raw_write(self.command[index], command);
                    self.chip.raw_write(REQUEST, CFG_COMPLETE);
                    self.chip.lpc.sleep(50);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testbus::{Bus, Device, Regs, regs};

    const BASE: u16 = 0xA20;

    /// The paged window, plus an engine that opens a phase on request and accepts on completion.
    struct Ec {
        regs: Regs,
        page: u8,
        index: u8,
        engine: bool,
    }

    impl Device for Ec {
        fn inb(&mut self, port: u16) -> u8 {
            match port - BASE {
                4 => self.page,
                6 => {
                    let at = (self.page as u16) << 8 | self.index as u16;
                    self.regs.lock().unwrap().get(&at).copied().unwrap_or(0)
                }
                _ => 0xFF,
            }
        }
        fn outb(&mut self, port: u16, value: u8) {
            match port - BASE {
                4 => self.page = value,
                5 => self.index = value,
                6 => {
                    let at = (self.page as u16) << 8 | self.index as u16;
                    let mut regs = self.regs.lock().unwrap();
                    regs.insert(at, value);
                    if self.engine && at == REQUEST {
                        let status = if value & CFG_COMPLETE != 0 {
                            CHECKED
                        } else {
                            PHASE
                        };
                        regs.insert(ENGINE, status);
                    }
                }
                _ => {}
            }
        }
    }

    fn paged(kind: Kind, values: &[(u16, u8)]) -> (Bus, Regs, Paged) {
        let bus = Bus::default();
        let regs = regs(values);
        bus.device(
            BASE,
            BASE + 7,
            Ec {
                regs: regs.clone(),
                page: EC_FREE,
                index: 0,
                engine: kind == Kind::NCT6687DR,
            },
        );
        let paged = Paged::new(bus.lpc(0x4E), kind, BASE);
        (bus, regs, paged)
    }

    fn read(at: u16) -> Vec<String> {
        vec![
            "pio_inb a24".into(),
            format!("pio_outb a24 {:02x}", at >> 8),
            format!("pio_outb a25 {:02x}", at & 0xFF),
            "pio_inb a26".into(),
            "pio_outb a24 ff".into(),
        ]
    }

    fn write(at: u16, value: u8) -> Vec<String> {
        vec![
            "pio_inb a24".into(),
            format!("pio_outb a24 {:02x}", at >> 8),
            format!("pio_outb a25 {:02x}", at & 0xFF),
            format!("pio_outb a26 {value:02x}"),
            "pio_outb a24 ff".into(),
        ]
    }

    #[test]
    fn reads_temperatures_and_speeds() {
        let values = [(0x100, 44), (0x101, 0x80), (0x142, 0x05), (0x143, 0xDC)];
        let (bus, _, mut chip) = paged(Kind::NCT6686D, &values);
        let mut v = Values {
            temps: vec![None; 11],
            fans: vec![None; 17],
        };
        chip.update(&mut v);
        assert_eq!(v.temps[0], Some(44.5));
        assert_eq!(v.fans[1], Some(1500.0));
        assert_eq!(v.fans[16], None);
        assert_eq!(bus.take()[..10], [read(0x100), read(0x101)].concat());
    }

    #[test]
    fn nct6686d_manual_duty_then_back() {
        let (bus, _, mut chip) = paged(Kind::NCT6686D, &[(MODE, 0x01), (0xA2A, 0x77)]);
        chip.set_control(2, Some(0x40)).unwrap();
        let want = [
            read(MODE),
            read(0xA2A),
            write(REQUEST, 0x80),
            read(MODE),
            write(MODE, 0x05),
            write(0xA2A, 0x40),
            write(REQUEST, 0x40),
        ]
        .concat();
        assert_eq!(bus.take(), want);
        chip.set_control(2, None).unwrap();
        let want = [
            read(MODE),
            write(MODE, 0x01),
            write(REQUEST, 0x80),
            write(0xA2A, 0x77),
            write(REQUEST, 0x40),
        ]
        .concat();
        assert_eq!(bus.take(), want);
    }

    #[test]
    fn nct6687dr_writes_inside_a_configuration_phase() {
        let (bus, regs, mut chip) = paged(Kind::NCT6687DR, &[(0x80F, 0x81), (0x265, 0x30)]);
        assert!(chip.set_control(5, Some(1)).is_err());
        chip.set_control(10, Some(0xC0)).unwrap();
        let want = [
            read(0x80F),
            read(0x265),
            read(ENGINE),
            read(ENGINE),
            read(REQUEST),
            read(REQUEST),
            write(REQUEST, 0x80),
            read(ENGINE),
            read(0x80F),
            write(0x80F, 0x81),
            write(0x265, 0xC0),
            read(ENGINE),
            read(REQUEST),
            write(REQUEST, 0xC0),
            read(ENGINE),
            read(ENGINE),
        ]
        .concat();
        assert_eq!(bus.take(), want);
        regs.lock().unwrap().extend([(ENGINE, 0), (REQUEST, 0)]);
        chip.set_control(10, None).unwrap();
        assert_eq!(bus.take().len(), 14 * 5);
        let r = regs.lock().unwrap();
        assert_eq!((r[&0x80F], r[&0x265]), (0x81, 0x30));
    }
}
