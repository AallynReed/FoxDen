//! Ported from OpenRGB i2c_smbus/Windows/i2c_smbus_amdadl.cpp (GPL-2.0-or-later).

use std::ffi::{CStr, c_char, c_int, c_void};
use std::mem::size_of;
use std::ptr::null_mut;
use std::sync::{Arc, Mutex};

use foxrgb_smbus::i2c::{BusInfo, Result, SharedBus, SmBus};
use windows_sys::Win32::Foundation::HMODULE;
use windows_sys::Win32::System::LibraryLoader::{
    GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
};
use windows_sys::Win32::System::Memory::{GetProcessHeap, HeapAlloc, HeapFree};

pub(crate) const BUS_NAME: &str = "AMD ADL";
const AMD_GPU_VEN: u16 = 0x1002;
const ADL_OK: c_int = 0;
const ADL_MAX_PATH: usize = 256;
const ACTION_READ: c_int = 1;
const ACTION_WRITE: c_int = 2;
const BLOCK_MAX: usize = 32;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct AdlI2c {
    size: c_int,
    line: c_int,
    address: c_int,
    offset: c_int,
    action: c_int,
    speed: c_int,
    data_size: c_int,
    data: *mut c_char,
}

#[repr(C)]
#[allow(dead_code)]
struct AdapterInfoX2 {
    size: c_int,
    adapter_index: c_int,
    udid: [c_char; ADL_MAX_PATH],
    bus_number: c_int,
    device_number: c_int,
    function_number: c_int,
    vendor_id: c_int,
    adapter_name: [c_char; ADL_MAX_PATH],
    display_name: [c_char; ADL_MAX_PATH],
    present: c_int,
    exist: c_int,
    driver_path: [c_char; ADL_MAX_PATH],
    driver_path_ext: [c_char; ADL_MAX_PATH],
    pnp_string: [c_char; ADL_MAX_PATH],
    os_display_index: c_int,
    info_mask: c_int,
    info_value: c_int,
}

/// The ADLI2C request i2c_smbus_xfer and i2c_xfer build: line 1, 100 kHz, 8-bit address.
fn request(addr: u8, offset: u8, action: c_int, buf: &mut [u8]) -> AdlI2c {
    AdlI2c {
        size: size_of::<AdlI2c>() as c_int,
        line: 1,
        address: (addr as c_int) << 1,
        offset: offset as c_int,
        action,
        speed: 100,
        data_size: buf.len() as c_int,
        data: buf.as_mut_ptr().cast(),
    }
}

/// Vendor, device, subsystem vendor and subsystem device from an adapter's PNP string.
fn parse_pnp(pnp: &str) -> Option<[u16; 4]> {
    let hex = |at: usize| -> Option<u16> {
        let s = pnp.get(at..)?;
        let digits: String = s
            .chars()
            .take(4)
            .take_while(|c| c.is_ascii_hexdigit())
            .collect();
        u16::from_str_radix(&digits, 16).ok()
    };
    let ven = pnp.find("VEN_")?;
    let dev = pnp.find("DEV_")?;
    let sub = pnp.find("SUBSYS_")?;
    Some([hex(ven + 4)?, hex(dev + 4)?, hex(sub + 11)?, hex(sub + 7)?])
}

type Call = Box<dyn Fn(&mut AdlI2c) -> c_int + Send>;

pub struct Bus {
    info: BusInfo,
    call: Call,
}

impl Bus {
    fn read(&self, addr: u8, command: u8, len: usize) -> Result<Vec<u8>> {
        let mut buf = vec![0; len];
        let mut req = request(addr, command, ACTION_READ, &mut buf);
        match (self.call)(&mut req) {
            ADL_OK => Ok(buf),
            e => Err(format!("ADL I2C read at {addr:#04x} failed ({e})")),
        }
    }

    /// An SMBus write puts the register before the data.
    fn write(&self, addr: u8, bytes: &[u8]) -> Result<()> {
        let mut buf = bytes.to_vec();
        let mut req = request(addr, 0, ACTION_WRITE, &mut buf);
        match (self.call)(&mut req) {
            e if e < 0 => Err(format!("ADL I2C write at {addr:#04x} failed ({e})")),
            _ => Ok(()),
        }
    }
}

impl SmBus for Bus {
    fn info(&self) -> &BusInfo {
        &self.info
    }
    fn write_quick(&self, _addr: u8) -> Result<()> {
        Err("ADL has no quick command".into())
    }
    fn read_byte(&self, addr: u8) -> Result<u8> {
        Ok(self.read(addr, 0, 1)?[0])
    }
    fn write_byte(&self, addr: u8, value: u8) -> Result<()> {
        self.write(addr, &[value])
    }
    fn read_byte_data(&self, addr: u8, command: u8) -> Result<u8> {
        Ok(self.read(addr, command, 1)?[0])
    }
    fn write_byte_data(&self, addr: u8, command: u8, value: u8) -> Result<()> {
        self.write(addr, &[command, value])
    }
    fn read_word_data(&self, addr: u8, command: u8) -> Result<u16> {
        let b = self.read(addr, command, 2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn write_word_data(&self, addr: u8, command: u8, value: u16) -> Result<()> {
        let [lo, hi] = value.to_le_bytes();
        self.write(addr, &[command, lo, hi])
    }
    fn read_block_data(&self, addr: u8, command: u8) -> Result<Vec<u8>> {
        let b = self.read(addr, command, BLOCK_MAX + 1)?;
        let len = (b[0] as usize).min(BLOCK_MAX);
        Ok(b[1..1 + len].to_vec())
    }
    fn write_block_data(&self, addr: u8, command: u8, data: &[u8]) -> Result<()> {
        let data = &data[..data.len().min(BLOCK_MAX)];
        let mut buf = vec![command, data.len() as u8];
        buf.extend_from_slice(data);
        self.write(addr, &buf)
    }
    fn i2c_read_block_data(&self, addr: u8, command: u8, len: u8) -> Result<Vec<u8>> {
        self.read(addr, command, (len as usize).min(BLOCK_MAX))
    }
    fn i2c_write_block_data(&self, addr: u8, command: u8, data: &[u8]) -> Result<()> {
        let mut buf = vec![command];
        buf.extend_from_slice(&data[..data.len().min(BLOCK_MAX)]);
        self.write(addr, &buf)
    }
}

type Malloc = extern "system" fn(c_int) -> *mut c_void;
type MainControlCreate = unsafe extern "C" fn(Malloc, c_int, *mut *mut c_void) -> c_int;
type MainControlDestroy = unsafe extern "C" fn(*mut c_void) -> c_int;
type AdapterInfoX4Get =
    unsafe extern "C" fn(*mut c_void, c_int, *mut c_int, *mut *mut AdapterInfoX2) -> c_int;
type WriteAndReadI2c = unsafe extern "C" fn(*mut c_void, c_int, *mut AdlI2c) -> c_int;

extern "system" fn allocate(size: c_int) -> *mut c_void {
    unsafe { HeapAlloc(GetProcessHeap(), 0, size.max(0) as usize) }
}

struct Adl {
    context: *mut c_void,
    destroy: MainControlDestroy,
    info: AdapterInfoX4Get,
    i2c: WriteAndReadI2c,
    calls: Mutex<()>,
}

// The context is only used while `calls` is held.
unsafe impl Send for Adl {}
unsafe impl Sync for Adl {}

impl Drop for Adl {
    fn drop(&mut self) {
        unsafe { (self.destroy)(self.context) };
    }
}

impl Adl {
    fn load() -> Option<Adl> {
        let name: Vec<u16> = "atiadlxx.dll\0".encode_utf16().collect();
        let module =
            unsafe { LoadLibraryExW(name.as_ptr(), null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32) };
        if module.is_null() {
            return None;
        }
        unsafe {
            let create: MainControlCreate = symbol(module, c"ADL2_Main_Control_Create")?;
            let destroy = symbol(module, c"ADL2_Main_Control_Destroy")?;
            symbol::<MainControlDestroy>(module, c"ADL2_Adapter_NumberOfAdapters_Get")?;
            symbol::<MainControlDestroy>(module, c"ADL2_Adapter_AdapterInfoX2_Get")?;
            let info = symbol(module, c"ADL2_Adapter_AdapterInfoX4_Get")?;
            let i2c = symbol(module, c"ADL2_Display_WriteAndReadI2C")?;
            let mut context = null_mut();
            if create(allocate, 1, &mut context) != ADL_OK {
                return None;
            }
            Some(Adl {
                context,
                destroy,
                info,
                i2c,
                calls: Mutex::new(()),
            })
        }
    }

    /// The adapters ADL lists for `index`, -1 for all of them.
    fn adapters(&self, index: c_int) -> Vec<(c_int, c_int, String)> {
        let _guard = self.calls.lock().unwrap_or_else(|e| e.into_inner());
        let mut count = 0;
        let mut list = null_mut();
        if unsafe { (self.info)(self.context, index, &mut count, &mut list) } != ADL_OK
            || list.is_null()
        {
            return Vec::new();
        }
        let out = (0..count.max(0) as usize)
            .map(|i| unsafe {
                let a = &*list.add(i);
                let pnp = CStr::from_ptr(a.pnp_string.as_ptr());
                (
                    a.adapter_index,
                    a.bus_number,
                    pnp.to_string_lossy().into_owned(),
                )
            })
            .collect();
        unsafe { HeapFree(GetProcessHeap(), 0, list.cast()) };
        out
    }
}

/// `T` must be the function pointer type of the export `name`.
unsafe fn symbol<T>(module: HMODULE, name: &CStr) -> Option<T> {
    const { assert!(size_of::<T>() == size_of::<usize>()) };
    let address = unsafe { GetProcAddress(module, name.as_ptr().cast()) }?;
    Some(unsafe { std::mem::transmute_copy(&address) })
}

/// One bus per AMD card, as i2c_smbus_amdadl_detect registers them. Empty without the driver.
pub fn buses() -> Vec<SharedBus> {
    let Some(adl) = Adl::load() else {
        return Vec::new();
    };
    let adl = Arc::new(adl);
    let mut buses = Vec::new();
    let mut last_bus_number = -1;
    for (index, bus_number, _) in adl.adapters(-1) {
        if bus_number == last_bus_number {
            continue;
        }
        last_bus_number = bus_number;
        let ids = adl
            .adapters(index)
            .first()
            .and_then(|(_, _, pnp)| parse_pnp(pnp));
        let Some([vendor, device, sub_vendor, sub_device]) = ids else {
            continue;
        };
        if vendor != AMD_GPU_VEN {
            continue;
        }
        let shared = adl.clone();
        let call: Call = Box::new(move |req| {
            let _guard = shared.calls.lock().unwrap_or_else(|e| e.into_inner());
            unsafe { (shared.i2c)(shared.context, index, req) }
        });
        let bus: Box<dyn SmBus> = Box::new(Bus {
            info: BusInfo {
                name: BUS_NAME.into(),
                pci_vendor: vendor,
                pci_device: device,
                pci_subsystem_vendor: sub_vendor,
                pci_subsystem_device: sub_device,
            },
            call,
        });
        buses.push(Arc::new(Mutex::new(bus)));
    }
    buses
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (size, line, address, offset, action, speed, data size, bytes written)
    type Seen = (c_int, c_int, c_int, c_int, c_int, c_int, c_int, Vec<u8>);

    fn bus(answer: Vec<u8>, ret: c_int) -> (Bus, Arc<Mutex<Vec<Seen>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let call: Call = Box::new(move |r| {
            let data = unsafe {
                std::slice::from_raw_parts_mut(r.data.cast::<u8>(), r.data_size as usize)
            };
            let sent = if r.action == ACTION_WRITE {
                data.to_vec()
            } else {
                Vec::new()
            };
            if r.action == ACTION_READ {
                for (d, a) in data.iter_mut().zip(&answer) {
                    *d = *a;
                }
            }
            log.lock().unwrap().push((
                r.size,
                r.line,
                r.address,
                r.offset,
                r.action,
                r.speed,
                r.data_size,
                sent,
            ));
            ret
        });
        let info = BusInfo {
            name: BUS_NAME.into(),
            ..Default::default()
        };
        (Bus { info, call }, seen)
    }

    #[test]
    fn adli2c_layout() {
        assert_eq!(size_of::<AdlI2c>(), 40);
        assert_eq!(size_of::<AdapterInfoX2>(), 4 * 11 + 256 * 6);
    }

    // i2c_smbus_amdadl::i2c_smbus_xfer, I2C_SMBUS_BYTE_DATA read
    #[test]
    fn read_byte_data_request() {
        let (b, seen) = bus(vec![0x15], ADL_OK);
        assert_eq!(b.read_byte_data(0x29, 0x20), Ok(0x15));
        assert_eq!(
            seen.lock().unwrap()[0],
            (40, 1, 0x52, 0x20, 1, 100, 1, vec![])
        );
    }

    // i2c_smbus_amdadl::i2c_smbus_xfer, I2C_SMBUS_BYTE_DATA write
    #[test]
    fn write_byte_data_request() {
        let (b, seen) = bus(vec![], ADL_OK);
        b.write_byte_data(0x2A, 0x07, 0x01).unwrap();
        assert_eq!(
            seen.lock().unwrap()[0],
            (40, 1, 0x54, 0, 2, 100, 2, vec![0x07, 0x01])
        );
    }

    // i2c_smbus_amdadl::i2c_smbus_xfer, I2C_SMBUS_WORD_DATA read and write
    #[test]
    fn word_requests() {
        let (b, seen) = bus(vec![0x34, 0x12], ADL_OK);
        assert_eq!(b.read_word_data(0x29, 0x10), Ok(0x1234));
        b.write_word_data(0x29, 0x10, 0xBEEF).unwrap();
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0], (40, 1, 0x52, 0x10, 1, 100, 2, vec![]));
        assert_eq!(seen[1], (40, 1, 0x52, 0, 2, 100, 3, vec![0x10, 0xEF, 0xBE]));
    }

    // i2c_smbus_amdadl::i2c_smbus_xfer, I2C_SMBUS_BLOCK_DATA and I2C_SMBUS_I2C_BLOCK_DATA writes
    #[test]
    fn block_writes() {
        let (b, seen) = bus(vec![], ADL_OK);
        b.write_block_data(0x29, 0x40, &[1, 2, 3]).unwrap();
        b.i2c_write_block_data(0x29, 0x40, &[1, 2, 3]).unwrap();
        b.i2c_write_block_data(0x29, 0x40, &[9; 40]).unwrap();
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].6, 5);
        assert_eq!(seen[0].7, vec![0x40, 3, 1, 2, 3]);
        assert_eq!(seen[1].7, vec![0x40, 1, 2, 3]);
        assert_eq!(seen[2].6, 33);
    }

    // i2c_smbus_amdadl::i2c_smbus_xfer, I2C_SMBUS_I2C_BLOCK_DATA read
    #[test]
    fn i2c_block_read() {
        let (b, seen) = bus(vec![1, 2, 3, 4], ADL_OK);
        assert_eq!(b.i2c_read_block_data(0x29, 0x81, 4), Ok(vec![1, 2, 3, 4]));
        assert_eq!(
            seen.lock().unwrap()[0],
            (40, 1, 0x52, 0x81, 1, 100, 4, vec![])
        );
    }

    // i2c_smbus_amdadl::i2c_smbus_xfer, I2C_SMBUS_BYTE read and I2C_SMBUS_QUICK
    #[test]
    fn byte_and_quick() {
        let (b, seen) = bus(vec![0xAB], ADL_OK);
        assert_eq!(b.read_byte(0x47), Ok(0xAB));
        b.write_byte(0x47, 0xAB).unwrap();
        assert!(b.write_quick(0x47).is_err());
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0], (40, 1, 0x8E, 0, 1, 100, 1, vec![]));
        assert_eq!(seen[1], (40, 1, 0x8E, 0, 2, 100, 1, vec![0xAB]));
    }

    // i2c_smbus_interface::i2c_smbus_read_byte_data: a nonzero status is a failed read
    #[test]
    fn errors() {
        let (b, _) = bus(vec![0], -1);
        assert!(b.read_byte_data(0x29, 0x20).is_err());
        assert!(b.write_byte_data(0x29, 0x20, 0).is_err());
        let (b, _) = bus(vec![0], 1);
        assert!(b.read_byte_data(0x29, 0x20).is_err());
        assert!(b.write_byte_data(0x29, 0x20, 0).is_ok());
    }

    // i2c_smbus_amdadl::i2c_smbus_amdadl, PNP string parsing
    #[test]
    fn pnp_ids() {
        assert_eq!(
            parse_pnp(r"PCI\VEN_1002&DEV_731F&SUBSYS_E4111DA2&REV_C1"),
            Some([0x1002, 0x731F, 0x1DA2, 0xE411])
        );
        assert_eq!(parse_pnp(r"PCI\VEN_1002&DEV_731F"), None);
    }
}
