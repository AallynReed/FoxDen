// SPDX-License-Identifier: GPL-2.0-or-later

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use super::Ctx;
use crate::i2c::{BusInfo, Result, SmBus};

/// An ENE chip: a 16-bit register pointer set through command 0x00 and a register file.
#[derive(Clone, Default)]
pub struct Ene {
    pub ptr: u16,
    pub mem: HashMap<u16, u8>,
}

impl Ene {
    /// A chip whose version string and config table are filled in.
    pub fn new(version: &str, config: &[(u8, u8)]) -> Self {
        let mut e = Ene::default();
        for (i, b) in version.bytes().enumerate() {
            e.mem.insert(0x1000 + i as u16, b);
        }
        for &(i, v) in config {
            e.mem.insert(0x1C00 + i as u16, v);
        }
        e
    }

    fn get(&self) -> u8 {
        self.mem.get(&self.ptr).copied().unwrap_or(0)
    }
}

#[derive(Default)]
pub struct State {
    /// Addresses that ACK quick writes and byte reads.
    pub present: HashSet<u8>,
    pub bytes: HashMap<(u8, u8), u8>,
    pub blocks: HashMap<(u8, u8), Vec<u8>>,
    pub ene: HashMap<u8, Ene>,
    /// ENE DRAM still answering at 0x77, waiting for an address.
    pub unmapped: Vec<Ene>,
    pub block_writes_fail: bool,
}

pub struct Fake {
    info: BusInfo,
    log: Arc<Mutex<Vec<String>>>,
    st: Mutex<State>,
}

pub struct Handle {
    pub log: Arc<Mutex<Vec<String>>>,
    pub ctx: Ctx,
}

impl Handle {
    pub fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.log.lock().unwrap())
    }
}

pub fn bus(info: BusInfo, st: State) -> Handle {
    let log = Arc::new(Mutex::new(Vec::new()));
    let fake = Fake {
        info,
        log: log.clone(),
        st: Mutex::new(st),
    };
    let sleep_log = log.clone();
    let ctx = Ctx {
        bus: Arc::new(Mutex::new(Box::new(fake) as Box<dyn SmBus>)),
        sleep: Arc::new(move |ms| sleep_log.lock().unwrap().push(format!("sleep {ms}"))),
    };
    Handle { log, ctx }
}

pub fn info(vendor: u16, device: u16, subvendor: u16, subdevice: u16) -> BusInfo {
    BusInfo {
        name: "fake".into(),
        pci_vendor: vendor,
        pci_device: device,
        pci_subsystem_vendor: subvendor,
        pci_subsystem_device: subdevice,
    }
}

const NACK: &str = "nack";

impl Fake {
    fn note(&self, s: String) {
        self.log.lock().unwrap().push(s);
    }

    fn here(st: &State, addr: u8) -> bool {
        st.present.contains(&addr)
            || st.ene.contains_key(&addr)
            || (addr == 0x77 && !st.unmapped.is_empty())
    }

    fn ene(st: &mut State, addr: u8) -> Option<&mut Ene> {
        if let Some(e) = st.ene.get_mut(&addr) {
            return Some(e);
        }
        if addr == 0x77 {
            return st.unmapped.first_mut();
        }
        None
    }
}

impl SmBus for Fake {
    fn info(&self) -> &BusInfo {
        &self.info
    }
    fn write_quick(&self, addr: u8) -> Result<()> {
        self.note(format!("quick {addr:02x}"));
        let st = self.st.lock().unwrap();
        if Self::here(&st, addr) {
            Ok(())
        } else {
            Err(NACK.into())
        }
    }
    fn read_byte(&self, addr: u8) -> Result<u8> {
        self.note(format!("rb {addr:02x}"));
        let st = self.st.lock().unwrap();
        if Self::here(&st, addr) {
            Ok(0)
        } else {
            Err(NACK.into())
        }
    }
    fn write_byte(&self, addr: u8, value: u8) -> Result<()> {
        self.note(format!("wb {addr:02x} {value:02x}"));
        Ok(())
    }
    fn read_byte_data(&self, addr: u8, command: u8) -> Result<u8> {
        self.note(format!("rbd {addr:02x} {command:02x}"));
        let mut st = self.st.lock().unwrap();
        if let Some(&v) = st.bytes.get(&(addr, command)) {
            return Ok(v);
        }
        let present = st.present.contains(&addr);
        match Self::ene(&mut st, addr) {
            Some(e) => Ok(match command {
                0xA0..=0xAF => command - 0xA0,
                0x81 => e.get(),
                _ => 0,
            }),
            None if present => Ok(0),
            None => Err(NACK.into()),
        }
    }
    fn write_byte_data(&self, addr: u8, command: u8, value: u8) -> Result<()> {
        self.note(format!("wbd {addr:02x} {command:02x} {value:02x}"));
        let mut st = self.st.lock().unwrap();
        let moving = addr == 0x77 && !st.ene.contains_key(&0x77);
        let Some(e) = Self::ene(&mut st, addr) else {
            return if Self::here(&st, addr) {
                Ok(())
            } else {
                Err(NACK.into())
            };
        };
        if command != 0x01 {
            return Ok(());
        }
        e.mem.insert(e.ptr, value);
        if moving && e.ptr == 0x80F9 {
            let mut e = st.unmapped.remove(0);
            e.ptr = 0;
            st.ene.insert(value >> 1, e);
        }
        Ok(())
    }
    fn read_word_data(&self, addr: u8, command: u8) -> Result<u16> {
        self.note(format!("rwd {addr:02x} {command:02x}"));
        Err(NACK.into())
    }
    fn write_word_data(&self, addr: u8, command: u8, value: u16) -> Result<()> {
        self.note(format!("wwd {addr:02x} {command:02x} {value:04x}"));
        let mut st = self.st.lock().unwrap();
        if let Some(e) = Self::ene(&mut st, addr) {
            if command == 0x00 {
                e.ptr = value.swap_bytes();
            }
            return Ok(());
        }
        if Self::here(&st, addr) {
            Ok(())
        } else {
            Err(NACK.into())
        }
    }
    fn read_block_data(&self, addr: u8, command: u8) -> Result<Vec<u8>> {
        self.note(format!("rblk {addr:02x} {command:02x}"));
        let st = self.st.lock().unwrap();
        st.blocks
            .get(&(addr, command))
            .cloned()
            .ok_or_else(|| NACK.into())
    }
    fn write_block_data(&self, addr: u8, command: u8, data: &[u8]) -> Result<()> {
        self.note(format!("wblk {addr:02x} {command:02x} {data:02x?}"));
        let mut st = self.st.lock().unwrap();
        if st.block_writes_fail {
            return Err(NACK.into());
        }
        if let Some(e) = Self::ene(&mut st, addr) {
            for (i, &b) in data.iter().enumerate() {
                e.mem.insert(e.ptr + i as u16, b);
            }
            return Ok(());
        }
        if Self::here(&st, addr) {
            Ok(())
        } else {
            Err(NACK.into())
        }
    }
    fn i2c_read_block_data(&self, addr: u8, command: u8, len: u8) -> Result<Vec<u8>> {
        self.note(format!("i2c_rblk {addr:02x} {command:02x} {len}"));
        Err(NACK.into())
    }
    fn i2c_write_block_data(&self, addr: u8, command: u8, data: &[u8]) -> Result<()> {
        self.note(format!("i2c_wblk {addr:02x} {command:02x} {data:02x?}"));
        Err(NACK.into())
    }
}

/// Log lines for an ENE register write.
pub fn ene_w(addr: u8, reg: u16, val: u8) -> Vec<String> {
    vec![
        format!("wwd {addr:02x} 00 {:04x}", reg.swap_bytes()),
        format!("wbd {addr:02x} 01 {val:02x}"),
    ]
}

/// Log lines for an ENE register read.
pub fn ene_r(addr: u8, reg: u16) -> Vec<String> {
    vec![
        format!("wwd {addr:02x} 00 {:04x}", reg.swap_bytes()),
        format!("rbd {addr:02x} 81"),
    ]
}

/// Log lines for an ENE probe on a chip that passes.
pub fn ene_test(addr: u8) -> Vec<String> {
    let mut v = vec![format!("rb {addr:02x}")];
    v.extend((0xA0..0xB0).map(|i| format!("rbd {addr:02x} {i:02x}")));
    v.extend((0..16).flat_map(|i| ene_r(addr, 0x1030 + i)));
    v
}

/// Log lines for ENE setup: version string then config table.
pub fn ene_ctor(addr: u8) -> Vec<String> {
    (0..16)
        .flat_map(|i| ene_r(addr, 0x1000 + i))
        .chain((0..64).flat_map(|i| ene_r(addr, 0x1C00 + i)))
        .collect()
}
