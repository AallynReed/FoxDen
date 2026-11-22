//! The bus interface every SMBus and graphics-card I2C driver uses.

use std::sync::{Arc, Mutex, MutexGuard};

use foxden_core::Backend;

/// PCI identity of the controller behind a bus, which PCI detectors match on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BusInfo {
    pub name: String,
    pub pci_vendor: u16,
    pub pci_device: u16,
    pub pci_subsystem_vendor: u16,
    pub pci_subsystem_device: u16,
}

pub type Result<T> = std::result::Result<T, String>;

pub trait SmBus: Send {
    fn info(&self) -> &BusInfo;
    fn write_quick(&self, addr: u8) -> Result<()>;
    fn read_byte(&self, addr: u8) -> Result<u8>;
    fn write_byte(&self, addr: u8, value: u8) -> Result<()>;
    fn read_byte_data(&self, addr: u8, command: u8) -> Result<u8>;
    fn write_byte_data(&self, addr: u8, command: u8, value: u8) -> Result<()>;
    fn read_word_data(&self, addr: u8, command: u8) -> Result<u16>;
    fn write_word_data(&self, addr: u8, command: u8, value: u16) -> Result<()>;
    /// SMBus block read: the device sends the length first. At most 32 bytes.
    fn read_block_data(&self, addr: u8, command: u8) -> Result<Vec<u8>>;
    fn write_block_data(&self, addr: u8, command: u8, data: &[u8]) -> Result<()>;
    /// I2C block read of `len` bytes, no length byte on the wire.
    fn i2c_read_block_data(&self, addr: u8, command: u8, len: u8) -> Result<Vec<u8>>;
    fn i2c_write_block_data(&self, addr: u8, command: u8, data: &[u8]) -> Result<()>;
    /// Raw I2C read of up to `len` bytes, i2c_read_block: no register, no length cap.
    fn i2c_read(&self, _addr: u8, _len: usize) -> Result<Vec<u8>> {
        Err("this bus has no raw I2C transfer".into())
    }
    /// Raw I2C write, i2c_write_block: no register, no length cap.
    fn i2c_write(&self, _addr: u8, _data: &[u8]) -> Result<()> {
        Err("this bus has no raw I2C transfer".into())
    }
}

/// A bus shared by every device on it. Lock it for each transaction, never across a sleep.
pub type SharedBus = Arc<Mutex<Box<dyn SmBus>>>;

pub fn lock(bus: &SharedBus) -> MutexGuard<'_, Box<dyn SmBus>> {
    bus.lock().unwrap_or_else(|e| e.into_inner())
}

/// What an SMBus or GPU driver family offers: look at one bus, return a backend per device found.
pub type Detect = fn(&SharedBus) -> Vec<Box<dyn Backend>>;

/// A bus that records writes and answers reads from a table, for tests.
#[derive(Default)]
pub struct FakeBus {
    pub info: BusInfo,
    pub log: Mutex<Vec<String>>,
    /// (addr, command) -> byte answered by byte and word reads.
    pub bytes: Mutex<std::collections::HashMap<(u8, u8), u8>>,
}

impl FakeBus {
    fn note(&self, entry: String) {
        self.log.lock().unwrap().push(entry);
    }

    fn get(&self, addr: u8, command: u8) -> Result<u8> {
        self.bytes
            .lock()
            .unwrap()
            .get(&(addr, command))
            .copied()
            .ok_or_else(|| format!("no device at {addr:#04x}"))
    }

    pub fn writes(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

impl SmBus for FakeBus {
    fn info(&self) -> &BusInfo {
        &self.info
    }
    fn write_quick(&self, addr: u8) -> Result<()> {
        self.note(format!("quick {addr:02x}"));
        Ok(())
    }
    fn read_byte(&self, addr: u8) -> Result<u8> {
        self.get(addr, 0)
    }
    fn write_byte(&self, addr: u8, value: u8) -> Result<()> {
        self.note(format!("byte {addr:02x} {value:02x}"));
        Ok(())
    }
    fn read_byte_data(&self, addr: u8, command: u8) -> Result<u8> {
        self.get(addr, command)
    }
    fn write_byte_data(&self, addr: u8, command: u8, value: u8) -> Result<()> {
        self.note(format!("byte_data {addr:02x} {command:02x} {value:02x}"));
        Ok(())
    }
    fn read_word_data(&self, addr: u8, command: u8) -> Result<u16> {
        Ok(self.get(addr, command)? as u16)
    }
    fn write_word_data(&self, addr: u8, command: u8, value: u16) -> Result<()> {
        self.note(format!("word_data {addr:02x} {command:02x} {value:04x}"));
        Ok(())
    }
    fn read_block_data(&self, addr: u8, command: u8) -> Result<Vec<u8>> {
        Ok(vec![self.get(addr, command)?])
    }
    fn write_block_data(&self, addr: u8, command: u8, data: &[u8]) -> Result<()> {
        self.note(format!("block_data {addr:02x} {command:02x} {data:02x?}"));
        Ok(())
    }
    fn i2c_read_block_data(&self, addr: u8, command: u8, len: u8) -> Result<Vec<u8>> {
        Ok(vec![self.get(addr, command)?; len as usize])
    }
    fn i2c_write_block_data(&self, addr: u8, command: u8, data: &[u8]) -> Result<()> {
        self.note(format!("i2c_block {addr:02x} {command:02x} {data:02x?}"));
        Ok(())
    }
}
