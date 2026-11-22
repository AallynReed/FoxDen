// SPDX-License-Identifier: GPL-2.0-or-later

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use crate::i2c::{BusInfo, Result, SharedBus, SmBus};

/// One bus call or sleep, in the order the driver made it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Quick(u8),
    Rb(u8),
    Wb(u8, u8),
    Rbd(u8, u8),
    Wbd(u8, u8, u8),
    Rw(u8, u8),
    Ww(u8, u8, u16),
    Blk(u8, u8, Vec<u8>),
    Sleep(u64),
}

thread_local! {
    static LOG: RefCell<Vec<Op>> = const { RefCell::new(Vec::new()) };
}

pub fn log(op: Op) {
    LOG.with(|l| l.borrow_mut().push(op));
}

/// Everything logged on this test's thread since the last call.
pub fn take() -> Vec<Op> {
    LOG.with(|l| std::mem::take(&mut *l.borrow_mut()))
}

type Hook = Box<dyn FnMut(&Op) -> Option<Result<u16>> + Send>;

/// Simulated devices: SPD EEPROMs with paging, plain register tables and an optional hook.
#[derive(Default)]
pub struct Sim {
    pub ddr4: HashMap<u8, Vec<u8>>,
    pub ddr5: HashMap<u8, Vec<u8>>,
    pub ddr4_page: usize,
    pub ddr5_page: HashMap<u8, u8>,
    pub bytes: HashMap<(u8, u8), u8>,
    pub words: HashMap<(u8, u8), u16>,
    pub present: HashSet<u8>,
    pub fail_block: bool,
    pub hook: Option<Hook>,
}

impl Sim {
    fn here(&self, a: u8) -> Result<u16> {
        if self.present.contains(&a) || self.ddr4.contains_key(&a) || self.ddr5.contains_key(&a) {
            Ok(0)
        } else {
            Err(format!("nack {a:02x}"))
        }
    }

    fn answer(&mut self, op: &Op) -> Result<u16> {
        if let Some(r) = self.hook.as_mut().and_then(|hook| hook(op)) {
            return r;
        }
        match *op {
            Op::Quick(a) | Op::Rb(a) => self.here(a),
            Op::Wb(a, _) if a == 0x36 || a == 0x37 => self.ddr4_page_select(a),
            Op::Wbd(a, 0, _) if a == 0x36 || a == 0x37 => self.ddr4_page_select(a),
            Op::Wbd(a, 0x0B, v) if self.ddr5.contains_key(&a) => {
                self.ddr5_page.insert(a, v);
                Ok(0)
            }
            Op::Rbd(a, c) => {
                if let Some(e) = self.ddr4.get(&a) {
                    return Ok(e[self.ddr4_page * 256 + c as usize] as u16);
                }
                if let Some(e) = self.ddr5.get(&a) {
                    let page = *self.ddr5_page.get(&a).unwrap_or(&0) as usize;
                    return Ok(match c {
                        0x00 => 0x51,
                        0x01 => 0x08,
                        0x0B => page as u16,
                        0x80..=0xFF => e[page * 128 + (c & 0x7F) as usize] as u16,
                        _ => 0,
                    });
                }
                match self.bytes.get(&(a, c)) {
                    Some(&v) => Ok(v as u16),
                    None => self.here(a),
                }
            }
            Op::Rw(a, c) => match self.words.get(&(a, c)) {
                Some(&v) => Ok(v),
                None => self.here(a),
            },
            Op::Blk(a, ..) if self.fail_block => Err(format!("block nack {a:02x}")),
            Op::Wb(a, _) | Op::Wbd(a, ..) | Op::Ww(a, ..) | Op::Blk(a, ..) => self.here(a),
            Op::Sleep(_) => Ok(0),
        }
    }

    fn ddr4_page_select(&mut self, a: u8) -> Result<u16> {
        if self.ddr4.is_empty() {
            return Err("no ee1004".into());
        }
        self.ddr4_page = (a - 0x36) as usize;
        Ok(0)
    }
}

pub struct Fake {
    info: BusInfo,
    sim: Mutex<Sim>,
}

impl Fake {
    fn call(&self, op: Op) -> Result<u16> {
        let r = self.sim.lock().unwrap().answer(&op);
        log(op);
        r
    }
}

/// A Raptor Lake SMBus, which passes the IF_DRAM_SMBUS check.
pub fn intel() -> BusInfo {
    BusInfo {
        name: "fake".into(),
        pci_vendor: 0x8086,
        pci_device: 0x7A23,
        ..Default::default()
    }
}

pub fn bus(sim: Sim) -> SharedBus {
    bus_on(intel(), sim)
}

pub fn bus_on(info: BusInfo, sim: Sim) -> SharedBus {
    Arc::new(Mutex::new(Box::new(Fake {
        info,
        sim: Mutex::new(sim),
    })))
}

/// A DDR4 SPD image with the JEDEC bytes at 0x140 and manufacturer data from 0x161.
pub fn ddr4_spd(jedec: [u8; 2], manuf: &[u8]) -> Vec<u8> {
    let mut e = vec![0u8; 512];
    e[0] = 0x23;
    e[2] = 12;
    e[0x140..0x142].copy_from_slice(&jedec);
    e[0x161..0x161 + manuf.len()].copy_from_slice(manuf);
    e
}

/// A DDR5 SPD image with the JEDEC bytes at 0x200 and manufacturer data from 0x22B.
pub fn ddr5_spd(jedec: [u8; 2], manuf: &[u8]) -> Vec<u8> {
    let mut e = vec![0u8; 1024];
    e[2] = 18;
    e[0x200..0x202].copy_from_slice(&jedec);
    e[0x22B..0x22B + manuf.len()].copy_from_slice(manuf);
    e
}

impl SmBus for Fake {
    fn info(&self) -> &BusInfo {
        &self.info
    }
    fn write_quick(&self, addr: u8) -> Result<()> {
        self.call(Op::Quick(addr)).map(|_| ())
    }
    fn read_byte(&self, addr: u8) -> Result<u8> {
        self.call(Op::Rb(addr)).map(|v| v as u8)
    }
    fn write_byte(&self, addr: u8, value: u8) -> Result<()> {
        self.call(Op::Wb(addr, value)).map(|_| ())
    }
    fn read_byte_data(&self, addr: u8, command: u8) -> Result<u8> {
        self.call(Op::Rbd(addr, command)).map(|v| v as u8)
    }
    fn write_byte_data(&self, addr: u8, command: u8, value: u8) -> Result<()> {
        self.call(Op::Wbd(addr, command, value)).map(|_| ())
    }
    fn read_word_data(&self, addr: u8, command: u8) -> Result<u16> {
        self.call(Op::Rw(addr, command))
    }
    fn write_word_data(&self, addr: u8, command: u8, value: u16) -> Result<()> {
        self.call(Op::Ww(addr, command, value)).map(|_| ())
    }
    fn read_block_data(&self, _addr: u8, _command: u8) -> Result<Vec<u8>> {
        Err("unused".into())
    }
    fn write_block_data(&self, addr: u8, command: u8, data: &[u8]) -> Result<()> {
        self.call(Op::Blk(addr, command, data.to_vec())).map(|_| ())
    }
    fn i2c_read_block_data(&self, _addr: u8, _command: u8, _len: u8) -> Result<Vec<u8>> {
        Err("unused".into())
    }
    fn i2c_write_block_data(&self, _addr: u8, _command: u8, _data: &[u8]) -> Result<()> {
        Err("unused".into())
    }
}
