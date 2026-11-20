//! Ported from OpenRGB i2c_smbus/i2c_smbus.h (GPL-2.0-or-later). A scripted bus for tests.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use foxden_smbus::i2c::{BusInfo, Result, SharedBus, SmBus};

/// Logs every transaction, reads included. `read_byte` answers from a queue.
#[derive(Default)]
pub struct Script {
    pub info: BusInfo,
    pub log: Arc<Mutex<Vec<String>>>,
    pub registers: HashMap<(u8, u8), u8>,
    pub stream: Mutex<VecDeque<u8>>,
    pub fail_writes: bool,
}

impl Script {
    pub fn new(info: BusInfo) -> Script {
        Script {
            info,
            ..Default::default()
        }
    }

    /// The bus and a handle on its log.
    pub fn share(self) -> (SharedBus, Arc<Mutex<Vec<String>>>) {
        let log = self.log.clone();
        let bus: Box<dyn SmBus> = Box::new(self);
        (Arc::new(Mutex::new(bus)), log)
    }

    fn note(&self, entry: String) {
        self.log.lock().unwrap().push(entry);
    }

    fn written(&self, entry: String) -> Result<()> {
        self.note(entry);
        match self.fail_writes {
            true => Err("nack".into()),
            false => Ok(()),
        }
    }
}

impl SmBus for Script {
    fn info(&self) -> &BusInfo {
        &self.info
    }
    fn write_quick(&self, addr: u8) -> Result<()> {
        self.written(format!("quick {addr:02x}"))
    }
    fn read_byte(&self, addr: u8) -> Result<u8> {
        self.note(format!("read {addr:02x}"));
        self.stream.lock().unwrap().pop_front().ok_or("nack".into())
    }
    fn write_byte(&self, addr: u8, value: u8) -> Result<()> {
        self.written(format!("byte {addr:02x} {value:02x}"))
    }
    fn read_byte_data(&self, addr: u8, command: u8) -> Result<u8> {
        self.note(format!("read_data {addr:02x} {command:02x}"));
        self.registers
            .get(&(addr, command))
            .copied()
            .ok_or("nack".into())
    }
    fn write_byte_data(&self, addr: u8, command: u8, value: u8) -> Result<()> {
        self.written(format!("byte_data {addr:02x} {command:02x} {value:02x}"))
    }
    fn read_word_data(&self, addr: u8, command: u8) -> Result<u16> {
        self.note(format!("read_word {addr:02x} {command:02x}"));
        Err("nack".into())
    }
    fn write_word_data(&self, addr: u8, command: u8, value: u16) -> Result<()> {
        self.written(format!("word_data {addr:02x} {command:02x} {value:04x}"))
    }
    fn read_block_data(&self, addr: u8, command: u8) -> Result<Vec<u8>> {
        self.note(format!("read_block {addr:02x} {command:02x}"));
        Err("nack".into())
    }
    fn write_block_data(&self, addr: u8, command: u8, data: &[u8]) -> Result<()> {
        self.written(format!("block_data {addr:02x} {command:02x} {data:02x?}"))
    }
    fn i2c_read_block_data(&self, addr: u8, command: u8, len: u8) -> Result<Vec<u8>> {
        self.note(format!("read_i2c_block {addr:02x} {command:02x} {len}"));
        Err("nack".into())
    }
    fn i2c_write_block_data(&self, addr: u8, command: u8, data: &[u8]) -> Result<()> {
        self.written(format!("i2c_block {addr:02x} {command:02x} {data:02x?}"))
    }
}
