//! Ported from OpenRGB DetectionManager.cpp and i2c_smbus/i2c_amd_gpu.h (GPL-2.0-or-later).

use std::cell::RefCell;
use std::time::Duration;

use foxrgb_core::{Backend, Color, Device, DeviceKind};
use foxrgb_smbus::i2c::{BusInfo, SharedBus, lock};

/// One REGISTER_I2C_PCI_DETECTOR line.
pub struct Card {
    pub name: &'static str,
    pub ven: u16,
    pub dev: u16,
    pub subven: u16,
    pub subdev: u16,
    pub addr: u8,
}

pub const fn c(name: &'static str, ven: u16, dev: u16, subven: u16, subdev: u16, addr: u8) -> Card {
    Card {
        name,
        ven,
        dev,
        subven,
        subdev,
        addr,
    }
}

pub fn info(bus: &SharedBus) -> BusInfo {
    lock(bus).info().clone()
}

/// Rows whose four PCI ids equal the bus's, as BackgroundDetectI2CPCIDevices compares them.
pub fn matching<'a>(bus: &SharedBus, cards: &'a [Card]) -> Vec<&'a Card> {
    let i = info(bus);
    cards
        .iter()
        .filter(|c| {
            c.ven == i.pci_vendor
                && c.dev == i.pci_device
                && c.subven == i.pci_subsystem_vendor
                && c.subdev == i.pci_subsystem_device
        })
        .collect()
}

/// Run `probe` for every matching row and collect what it finds.
pub fn detect_each(
    bus: &SharedBus,
    cards: &[Card],
    probe: impl Fn(&SharedBus, &Card) -> Option<Box<dyn Backend>>,
) -> Vec<Box<dyn Backend>> {
    matching(bus, cards)
        .into_iter()
        .filter_map(|card| probe(bus, card))
        .collect()
}

pub fn is_amd_gpu_i2c_bus(info: &BusInfo) -> bool {
    [
        "AMD ADL",
        "AMDGPU DM i2c OEM bus",
        "AMDGPU i2c bit bus OEM 0x97",
    ]
    .iter()
    .any(|n| info.name.starts_with(n))
}

pub const AMD_GPU_VEN: u16 = 0x1002;

pub fn sleep(d: Duration) {
    #[cfg(test)]
    super::fake::note(format!("sleep {}ms", d.as_millis()));
    #[cfg(not(test))]
    std::thread::sleep(d);
}

/// Bus calls shaped like OpenRGB's: one lock per transaction, reads give -1 on error, writes
/// never stop the sequence and the first error is kept for the caller.
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

    fn keep(&self, r: Result<(), String>) {
        if let Err(e) = r {
            self.err.borrow_mut().get_or_insert(e);
        }
    }

    pub fn result(self) -> Result<(), String> {
        match self.err.into_inner() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    pub fn read(&self, addr: u8, cmd: u8) -> i32 {
        lock(self.bus)
            .read_byte_data(addr, cmd)
            .map_or(-1, i32::from)
    }

    pub fn read_byte(&self, addr: u8) -> i32 {
        lock(self.bus).read_byte(addr).map_or(-1, i32::from)
    }

    pub fn quick_ok(&self, addr: u8) -> bool {
        lock(self.bus).write_quick(addr).is_ok()
    }

    pub fn write(&self, addr: u8, cmd: u8, value: u8) {
        let r = lock(self.bus).write_byte_data(addr, cmd, value);
        self.keep(r);
    }

    /// I2C block read that counts only when the device returned all `len` bytes.
    pub fn read_block(&self, addr: u8, cmd: u8, len: u8) -> Option<Vec<u8>> {
        lock(self.bus)
            .i2c_read_block_data(addr, cmd, len)
            .ok()
            .filter(|d| d.len() == len as usize)
    }

    pub fn write_block(&self, addr: u8, cmd: u8, data: &[u8]) {
        let r = lock(self.bus).i2c_write_block_data(addr, cmd, data);
        self.keep(r);
    }

    /// i2c_read_block into a zeroed buffer of `len` bytes; None when the read failed.
    pub fn raw_read(&self, addr: u8, len: usize) -> Option<Vec<u8>> {
        let mut d = lock(self.bus).i2c_read(addr, len).ok()?;
        d.resize(len, 0);
        Some(d)
    }

    /// i2c_write_block; true when it went through.
    pub fn raw_write(&self, addr: u8, data: &[u8]) -> bool {
        let r = lock(self.bus).i2c_write(addr, data);
        let ok = r.is_ok();
        self.keep(r);
        ok
    }
}

/// The C string at the start of a buffer, as std::string((char*)buf) reads it.
pub fn c_string(buf: &[u8]) -> String {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

/// The protocol half of a device: OpenRGB's DeviceUpdateMode for the direct mode, and
/// DeviceUpdateLEDs.
pub trait Driver: Send {
    fn setup(&mut self, io: &Io, colors: &[Color]);
    /// Whether `setup` already writes the LEDs, as when DeviceUpdateMode calls DeviceUpdateLEDs.
    fn setup_writes_leds(&self) -> bool {
        false
    }
    /// `prev` is the last frame sent, for drivers that update one zone or LED at a time.
    fn update(&mut self, io: &Io, colors: &[Color], prev: Option<&[Color]>);
}

pub struct Gpu {
    bus: SharedBus,
    device: Device,
    driver: Box<dyn Driver>,
    ready: bool,
    sent: Option<Vec<Color>>,
}

pub const RECT: [f32; 4] = [0.06, 0.30, 0.40, 0.05];

pub fn device_id(tag: &str, bus: &SharedBus, addr: u8) -> String {
    format!("gpu:{tag}:{}:{addr:02x}", info(bus).name)
}

/// LEDs in one row across the card.
pub fn row(id: String, name: &str, count: u32) -> Device {
    Device::grid(&id, name, DeviceKind::Gpu, RECT, count, 1)
}

impl Gpu {
    pub fn boxed(
        bus: &SharedBus,
        device: Device,
        driver: impl Driver + 'static,
    ) -> Box<dyn Backend> {
        Box::new(Gpu {
            bus: bus.clone(),
            device,
            driver: Box::new(driver),
            ready: false,
            sent: None,
        })
    }
}

impl Backend for Gpu {
    fn name(&self) -> &str {
        "gpu"
    }

    fn devices(&self) -> Vec<Device> {
        vec![self.device.clone()]
    }

    fn push(&mut self, device_id: &str, colors: &[Color]) -> Result<(), String> {
        if device_id != self.device.id {
            return Err(format!("unknown device {device_id}"));
        }
        if colors.len() != self.device.leds.len() {
            return Err(format!(
                "{} expects {} colours, got {}",
                self.device.name,
                self.device.leds.len(),
                colors.len()
            ));
        }
        if self.sent.as_deref() == Some(colors) {
            return Ok(());
        }
        let io = Io::new(&self.bus);
        if self.ready {
            self.driver.update(&io, colors, self.sent.as_deref());
        } else {
            self.driver.setup(&io, colors);
            if !self.driver.setup_writes_leds() {
                self.driver.update(&io, colors, None);
            }
        }
        io.result()?;
        self.ready = true;
        self.sent = Some(colors.to_vec());
        Ok(())
    }
}

/// Indices of the entries in `colors` that differ from `prev`, or all of them.
pub fn changed(colors: &[Color], prev: Option<&[Color]>) -> Vec<usize> {
    (0..colors.len())
        .filter(|&i| prev.is_none_or(|p| p.get(i) != Some(&colors[i])))
        .collect()
}
