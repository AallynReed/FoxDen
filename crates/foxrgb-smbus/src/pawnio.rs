//! The Intel chipset SMBus through the PawnIO kernel driver, spoken to directly.
//! Interface from PawnIOLib (LGPL-2.1, namazso); bus calls from OpenRGB's i2c_smbus_pawnio.cpp.

use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{
    CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Services::{
    CloseServiceHandle, OpenSCManagerW, OpenServiceW, SC_MANAGER_CONNECT, SERVICE_START,
    StartServiceW,
};
use windows_sys::Win32::System::Threading::{
    CreateMutexW, INFINITE, ReleaseMutex, WaitForSingleObject,
};

const DEVICE_TYPE: u32 = 41394;
const fn ctl_code(function: u32) -> u32 {
    (DEVICE_TYPE << 16) | (function << 2)
}
const IOCTL_LOAD_BINARY: u32 = ctl_code(0x821);
const IOCTL_EXECUTE_FN: u32 = ctl_code(0x841);
const FN_NAME_LENGTH: usize = 32;

/// The Intel SMBus module from PawnIO.Modules (LGPL-2.1), signed for the driver.
const SMBUS_I801: &[u8] = include_bytes!("../pawnio/SmbusI801.bin");

/// Shared with other SMBus software so two programs never interleave transfers.
const BUS_MUTEX: &str = "Global\\Access_SMBUS.HTP.Method";
const SLEEP_ALWAYS: u64 = 2;

const READ: u64 = 1;
const WRITE: u64 = 0;
const QUICK: u64 = 0;
const BYTE_DATA: u64 = 2;
const WORD_DATA: u64 = 3;
const BLOCK_DATA: u64 = 5;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

/// Starts the PawnIO driver if it is installed but not running. Needs administrator rights.
fn start_driver() {
    unsafe {
        let manager = OpenSCManagerW(null_mut(), null_mut(), SC_MANAGER_CONNECT);
        if manager.is_null() {
            return;
        }
        let service = OpenServiceW(manager, wide("PawnIO").as_ptr(), SERVICE_START);
        if !service.is_null() {
            StartServiceW(service, 0, null_mut());
            CloseServiceHandle(service);
        }
        CloseServiceHandle(manager);
    }
}

fn open_device() -> Result<HANDLE, String> {
    let mut errors = Vec::new();
    for path in [r"\\?\GLOBALROOT\Device\PawnIO", r"\\.\PawnIO"] {
        match open_path(path) {
            Ok(handle) => return Ok(handle),
            Err(e) => errors.push(format!("{path}: {e}")),
        }
    }
    Err(errors.join("; "))
}

fn open_path(path: &str) -> Result<HANDLE, String> {
    let path = wide(path);
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            null_mut(),
            OPEN_EXISTING,
            0,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(handle)
}

pub struct Bus {
    handle: HANDLE,
    mutex: HANDLE,
}

unsafe impl Send for Bus {}

impl Bus {
    /// Opens the Intel SMBus. Fails without PawnIO, without administrator rights or on another chipset.
    pub fn open() -> Result<Bus, String> {
        let handle = match open_device() {
            Ok(handle) => handle,
            Err(_) => {
                start_driver();
                open_device().map_err(|e| format!("PawnIO is not available: {e}"))?
            }
        };
        let mutex = unsafe { CreateMutexW(null_mut(), 0, wide(BUS_MUTEX).as_ptr()) };
        let bus = Bus { handle, mutex };
        bus.ioctl(IOCTL_LOAD_BINARY, SMBUS_I801, &mut [])
            .map_err(|e| format!("no Intel SMBus here ({e})"))?;
        let _ = bus.execute("ioctl_set_sleep_mode", &[SLEEP_ALWAYS], 0);
        Ok(bus)
    }

    fn ioctl(&self, code: u32, input: &[u8], output: &mut [u8]) -> Result<usize, String> {
        let mut written = 0u32;
        let ok = unsafe {
            DeviceIoControl(
                self.handle,
                code,
                input.as_ptr().cast(),
                input.len() as u32,
                output.as_mut_ptr().cast(),
                output.len() as u32,
                &mut written,
                null_mut(),
            )
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(written as usize)
    }

    fn execute(&self, name: &str, input: &[u64], outputs: usize) -> Result<Vec<u64>, String> {
        let mut request = vec![0u8; FN_NAME_LENGTH];
        request[..name.len()].copy_from_slice(name.as_bytes());
        for value in input {
            request.extend_from_slice(&value.to_le_bytes());
        }
        let mut out = vec![0u8; outputs * 8];
        let n = self.ioctl(IOCTL_EXECUTE_FN, &request, &mut out)?;
        Ok(out[..n]
            .chunks_exact(8)
            .map(|b| u64::from_le_bytes(b.try_into().unwrap()))
            .collect())
    }

    /// One SMBus transfer. `data` is the 34-byte i2c_smbus_data union.
    fn xfer(
        &self,
        addr: u8,
        read: bool,
        command: u8,
        size: u64,
        data: &mut [u8; 40],
    ) -> Result<(), String> {
        let mut input = vec![
            addr as u64,
            if read { READ } else { WRITE },
            command as u64,
            size,
        ];
        input.extend(
            data.chunks_exact(8)
                .map(|b| u64::from_le_bytes(b.try_into().unwrap())),
        );
        if !self.mutex.is_null() {
            unsafe { WaitForSingleObject(self.mutex, INFINITE) };
        }
        let result = self.execute("ioctl_smbus_xfer", &input, 5);
        if !self.mutex.is_null() {
            unsafe { ReleaseMutex(self.mutex) };
        }
        let out = result?;
        for (i, value) in out.iter().enumerate().take(5) {
            data[i * 8..i * 8 + 8].copy_from_slice(&value.to_le_bytes());
        }
        Ok(())
    }

    pub fn write_quick(&self, addr: u8) -> Result<(), String> {
        self.xfer(addr, false, 0, QUICK, &mut [0; 40])
    }

    pub fn read_byte(&self, addr: u8, command: u8) -> Result<u8, String> {
        let mut data = [0u8; 40];
        self.xfer(addr, true, command, BYTE_DATA, &mut data)?;
        Ok(data[0])
    }

    pub fn write_byte(&self, addr: u8, command: u8, value: u8) -> Result<(), String> {
        let mut data = [0u8; 40];
        data[0] = value;
        self.xfer(addr, false, command, BYTE_DATA, &mut data)
    }

    pub fn write_word(&self, addr: u8, command: u8, value: u16) -> Result<(), String> {
        let mut data = [0u8; 40];
        data[..2].copy_from_slice(&value.to_le_bytes());
        self.xfer(addr, false, command, WORD_DATA, &mut data)
    }

    pub fn write_block(&self, addr: u8, command: u8, bytes: &[u8]) -> Result<(), String> {
        let mut data = [0u8; 40];
        let n = bytes.len().min(32);
        data[0] = n as u8;
        data[1..1 + n].copy_from_slice(&bytes[..n]);
        self.xfer(addr, false, command, BLOCK_DATA, &mut data)
    }
}

impl Drop for Bus {
    fn drop(&mut self) {
        unsafe {
            if !self.mutex.is_null() {
                CloseHandle(self.mutex);
            }
            CloseHandle(self.handle);
        }
    }
}
