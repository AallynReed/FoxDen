//! The PawnIO kernel driver, spoken to directly. Needs administrator rights.
//! Interface from PawnIOLib (LGPL-2.1, namazso). Modules from PawnIO.Modules (LGPL-2.1).

use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{
    CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE, WAIT_ABANDONED,
    WAIT_OBJECT_0,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Services::{
    CloseServiceHandle, OpenSCManagerW, OpenServiceW, SC_MANAGER_CONNECT, SERVICE_START,
    StartServiceW,
};
use windows_sys::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};

pub const SMBUS_I801: &[u8] = include_bytes!("../modules/SmbusI801.bin");
pub const SMBUS_PIIX4: &[u8] = include_bytes!("../modules/SmbusPIIX4.bin");
pub const SMBUS_NCT6793: &[u8] = include_bytes!("../modules/SmbusNCT6793.bin");
pub const LPC_IO: &[u8] = include_bytes!("../modules/LpcIO.bin");
pub const INTEL_MSR: &[u8] = include_bytes!("../modules/IntelMSR.bin");
pub const LEDS_VALVE: &[u8] = include_bytes!("../modules/LedsValve.bin");

const DEVICE_TYPE: u32 = 41394;
const fn ctl_code(function: u32) -> u32 {
    (DEVICE_TYPE << 16) | (function << 2)
}
const IOCTL_LOAD_BINARY: u32 = ctl_code(0x821);
const IOCTL_EXECUTE_FN: u32 = ctl_code(0x841);
const FN_NAME_LENGTH: usize = 32;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

/// Starts the PawnIO driver if it is installed but not running.
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
    let path = wide(r"\\?\GLOBALROOT\Device\PawnIO");
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

/// One loaded PawnIO module.
pub struct Module {
    handle: HANDLE,
}

unsafe impl Send for Module {}
// DeviceIoControl on one handle is safe from several threads.
unsafe impl Sync for Module {}

impl Module {
    pub fn open(blob: &[u8]) -> Result<Module, String> {
        let handle = match open_device() {
            Ok(handle) => handle,
            Err(_) => {
                start_driver();
                open_device().map_err(|e| format!("PawnIO is not available: {e}"))?
            }
        };
        let module = Module { handle };
        module
            .ioctl(IOCTL_LOAD_BINARY, blob, &mut [])
            .map_err(|e| format!("module not loaded ({e})"))?;
        Ok(module)
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

    /// Calls a module function with 64-bit arguments, returning up to `outputs` 64-bit results.
    pub fn execute(&self, name: &str, input: &[u64], outputs: usize) -> Result<Vec<u64>, String> {
        let mut request = vec![0u8; FN_NAME_LENGTH];
        request[..name.len()].copy_from_slice(name.as_bytes());
        for value in input {
            request.extend_from_slice(&value.to_le_bytes());
        }
        let mut out = vec![0u8; outputs * 8];
        let n = self.ioctl(IOCTL_EXECUTE_FN, &request, &mut out)?;
        Ok(out[..n]
            .as_chunks::<8>()
            .0
            .iter()
            .map(|b| u64::from_le_bytes(*b))
            .collect())
    }
}

impl Drop for Module {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.handle) };
    }
}

/// How long another tool may hold a shared lock before the access is given up.
const LOCK_WAIT_MS: u32 = 2000;

/// A named mutex shared with other hardware tools, such as `Global\Access_ISABUS.HTP.Method`.
pub struct SharedLock(HANDLE);

/// Releases the mutex even when the work under it panics.
struct Held(HANDLE);

impl Drop for Held {
    fn drop(&mut self) {
        unsafe { ReleaseMutex(self.0) };
    }
}

unsafe impl Send for SharedLock {}
unsafe impl Sync for SharedLock {}

impl SharedLock {
    pub fn new(name: &str) -> SharedLock {
        SharedLock(unsafe { CreateMutexW(null_mut(), 0, wide(name).as_ptr()) })
    }

    /// Runs `f` holding the lock; an error when another holder keeps it too long.
    pub fn run<T>(&self, f: impl FnOnce() -> T) -> Result<T, String> {
        if self.0.is_null() {
            return Ok(f());
        }
        match unsafe { WaitForSingleObject(self.0, LOCK_WAIT_MS) } {
            WAIT_OBJECT_0 | WAIT_ABANDONED => {}
            _ => return Err("another program holds the hardware lock".into()),
        }
        let _held = Held(self.0);
        Ok(f())
    }
}

impl Drop for SharedLock {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CloseHandle(self.0) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(tag: &str) -> String {
        format!("Local\\foxden-pawnio-test-{}-{tag}", std::process::id())
    }

    #[test]
    fn a_panic_under_the_lock_releases_it() {
        let lock = SharedLock::new(&name("panic"));
        let hit = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = lock.run(|| panic!("on purpose"));
        }));
        assert!(hit.is_err());
        let other = SharedLock::new(&name("panic"));
        let free = std::thread::spawn(move || other.run(|| 7)).join().unwrap();
        assert_eq!(free, Ok(7));
    }

    #[test]
    fn a_lock_held_elsewhere_is_given_up_in_time() {
        let lock = SharedLock::new(&name("held"));
        let (tx, rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        let holder = std::thread::spawn(move || {
            lock.run(|| {
                tx.send(()).unwrap();
                let _ = done_rx.recv();
            })
        });
        rx.recv().unwrap();
        let start = std::time::Instant::now();
        let other = SharedLock::new(&name("held"));
        assert!(other.run(|| ()).is_err());
        assert!(start.elapsed() >= std::time::Duration::from_millis(LOCK_WAIT_MS as u64 - 100));
        done_tx.send(()).unwrap();
        assert_eq!(holder.join().unwrap(), Ok(()));
        assert_eq!(other.run(|| 1), Ok(1));
    }
}
