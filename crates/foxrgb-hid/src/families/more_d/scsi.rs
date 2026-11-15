//! Ported from OpenRGB scsiapi/scsiapi_windows.c (GPL-2.0-or-later).

use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{
    CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, QueryDosDeviceW,
};
use windows_sys::Win32::Storage::IscsiDisc::{
    IOCTL_SCSI_PASS_THROUGH_DIRECT, SCSI_IOCTL_DATA_OUT, SCSI_PASS_THROUGH_DIRECT,
};
use windows_sys::Win32::System::IO::DeviceIoControl;

use super::common::Io;
use super::serial::wide;

const SENSE: usize = 32;
pub const CDB: usize = 12;

/// A SCSI disk as `scsi_enumerate` lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Disk {
    pub path: String,
    pub vendor: String,
    pub product: String,
}

/// `SCSI#Disk&Ven_x&Prod_y#...` to the disk it names.
pub fn parse(entry: &str) -> Option<Disk> {
    let rest = entry.strip_prefix("SCSI#Disk&Ven_")?;
    let (vendor, rest) = rest.split_once('&')?;
    let rest = rest.strip_prefix("Prod_")?;
    let product = rest.split('#').next()?;
    Some(Disk {
        path: format!(r"\\?\{entry}"),
        vendor: vendor.replace('_', " "),
        product: product.replace('_', " "),
    })
}

pub fn enumerate() -> Vec<Disk> {
    let mut buf = vec![0u16; 128 * 1024];
    // SAFETY: `buf` holds `buf.len()` characters.
    let n = unsafe { QueryDosDeviceW(null(), buf.as_mut_ptr(), buf.len() as u32) } as usize;
    String::from_utf16_lossy(&buf[..n.min(buf.len())])
        .split('\0')
        .filter_map(parse)
        .collect()
}

/// An open disk that takes pass-through commands.
pub struct Scsi(HANDLE);

// SAFETY: the handle is only used through `&mut self`.
unsafe impl Send for Scsi {}

impl Scsi {
    pub fn open(path: &str) -> Result<Scsi, String> {
        let path = wide(path);
        // SAFETY: `path` is NUL-terminated.
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                null(),
                OPEN_EXISTING,
                0,
                null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err("disk would not open".into());
        }
        Ok(Scsi(handle))
    }
}

impl Drop for Scsi {
    fn drop(&mut self) {
        // SAFETY: the handle is open and closed once.
        unsafe { CloseHandle(self.0) };
    }
}

#[repr(C)]
struct Command {
    head: SCSI_PASS_THROUGH_DIRECT,
    sense: [u8; SENSE],
}

impl Io for Scsi {
    /// `scsi_write`: `data` is the 12-byte CDB followed by the data sent out.
    fn write(&mut self, data: &[u8]) -> Result<usize, String> {
        let (cdb, payload) = data.split_at(CDB.min(data.len()));
        let mut payload = payload.to_vec();
        // SAFETY: all-zero is a valid value for both plain-data structs.
        let mut cmd: Command = unsafe { std::mem::zeroed() };
        cmd.head.Length = size_of::<SCSI_PASS_THROUGH_DIRECT>() as u16;
        cmd.head.CdbLength = cdb.len() as u8;
        cmd.head.SenseInfoLength = SENSE as u8;
        cmd.head.DataIn = SCSI_IOCTL_DATA_OUT as u8;
        cmd.head.DataTransferLength = payload.len() as u32;
        cmd.head.TimeOutValue = 0x14;
        cmd.head.DataBuffer = payload.as_mut_ptr().cast();
        cmd.head.SenseInfoOffset = size_of::<SCSI_PASS_THROUGH_DIRECT>() as u32;
        cmd.head.Cdb[..cdb.len()].copy_from_slice(cdb);
        let size = size_of::<Command>() as u32;
        let mut returned = 0;
        // SAFETY: `cmd` and `payload` outlive the synchronous call.
        unsafe {
            DeviceIoControl(
                self.0,
                IOCTL_SCSI_PASS_THROUGH_DIRECT,
                (&raw const cmd).cast(),
                size,
                (&raw mut cmd).cast(),
                size,
                &mut returned,
                null_mut(),
            )
        };
        Ok(data.len())
    }
    fn read(&mut self, _buf: &mut [u8], _timeout_ms: i32) -> Result<usize, String> {
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dos_device_names() {
        let d =
            parse("SCSI#Disk&Ven_Seagate&Prod_FireCuda_HDD#5&1a2b&0&000000#{53f56307}").unwrap();
        assert_eq!(
            (d.vendor.as_str(), d.product.as_str()),
            ("Seagate", "FireCuda HDD")
        );
        assert!(d.path.starts_with(r"\\?\SCSI#Disk"));
        assert_eq!(parse("PhysicalDrive0"), None);
    }
}
