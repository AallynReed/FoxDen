// SPDX-License-Identifier: GPL-2.0-or-later

use std::cell::RefCell;

use foxden_core::{Backend, Color, Device, DeviceKind};

use crate::i2c::{BusInfo, Result, SharedBus, SmBus, lock};

/// A sleep, recorded instead of slept in tests.
pub fn delay(ms: u64) {
    #[cfg(test)]
    super::fake::log(super::fake::Op::Sleep(ms));
    #[cfg(not(test))]
    std::thread::sleep(std::time::Duration::from_millis(ms));
}

/// Whether DRAM can sit on this bus.
pub fn is_dram_smbus(info: &BusInfo) -> bool {
    const INTEL: &[u16] = &[
        0x3A30, 0xA123, 0x2085, 0xA2A3, 0xA323, 0x06A3, 0xA3A3, 0x43A3, 0x7AA3, 0x7A23, 0x7F23,
    ];
    (info.pci_vendor == 0x1022 && info.pci_device == 0x790B)
        || (info.pci_vendor == 0x8086 && INTEL.contains(&info.pci_device))
}

/// The bus with int results (negative on error), locked per call.
/// Failures are remembered so a push can report them while still running on.
pub struct Io<'a> {
    bus: &'a SharedBus,
    err: RefCell<Option<String>>,
}

impl<'a> Io<'a> {
    pub fn new(bus: &'a SharedBus) -> Self {
        Io {
            bus,
            err: RefCell::new(None),
        }
    }

    /// One call without recording its failure, for drivers that retry.
    pub fn raw<T>(&self, f: impl FnOnce(&dyn SmBus) -> Result<T>) -> Result<T> {
        let guard = lock(self.bus);
        f(guard.as_ref())
    }

    pub fn note(&self, e: String) {
        self.err.borrow_mut().get_or_insert(e);
    }

    pub fn take_err(&self) -> Option<String> {
        self.err.borrow_mut().take()
    }

    fn int<T: Into<i32>>(&self, r: Result<T>) -> i32 {
        match r {
            Ok(v) => v.into(),
            Err(e) => {
                self.note(e);
                -1
            }
        }
    }

    fn ack(&self, r: Result<()>) -> i32 {
        self.int(r.map(|_| 0))
    }

    pub fn quick(&self, a: u8) -> i32 {
        self.ack(self.raw(|b| b.write_quick(a)))
    }
    pub fn read_byte(&self, a: u8) -> i32 {
        self.int(self.raw(|b| b.read_byte(a)))
    }
    pub fn write_byte(&self, a: u8, v: u8) -> i32 {
        self.ack(self.raw(|b| b.write_byte(a, v)))
    }
    pub fn read_byte_data(&self, a: u8, c: u8) -> i32 {
        self.int(self.raw(|b| b.read_byte_data(a, c)))
    }
    pub fn write_byte_data(&self, a: u8, c: u8, v: u8) -> i32 {
        self.ack(self.raw(|b| b.write_byte_data(a, c, v)))
    }
    pub fn read_word_data(&self, a: u8, c: u8) -> i32 {
        self.int(self.raw(|b| b.read_word_data(a, c)))
    }
    pub fn write_word_data(&self, a: u8, c: u8, v: u16) -> i32 {
        self.ack(self.raw(|b| b.write_word_data(a, c, v)))
    }
    pub fn write_block_data(&self, a: u8, c: u8, data: &[u8]) -> i32 {
        self.ack(self.raw(|b| b.write_block_data(a, c, data)))
    }

    /// The ENE-style indirect register write used by Crucial and T-Force: pointer word, then value.
    pub fn ene_write(&self, dev: u8, reg: u16, val: u8) {
        self.write_word_data(dev, 0x00, reg.swap_bytes());
        self.write_byte_data(dev, 0x01, val);
    }

    pub fn ene_read(&self, dev: u8, reg: u16) -> u8 {
        self.write_word_data(dev, 0x00, reg.swap_bytes());
        self.read_byte_data(dev, 0x81) as u8
    }
}

/// One family's direct-mode protocol over its zones, each zone one Device.
pub trait Proto: Send {
    /// Enters the software-control mode.
    fn start(&mut self, io: &Io, zones: &[Vec<Color>]);
    /// Pushes one zone's colours.
    fn frame(&mut self, io: &Io, zones: &[Vec<Color>], zone: usize);
    /// Entering the mode already sends every LED.
    const START_SENDS_FRAME: bool = false;
}

pub struct Driver<P: Proto> {
    bus: SharedBus,
    name: &'static str,
    devices: Vec<Device>,
    zones: Vec<Vec<Color>>,
    sent: Vec<Option<Vec<Color>>>,
    started: bool,
    pub proto: P,
}

impl<P: Proto> Driver<P> {
    pub fn new(bus: &SharedBus, name: &'static str, devices: Vec<Device>, proto: P) -> Self {
        Driver {
            bus: bus.clone(),
            name,
            zones: devices
                .iter()
                .map(|d| vec![Color(0, 0, 0); d.leds.len()])
                .collect(),
            sent: vec![None; devices.len()],
            devices,
            started: false,
            proto,
        }
    }
}

impl<P: Proto> Backend for Driver<P> {
    fn name(&self) -> &str {
        self.name
    }

    fn devices(&self) -> Vec<Device> {
        self.devices.clone()
    }

    fn push(&mut self, device_id: &str, colors: &[Color]) -> std::result::Result<(), String> {
        let Some(z) = self.devices.iter().position(|d| d.id == device_id) else {
            return Err(format!("unknown device {device_id}"));
        };
        let mut colors = colors.to_vec();
        colors.resize(self.zones[z].len(), Color(0, 0, 0));
        if self.sent[z].as_ref() == Some(&colors) {
            return Ok(());
        }
        self.zones[z] = colors.clone();
        let io = Io::new(&self.bus);
        if !self.started {
            self.proto.start(&io, &self.zones);
            if let Some(e) = io.take_err() {
                return Err(e);
            }
            self.started = true;
            if P::START_SENDS_FRAME {
                self.sent = self.zones.iter().cloned().map(Some).collect();
                return Ok(());
            }
        }
        self.proto.frame(&io, &self.zones, z);
        match io.take_err() {
            Some(e) => {
                self.sent[z] = None;
                Err(e)
            }
            None => {
                self.sent[z] = Some(colors);
                Ok(())
            }
        }
    }
}

/// A vertical stick of `leds` in memory slot `slot`, LED 0 at the top.
pub fn stick(id: String, name: String, slot: usize, leds: usize) -> Device {
    let rect = [0.62 + slot as f32 * 0.035, 0.08, 0.025, 0.3];
    Device::grid(&id, &name, DeviceKind::Memory, rect, 1, leds as u32)
}
