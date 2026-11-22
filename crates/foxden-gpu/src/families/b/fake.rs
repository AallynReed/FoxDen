// SPDX-License-Identifier: GPL-2.0-or-later

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use foxden_core::{Backend, Color};
use foxden_smbus::i2c::{BusInfo, Detect, Result, SharedBus, SmBus};

use super::common::Card;

thread_local! {
    static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

pub fn note(entry: String) {
    LOG.with(|l| l.borrow_mut().push(entry));
}

/// Everything logged on this thread since the last call.
pub fn take() -> Vec<String> {
    LOG.with(|l| std::mem::take(&mut *l.borrow_mut()))
}

#[derive(Default)]
pub struct Fake {
    pub info: BusInfo,
    /// Byte reads per (addr, cmd); the last answer repeats.
    bytes: Mutex<HashMap<(u8, u8), VecDeque<u8>>>,
    blocks: HashMap<(u8, u8), Vec<u8>>,
    plain: HashMap<u8, u8>,
    /// Raw reads per address; the last answer repeats.
    raw: Mutex<HashMap<u8, VecDeque<Vec<u8>>>>,
    pub quick_ok: bool,
    pub fail_writes: bool,
}

impl Fake {
    pub fn new(name: &str, ids: [u16; 4]) -> Self {
        take();
        Fake {
            info: BusInfo {
                name: name.into(),
                pci_vendor: ids[0],
                pci_device: ids[1],
                pci_subsystem_vendor: ids[2],
                pci_subsystem_device: ids[3],
            },
            ..Default::default()
        }
    }

    pub fn byte(mut self, addr: u8, cmd: u8, answers: &[u8]) -> Self {
        self.bytes
            .get_mut()
            .unwrap()
            .insert((addr, cmd), answers.iter().copied().collect());
        self
    }

    pub fn block(mut self, addr: u8, cmd: u8, data: &[u8]) -> Self {
        self.blocks.insert((addr, cmd), data.to_vec());
        self
    }

    pub fn plain(mut self, addr: u8, value: u8) -> Self {
        self.plain.insert(addr, value);
        self
    }

    pub fn raw(mut self, addr: u8, answers: &[&[u8]]) -> Self {
        let q = answers.iter().map(|a| a.to_vec()).collect();
        self.raw.get_mut().unwrap().insert(addr, q);
        self
    }

    pub fn shared(self) -> SharedBus {
        Arc::new(Mutex::new(Box::new(self)))
    }

    fn written(&self) -> Result<()> {
        if self.fail_writes {
            Err("write failed".into())
        } else {
            Ok(())
        }
    }
}

fn hex(data: &[u8]) -> String {
    data.iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

impl SmBus for Fake {
    fn info(&self) -> &BusInfo {
        &self.info
    }
    fn write_quick(&self, addr: u8) -> Result<()> {
        note(format!("quick {addr:02x}"));
        if self.quick_ok {
            Ok(())
        } else {
            Err("nack".into())
        }
    }
    fn read_byte(&self, addr: u8) -> Result<u8> {
        note(format!("rb {addr:02x}"));
        self.plain.get(&addr).copied().ok_or("nack".into())
    }
    fn write_byte(&self, addr: u8, value: u8) -> Result<()> {
        note(format!("wb {addr:02x} {value:02x}"));
        self.written()
    }
    fn read_byte_data(&self, addr: u8, command: u8) -> Result<u8> {
        note(format!("rd {addr:02x} {command:02x}"));
        let mut bytes = self.bytes.lock().unwrap();
        let q = bytes.get_mut(&(addr, command)).ok_or("nack")?;
        if q.len() > 1 {
            Ok(q.pop_front().unwrap())
        } else {
            q.front().copied().ok_or("nack".into())
        }
    }
    fn write_byte_data(&self, addr: u8, command: u8, value: u8) -> Result<()> {
        note(format!("wr {addr:02x} {command:02x} {value:02x}"));
        self.written()
    }
    fn read_word_data(&self, addr: u8, command: u8) -> Result<u16> {
        note(format!("rw {addr:02x} {command:02x}"));
        Err("nack".into())
    }
    fn write_word_data(&self, addr: u8, command: u8, value: u16) -> Result<()> {
        note(format!("ww {addr:02x} {command:02x} {value:04x}"));
        self.written()
    }
    fn read_block_data(&self, addr: u8, command: u8) -> Result<Vec<u8>> {
        note(format!("rbd {addr:02x} {command:02x}"));
        Err("nack".into())
    }
    fn write_block_data(&self, addr: u8, command: u8, data: &[u8]) -> Result<()> {
        note(format!("wbd {addr:02x} {command:02x} {}", hex(data)));
        self.written()
    }
    fn i2c_read_block_data(&self, addr: u8, command: u8, len: u8) -> Result<Vec<u8>> {
        note(format!("rblk {addr:02x} {command:02x} {len}"));
        let mut d = self.blocks.get(&(addr, command)).ok_or("nack")?.clone();
        d.truncate(len as usize);
        Ok(d)
    }
    fn i2c_write_block_data(&self, addr: u8, command: u8, data: &[u8]) -> Result<()> {
        note(format!("wblk {addr:02x} {command:02x} {}", hex(data)));
        self.written()
    }
    fn i2c_read(&self, addr: u8, len: usize) -> Result<Vec<u8>> {
        note(format!("rraw {addr:02x} {len}"));
        let mut raw = self.raw.lock().unwrap();
        let q = raw.get_mut(&addr).ok_or("nack")?;
        let mut d = if q.len() > 1 {
            q.pop_front().unwrap()
        } else {
            q.front().cloned().ok_or("nack")?
        };
        d.truncate(len);
        Ok(d)
    }
    fn i2c_write(&self, addr: u8, data: &[u8]) -> Result<()> {
        note(format!("wraw {addr:02x} {}", hex(data)));
        self.written()
    }
}

/// Run one family's detectors on a bus.
pub fn run(detects: &[Detect], bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    detects.iter().flat_map(|d| d(bus)).collect()
}

/// Push `colors` to the only device of `b`, returning the log of that push.
pub fn push(b: &mut Box<dyn Backend>, colors: &[Color]) -> Vec<String> {
    take();
    let id = b.devices()[0].id.clone();
    b.push(&id, colors).unwrap();
    take()
}

/// PCI ids of the first table row with this name.
pub fn ids(table: &[Card], name: &str) -> [u16; 4] {
    let c = table.iter().find(|c| c.name == name).unwrap();
    [c.ven, c.dev, c.subven, c.subdev]
}

pub fn s(lines: &[&str]) -> Vec<String> {
    lines.iter().map(|l| l.to_string()).collect()
}
