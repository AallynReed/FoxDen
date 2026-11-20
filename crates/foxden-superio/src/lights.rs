//! Ported from OpenRGB super_io/super_io_pawnio.cpp and dmiinfo/dmiinfo.cpp (GPL-2.0-or-later).

use foxden_core::{Backend, Color, Device};
use foxden_pawnio::{LEDS_VALVE, LPC_IO, Module, SharedLock};

use crate::nct6687::ISA_MUTEX;
use crate::{gigabyte_sio, msi_rgb, valve};

/// Board and case lighting reached through PawnIO, for boards whose DMI names a supported model.
pub fn lights() -> Vec<Box<dyn Backend>> {
    let dmi = dmi();
    let sio = |addr| {
        Module::open(LPC_IO)
            .ok()
            .map(|m| Sio::new(Box::new(m), addr, Some(SharedLock::new(ISA_MUTEX))))
    };
    let leds = || {
        Module::open(LEDS_VALVE)
            .ok()
            .map(|m| Box::new(m) as Box<dyn Pawn>)
    };
    let mut found = msi_rgb::detect(&dmi, &sio);
    found.extend(gigabyte_sio::detect(&dmi, &sio));
    found.extend(valve::detect(&dmi, &leds));
    found
}

/// A PawnIO module as the drivers use it; tests swap in a logger.
pub(crate) trait Pawn: Send + Sync {
    fn exec(&self, name: &str, input: &[u64], outputs: usize) -> Result<Vec<u64>, String>;

    fn read(&self, name: &str, input: &[u64]) -> Option<u64> {
        self.exec(name, input, 1).ok()?.first().copied()
    }
}

impl Pawn for Module {
    fn exec(&self, name: &str, input: &[u64], outputs: usize) -> Result<Vec<u64>, String> {
        self.execute(name, input, outputs)
    }
}

pub(crate) type OpenSio<'a> = &'a dyn Fn(u16) -> Option<Sio>;

/// One Super I/O chip at 0x2E or 0x4E, through its own LpcIO instance.
pub(crate) struct Sio {
    pawn: Box<dyn Pawn>,
    pub addr: u16,
    lock: Option<SharedLock>,
}

pub(crate) const SIO_REG_LOGDEV: u8 = 0x07;
const SIO_REG_DEVID: u8 = 0x20;
pub(crate) const SIO_ID_MASK: u16 = 0xFFF8;

impl Sio {
    pub fn new(pawn: Box<dyn Pawn>, addr: u16, lock: Option<SharedLock>) -> Sio {
        Sio { pawn, addr, lock }
    }

    pub fn locked<T>(&self, f: impl FnOnce() -> T) -> T {
        match &self.lock {
            Some(lock) => lock.run(f),
            None => f(),
        }
    }

    /// superio_enter.
    pub fn enter(&self) {
        let slot = u64::from(self.addr == 0x4E);
        let _ = self.pawn.exec("ioctl_select_slot", &[slot], 0);
        self.inb(0x87);
        self.inb(0x87);
    }

    /// Leaves config mode after each access.
    pub fn exit(&self) {
        let _ = self
            .pawn
            .exec("ioctl_pio_outb", &[self.addr as u64, 0xAA], 0);
    }

    pub fn outb(&self, reg: u8, val: u8) {
        let _ = self
            .pawn
            .exec("ioctl_superio_outb", &[reg as u64, val as u64], 0);
    }

    pub fn inb(&self, reg: u8) -> u8 {
        self.pawn
            .read("ioctl_superio_inb", &[reg as u64])
            .map_or(0xFF, |v| v as u8)
    }

    pub fn devid(&self) -> u16 {
        (self.inb(SIO_REG_DEVID) as u16) << 8 | self.inb(SIO_REG_DEVID + 1) as u16
    }
}

/// One family's protocol: `first` is the push that takes software control.
pub(crate) trait Driver: Send {
    fn write(&mut self, colors: &[Color], first: bool);
}

/// The Backend every family here shares: id check, first-push setup, unchanged frames skipped.
pub(crate) struct Light<D> {
    name: &'static str,
    device: Device,
    driver: D,
    started: bool,
    sent: Option<Vec<Color>>,
}

impl<D: Driver + 'static> Light<D> {
    pub fn boxed(name: &'static str, device: Device, driver: D) -> Box<dyn Backend> {
        Box::new(Light {
            name,
            device,
            driver,
            started: false,
            sent: None,
        })
    }
}

impl<D: Driver> Backend for Light<D> {
    fn name(&self) -> &str {
        self.name
    }

    fn devices(&self) -> Vec<Device> {
        vec![self.device.clone()]
    }

    fn push(&mut self, device_id: &str, colors: &[Color]) -> Result<(), String> {
        if device_id != self.device.id {
            return Err(format!("unknown device {device_id}"));
        }
        let mut colors = colors.to_vec();
        colors.resize(self.device.leds.len(), Color::BLACK);
        if self.sent.as_ref() == Some(&colors) {
            return Ok(());
        }
        self.driver.write(&colors, !self.started);
        self.started = true;
        self.sent = Some(colors);
        Ok(())
    }
}

/// DMIInfo: baseboard maker and model, and the system model.
#[derive(Clone, Debug, Default)]
pub(crate) struct Dmi {
    pub manufacturer: String,
    pub mainboard: String,
    pub product_name: String,
}

pub(crate) fn dmi() -> Dmi {
    Dmi {
        manufacturer: bios_value("BaseBoardManufacturer").unwrap_or_default(),
        mainboard: bios_value("BaseBoardProduct").unwrap_or_default(),
        product_name: bios_value("SystemProductName").unwrap_or_default(),
    }
}

/// Whether the processor is an AMD one, which decides how some boards are reached.
pub(crate) fn cpu_is_amd() -> bool {
    let key = "HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0";
    registry_value(key, "VendorIdentifier").is_some_and(|v| v == "AuthenticAMD")
}

fn bios_value(name: &str) -> Option<String> {
    registry_value("HARDWARE\\DESCRIPTION\\System\\BIOS", name)
}

fn registry_value(key: &str, name: &str) -> Option<String> {
    use std::ffi::c_void;
    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegGetValueW(
            key: *mut c_void,
            sub_key: *const u16,
            value: *const u16,
            flags: u32,
            kind: *mut u32,
            data: *mut c_void,
            len: *mut u32,
        ) -> i32;
    }
    let hklm = 0x8000_0002u32 as i32 as isize as *mut c_void;
    let sub: Vec<u16> = key.encode_utf16().chain([0]).collect();
    let value: Vec<u16> = name.encode_utf16().chain([0]).collect();
    let mut buf = [0u16; 256];
    let mut len = (buf.len() * 2) as u32;
    // SAFETY: both names are NUL-terminated and `len` is the byte size of `buf`.
    let status = unsafe {
        RegGetValueW(
            hklm,
            sub.as_ptr(),
            value.as_ptr(),
            0x2,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut len,
        )
    };
    if status != 0 {
        return None;
    }
    let n = (len as usize / 2).min(buf.len());
    let text = String::from_utf16_lossy(&buf[..n]);
    Some(text.trim_end_matches('\0').trim().to_string())
}

#[cfg(test)]
pub(crate) mod fake {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use super::*;

    pub type Log = Arc<Mutex<Vec<String>>>;

    /// Logs every call as `name args..` in hex and answers one-output calls from a table.
    pub struct Fake {
        pub log: Log,
        pub reads: HashMap<(&'static str, u64), u64>,
    }

    impl Pawn for Fake {
        fn exec(&self, name: &str, input: &[u64], outputs: usize) -> Result<Vec<u64>, String> {
            let name = name.trim_start_matches("ioctl_");
            let args: Vec<String> = input.iter().map(|v| format!("{v:02x}")).collect();
            self.log
                .lock()
                .unwrap()
                .push([name.to_string(), args.join(" ")].join(" ").trim().into());
            if outputs == 0 {
                return Ok(Vec::new());
            }
            let key = input.first().copied().unwrap_or(0);
            self.reads
                .iter()
                .find(|((n, k), _)| *n == name && *k == key)
                .map(|(_, &v)| vec![v])
                .ok_or_else(|| format!("no answer for {name}"))
        }
    }

    pub fn take(log: &Log) -> Vec<String> {
        std::mem::take(&mut *log.lock().unwrap())
    }

    pub fn dmi(manufacturer: &str, mainboard: &str, product_name: &str) -> Dmi {
        Dmi {
            manufacturer: manufacturer.into(),
            mainboard: mainboard.into(),
            product_name: product_name.into(),
        }
    }

    /// A chip id per address, plus extra superio_inb answers.
    pub fn chips(
        log: &Log,
        ids: &[(u16, u16)],
        extra: &[(u8, u8)],
    ) -> impl Fn(u16) -> Option<Sio> + use<> {
        let log = log.clone();
        let ids = ids.to_vec();
        let extra = extra.to_vec();
        move |addr| {
            let id = ids
                .iter()
                .find(|(a, _)| *a == addr)
                .map_or(0xFFFF, |&(_, id)| id);
            let mut reads = HashMap::new();
            reads.insert(("superio_inb", 0x20), (id >> 8) as u64);
            reads.insert(("superio_inb", 0x21), (id & 0xFF) as u64);
            for &(reg, val) in &extra {
                reads.insert(("superio_inb", reg as u64), val as u64);
            }
            let fake = Fake {
                log: log.clone(),
                reads,
            };
            Some(Sio::new(Box::new(fake), addr, None))
        }
    }
}
