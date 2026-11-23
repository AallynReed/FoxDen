// SPDX-License-Identifier: GPL-2.0-or-later

use std::ffi::{CStr, c_void};
use std::mem::size_of;
use std::ptr::{NonNull, null_mut};
use std::sync::{Arc, Mutex};

use foxden_smbus::i2c::{BusInfo, Result, SharedBus, SmBus};
use windows_sys::Win32::System::LibraryLoader::{
    GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
};

const BLOCK_MAX: usize = 32;
const VERSION: u32 = (3 << 16) | size_of::<NvI2cInfoV3>() as u32;

const ID_INITIALIZE: u32 = 0x0150E828;
const ID_ENUM_PHYSICAL_GPUS: u32 = 0xE5AC921F;
const ID_GET_PCI_IDENTIFIERS: u32 = 0x2DDFB66E;
const ID_I2C_WRITE_EX: u32 = 0x283AC65A;
const ID_I2C_READ_EX: u32 = 0x4D7B0709;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct NvI2cInfoV3 {
    version: u32,
    display_mask: u32,
    is_ddc_port: u8,
    i2c_dev_address: u8,
    i2c_reg_address: *mut u8,
    reg_addr_size: u32,
    data: *mut u8,
    size: u32,
    i2c_speed: u32,
    i2c_speed_khz: u32,
    port_id: u8,
    is_port_id_set: u32,
}

/// The NV_I2C_INFO_V3 request for every transfer: port 1, default speed, 8-bit address.
fn request(addr: u8, reg: Option<&mut u8>, data: &mut [u8]) -> NvI2cInfoV3 {
    let (reg, reg_size) = match reg {
        Some(r) => (r as *mut u8, 1),
        None => (null_mut(), 0),
    };
    NvI2cInfoV3 {
        version: VERSION,
        display_mask: 0,
        is_ddc_port: 0,
        i2c_dev_address: addr << 1,
        i2c_reg_address: reg,
        reg_addr_size: reg_size,
        data: data.as_mut_ptr(),
        size: data.len() as u32,
        i2c_speed: 0xFFFF,
        i2c_speed_khz: 0,
        port_id: 1,
        is_port_id_set: 1,
    }
}

/// Vendor, device, subsystem vendor and subsystem device from NvAPI_GPU_GetPCIIdentifiers.
fn pci_ids(device_id: u32, sub_system_id: u32) -> [u16; 4] {
    [
        device_id as u16,
        (device_id >> 16) as u16,
        sub_system_id as u16,
        (sub_system_id >> 16) as u16,
    ]
}

/// `write` picks NvAPI_I2CWriteEx over NvAPI_I2CReadEx.
type Call = Box<dyn Fn(bool, &mut NvI2cInfoV3) -> i32 + Send>;

pub struct Bus {
    info: BusInfo,
    call: Call,
}

impl Bus {
    /// The full buffer and the size the driver reports back.
    fn read(&self, addr: u8, reg: Option<u8>, len: usize) -> Result<(Vec<u8>, usize)> {
        let mut buf = vec![0; len];
        let mut reg = reg;
        let mut req = request(addr, reg.as_mut(), &mut buf);
        match (self.call)(false, &mut req) {
            0 => {
                let size = (req.size as usize).min(len);
                Ok((buf, size))
            }
            e => Err(format!("NvAPI I2C read at {addr:#04x} failed ({e})")),
        }
    }

    fn write(&self, addr: u8, reg: Option<u8>, data: &[u8]) -> Result<()> {
        let mut buf = data.to_vec();
        let mut reg = reg;
        let mut req = request(addr, reg.as_mut(), &mut buf);
        match (self.call)(true, &mut req) {
            e if e < 0 => Err(format!("NvAPI I2C write at {addr:#04x} failed ({e})")),
            _ => Ok(()),
        }
    }

    /// Byte and word reads take the buffer whatever size comes back.
    fn read_n<const N: usize>(&self, addr: u8, reg: Option<u8>) -> Result<[u8; N]> {
        let (b, _) = self.read(addr, reg, N)?;
        Ok(b.try_into().unwrap())
    }
}

impl SmBus for Bus {
    fn info(&self) -> &BusInfo {
        &self.info
    }
    fn write_quick(&self, _addr: u8) -> Result<()> {
        Err("NvAPI has no quick command".into())
    }
    fn read_byte(&self, addr: u8) -> Result<u8> {
        Ok(self.read_n::<1>(addr, None)?[0])
    }
    fn write_byte(&self, addr: u8, value: u8) -> Result<()> {
        self.write(addr, None, &[value])
    }
    fn read_byte_data(&self, addr: u8, command: u8) -> Result<u8> {
        Ok(self.read_n::<1>(addr, Some(command))?[0])
    }
    fn write_byte_data(&self, addr: u8, command: u8, value: u8) -> Result<()> {
        self.write(addr, Some(command), &[value])
    }
    fn read_word_data(&self, addr: u8, command: u8) -> Result<u16> {
        Ok(u16::from_le_bytes(self.read_n::<2>(addr, Some(command))?))
    }
    fn write_word_data(&self, addr: u8, command: u8, value: u16) -> Result<()> {
        self.write(addr, Some(command), &value.to_le_bytes())
    }
    fn read_block_data(&self, addr: u8, command: u8) -> Result<Vec<u8>> {
        let (mut b, size) = self.read(addr, Some(command), BLOCK_MAX + 1)?;
        b.truncate(size);
        let len = b.first().map_or(0, |&n| (n as usize).min(b.len() - 1));
        Ok(b[1..1 + len].to_vec())
    }
    fn write_block_data(&self, addr: u8, command: u8, data: &[u8]) -> Result<()> {
        let data = &data[..data.len().min(BLOCK_MAX)];
        let mut buf = vec![data.len() as u8];
        buf.extend_from_slice(data);
        self.write(addr, Some(command), &buf)
    }
    fn i2c_read_block_data(&self, addr: u8, command: u8, len: u8) -> Result<Vec<u8>> {
        let (mut b, size) = self.read(addr, Some(command), (len as usize).min(BLOCK_MAX))?;
        b.truncate(size);
        Ok(b)
    }
    fn i2c_write_block_data(&self, addr: u8, command: u8, data: &[u8]) -> Result<()> {
        self.write(addr, Some(command), &data[..data.len().min(BLOCK_MAX)])
    }
    fn i2c_read(&self, addr: u8, len: usize) -> Result<Vec<u8>> {
        let (mut b, size) = self.read(addr, None, len)?;
        b.truncate(size);
        Ok(b)
    }
    fn i2c_write(&self, addr: u8, data: &[u8]) -> Result<()> {
        self.write(addr, None, data)
    }
}

type Handle = *mut i32;
pub(crate) type QueryInterface = unsafe extern "C" fn(u32) -> *mut c_void;
type Initialize = unsafe extern "C" fn() -> i32;
type EnumPhysicalGpus = unsafe extern "C" fn(*mut Handle, *mut i32) -> i32;
type GetPciIdentifiers =
    unsafe extern "C" fn(Handle, *mut u32, *mut u32, *mut u32, *mut u32) -> i32;
type I2cEx = unsafe extern "C" fn(Handle, *mut NvI2cInfoV3, *mut u32) -> i32;

pub(crate) struct Gpu(pub(crate) Handle);

// NvAPI handles are opaque driver tokens, usable from any thread.
unsafe impl Send for Gpu {}
unsafe impl Sync for Gpu {}

/// `T` must be the function pointer type of interface `id`.
pub(crate) unsafe fn query<T>(query_interface: QueryInterface, id: u32) -> Option<T> {
    const { assert!(size_of::<T>() == size_of::<usize>()) };
    let address = NonNull::new(unsafe { query_interface(id) })?;
    Some(unsafe { std::mem::transmute_copy(&address) })
}

/// A physical GPU and the PCI ids it reports.
pub(crate) type Found = (Arc<Gpu>, Option<[u16; 4]>);

/// nvapi64.dll initialised, and every physical GPU with the PCI ids it reports. None without the driver.
pub(crate) fn load() -> Option<(QueryInterface, Vec<Found>)> {
    let name: Vec<u16> = "nvapi64.dll\0".encode_utf16().collect();
    let module = unsafe { LoadLibraryExW(name.as_ptr(), null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32) };
    if module.is_null() {
        return None;
    }
    let entry: &CStr = c"nvapi_QueryInterface";
    let address = unsafe { GetProcAddress(module, entry.as_ptr().cast()) }?;
    let query_interface: QueryInterface = unsafe { std::mem::transmute(address) };
    unsafe {
        let initialize = query::<Initialize>(query_interface, ID_INITIALIZE)?;
        let enumerate = query::<EnumPhysicalGpus>(query_interface, ID_ENUM_PHYSICAL_GPUS)?;
        let identify = query::<GetPciIdentifiers>(query_interface, ID_GET_PCI_IDENTIFIERS)?;
        initialize();
        let mut handles: [Handle; 64] = [null_mut(); 64];
        let mut count = 0;
        if enumerate(handles.as_mut_ptr(), &mut count) != 0 {
            return None;
        }
        let gpus = handles
            .iter()
            .take(count.clamp(0, 64) as usize)
            .map(|&handle| {
                let (mut device, mut sub, mut revision, mut ext) = (0, 0, 0, 0);
                let ids = (identify(handle, &mut device, &mut sub, &mut revision, &mut ext) == 0)
                    .then(|| pci_ids(device, sub));
                (Arc::new(Gpu(handle)), ids)
            })
            .collect();
        Some((query_interface, gpus))
    }
}

/// One bus per NVIDIA GPU. Empty without the driver.
pub fn buses() -> Vec<SharedBus> {
    let Some((query_interface, gpus)) = load() else {
        return Vec::new();
    };
    let write = unsafe { query::<I2cEx>(query_interface, ID_I2C_WRITE_EX) };
    let read = unsafe { query::<I2cEx>(query_interface, ID_I2C_READ_EX) };
    let (Some(write), Some(read)) = (write, read) else {
        return Vec::new();
    };
    let mut buses = Vec::new();
    for (index, (gpu, ids)) in gpus.into_iter().enumerate() {
        let [vendor, device, sub_vendor, sub_device] = ids.unwrap_or([0xFFFF; 4]);
        let call: Call = Box::new(move |is_write, req| {
            let mut unknown = 0;
            unsafe {
                if is_write {
                    write(gpu.0, req, &mut unknown)
                } else {
                    read(gpu.0, req, &mut unknown)
                }
            }
        });
        let bus: Box<dyn SmBus> = Box::new(Bus {
            info: BusInfo {
                name: format!("Nvidia NvAPI I2C on GPU {index}"),
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

    /// (write, version, ddc, address, register, register size, data, speed, speed khz, port, port set)
    type Seen = (
        bool,
        u32,
        u8,
        u8,
        Option<u8>,
        u32,
        Vec<u8>,
        u32,
        u32,
        u8,
        u32,
    );

    fn bus(answer: Vec<u8>, ret: i32) -> (Bus, Arc<Mutex<Vec<Seen>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let call: Call = Box::new(move |is_write, r| {
            let data = unsafe { std::slice::from_raw_parts_mut(r.data, r.size as usize) };
            let reg = (r.reg_addr_size == 1).then(|| unsafe { *r.i2c_reg_address });
            let sent = if is_write {
                data.to_vec()
            } else {
                vec![0; data.len()]
            };
            if !is_write {
                for (d, a) in data.iter_mut().zip(&answer) {
                    *d = *a;
                }
            }
            log.lock().unwrap().push((
                is_write,
                r.version,
                r.is_ddc_port,
                r.i2c_dev_address,
                reg,
                r.reg_addr_size,
                sent,
                r.i2c_speed,
                r.i2c_speed_khz,
                r.port_id,
                r.is_port_id_set,
            ));
            ret
        });
        let info = BusInfo::default();
        (Bus { info, call }, seen)
    }

    fn seen(write: bool, addr: u8, reg: Option<u8>, data: Vec<u8>) -> Seen {
        let size = reg.is_some() as u32;
        (
            write,
            0x0003_0040,
            0,
            addr,
            reg,
            size,
            data,
            0xFFFF,
            0,
            1,
            1,
        )
    }

    // Byte read ignores size, block read uses it
    #[test]
    fn returned_size() {
        let call: Call = Box::new(|_, r| {
            unsafe { *r.data = 0x42 };
            r.size = 0;
            0
        });
        let b = Bus {
            info: BusInfo::default(),
            call,
        };
        assert_eq!(b.read_byte_data(0x29, 0x20), Ok(0x42));
        assert_eq!(b.i2c_read_block_data(0x29, 0x20, 4), Ok(vec![]));
    }

    // NV_I2C_INFO_V3::NV_I2C_INFO_V3, NV_STRUCT_VERSION(NV_I2C_INFO_V3, 3)
    #[test]
    fn info_layout() {
        assert_eq!(size_of::<NvI2cInfoV3>(), 64);
        assert_eq!(VERSION, 0x0003_0040);
    }

    // I2C_SMBUS_BYTE_DATA read and write
    #[test]
    fn byte_data() {
        let (b, log) = bus(vec![0x89], 0);
        assert_eq!(b.read_byte_data(0x2A, 0x21), Ok(0x89));
        b.write_byte_data(0x2A, 0x04, 0xFF).unwrap();
        let log = log.lock().unwrap();
        assert_eq!(log[0], seen(false, 0x54, Some(0x21), vec![0]));
        assert_eq!(log[1], seen(true, 0x54, Some(0x04), vec![0xFF]));
    }

    // I2C_SMBUS_BYTE read and write
    #[test]
    fn byte() {
        let (b, log) = bus(vec![0xAB], 0);
        assert_eq!(b.read_byte(0x47), Ok(0xAB));
        b.write_byte(0x47, 0x40).unwrap();
        let log = log.lock().unwrap();
        assert_eq!(log[0], seen(false, 0x8E, None, vec![0]));
        assert_eq!(log[1], seen(true, 0x8E, None, vec![0x40]));
    }

    // I2C_SMBUS_WORD_DATA read and write
    #[test]
    fn word() {
        let (b, log) = bus(vec![0x34, 0x12], 0);
        assert_eq!(b.read_word_data(0x29, 0x10), Ok(0x1234));
        b.write_word_data(0x29, 0x10, 0xBEEF).unwrap();
        let log = log.lock().unwrap();
        assert_eq!(log[0], seen(false, 0x52, Some(0x10), vec![0, 0]));
        assert_eq!(log[1], seen(true, 0x52, Some(0x10), vec![0xEF, 0xBE]));
    }

    // I2C_SMBUS_BLOCK_DATA and I2C_SMBUS_I2C_BLOCK_DATA
    #[test]
    fn blocks() {
        let (b, log) = bus(vec![1, 2, 3], 0);
        b.write_block_data(0x29, 0x40, &[7, 8]).unwrap();
        b.i2c_write_block_data(0x29, 0x40, &[7, 8]).unwrap();
        b.i2c_write_block_data(0x29, 0x40, &[9; 40]).unwrap();
        assert_eq!(b.i2c_read_block_data(0x29, 0x81, 3), Ok(vec![1, 2, 3]));
        let log = log.lock().unwrap();
        assert_eq!(log[0], seen(true, 0x52, Some(0x40), vec![2, 7, 8]));
        assert_eq!(log[1], seen(true, 0x52, Some(0x40), vec![7, 8]));
        assert_eq!(log[2].6.len(), 32);
        assert_eq!(log[3], seen(false, 0x52, Some(0x81), vec![0, 0, 0]));
    }

    // Raw I2C transfer: no register, no block cap, the read keeps the size returned
    #[test]
    fn raw_transfer() {
        let (b, log) = bus(vec![0xAB, 0x11], 0);
        b.i2c_write(0x71, &[0x10; 64]).unwrap();
        assert_eq!(b.i2c_read(0x71, 4), Ok(vec![0xAB, 0x11, 0, 0]));
        let log = log.lock().unwrap();
        assert_eq!(log[0], seen(true, 0xE2, None, vec![0x10; 64]));
        assert_eq!(log[1], seen(false, 0xE2, None, vec![0; 4]));
    }

    // I2C_SMBUS_QUICK and failed reads
    #[test]
    fn errors() {
        let (b, log) = bus(vec![], -5);
        assert!(b.write_quick(0x29).is_err());
        assert!(log.lock().unwrap().is_empty());
        assert!(b.read_byte_data(0x29, 0x20).is_err());
        assert!(b.write_byte(0x29, 0).is_err());
    }

    // Bus detection: the NvAPI_GPU_GetPCIIdentifiers split
    #[test]
    fn ids() {
        assert_eq!(
            pci_ids(0x2484_10DE, 0x404F_1458),
            [0x10DE, 0x2484, 0x1458, 0x404F]
        );
    }
}
