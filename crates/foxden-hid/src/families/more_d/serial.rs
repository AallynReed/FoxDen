// SPDX-License-Identifier: GPL-2.0-or-later

use std::ptr::{null, null_mut};

use windows_sys::Win32::Devices::Communication::{
    CLRDTR, COMMTIMEOUTS, DCB, EscapeCommFunction, GetCommState, NOPARITY, ONESTOPBIT,
    PURGE_RXABORT, PURGE_RXCLEAR, PURGE_TXABORT, PURGE_TXCLEAR, PurgeComm, SETDTR, SetCommState,
    SetCommTimeouts, SetupComm, TWOSTOPBITS,
};
use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    DICS_FLAG_GLOBAL, DIGCF_ALLCLASSES, DIGCF_PRESENT, DIREG_DEV, SP_DEVINFO_DATA,
    SPDRP_HARDWAREID, SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiGetClassDevsW,
    SetupDiGetDeviceRegistryPropertyW, SetupDiOpenDevRegKey,
};
use windows_sys::Win32::Foundation::{
    CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{CreateFileW, OPEN_EXISTING, ReadFile, WriteFile};
use windows_sys::Win32::System::Registry::{KEY_READ, REG_SZ, RegCloseKey, RegQueryValueExW};

use super::common::Io;

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// An open COM port.
pub struct Serial(HANDLE);

// SAFETY: the handle is only used through `&mut self`.
unsafe impl Send for Serial {}

impl Serial {
    /// 8 data bits, no parity, no flow control, DTR and RTS off.
    pub fn open(port: &str, baud: u32, two_stop_bits: bool) -> Result<Serial, String> {
        let path = wide(&format!(r"\\.\{port}"));
        // SAFETY: `path` is NUL-terminated.
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                null(),
                OPEN_EXISTING,
                0,
                null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(format!("{port} would not open"));
        }
        let port = Serial(handle);
        // SAFETY: `handle` is open and both structs live across the calls.
        unsafe {
            let mut dcb: DCB = std::mem::zeroed();
            dcb.DCBlength = size_of::<DCB>() as u32;
            SetupComm(handle, 1, 128);
            if GetCommState(handle, &mut dcb) == 0 {
                return Err("serial state unreadable".into());
            }
            dcb.BaudRate = baud;
            dcb.Parity = NOPARITY;
            dcb.ByteSize = 8;
            dcb.StopBits = if two_stop_bits {
                TWOSTOPBITS
            } else {
                ONESTOPBIT
            };
            // Clears CTS/DSR flow (bits 2-3), DTR control (4-5), XON/XOFF (8-9), RTS control (12-13) and abort on error (14).
            dcb._bitfield &= !(0b1100 | 0b11_0000 | 0b11_0000_0000 | 0b111 << 12);
            if SetCommState(handle, &dcb) == 0 {
                return Err("serial settings rejected".into());
            }
            let timeouts = COMMTIMEOUTS {
                ReadIntervalTimeout: 50,
                ReadTotalTimeoutMultiplier: 10,
                ReadTotalTimeoutConstant: 50,
                WriteTotalTimeoutMultiplier: 10,
                WriteTotalTimeoutConstant: 50,
            };
            if SetCommTimeouts(handle, &timeouts) == 0 {
                return Err("serial timeouts rejected".into());
            }
        }
        Ok(port)
    }

    pub fn set_dtr(&mut self, on: bool) {
        // SAFETY: the handle is open.
        unsafe { EscapeCommFunction(self.0, if on { SETDTR } else { CLRDTR }) };
    }
}

impl Drop for Serial {
    fn drop(&mut self) {
        // SAFETY: the handle is open and closed once.
        unsafe { CloseHandle(self.0) };
    }
}

impl Io for Serial {
    fn write(&mut self, data: &[u8]) -> Result<usize, String> {
        let mut done = 0;
        // SAFETY: `data` outlives the synchronous call.
        let ok = unsafe {
            WriteFile(
                self.0,
                data.as_ptr(),
                data.len() as u32,
                &mut done,
                null_mut(),
            )
        };
        if ok == 0 {
            return Err("serial write failed".into());
        }
        Ok(done as usize)
    }
    fn read(&mut self, buf: &mut [u8], _timeout_ms: i32) -> Result<usize, String> {
        let mut done = 0;
        // SAFETY: `buf` outlives the synchronous call.
        let ok = unsafe {
            ReadFile(
                self.0,
                buf.as_mut_ptr(),
                buf.len() as u32,
                &mut done,
                null_mut(),
            )
        };
        if ok == 0 {
            return Err("serial read failed".into());
        }
        Ok((done as usize).min(buf.len()))
    }
    fn flush(&mut self, rx: bool, tx: bool) {
        let mut flags = 0;
        if rx {
            flags |= PURGE_RXABORT | PURGE_RXCLEAR;
        }
        if tx {
            flags |= PURGE_TXABORT | PURGE_TXCLEAR;
        }
        // SAFETY: the handle is open.
        unsafe { PurgeComm(self.0, flags) };
    }
}

/// A COM port of a USB device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Port {
    pub name: String,
    pub vid: u16,
    pub pid: u16,
}

/// `USB\VID_xxxx&PID_yyyy...` to its ids.
pub fn ids(hardware_id: &str) -> Option<(u16, u16)> {
    let rest = hardware_id.get(..21)?.strip_prefix("USB\\VID_")?;
    let vid = u16::from_str_radix(rest.get(..4)?, 16).ok()?;
    let pid =
        u16::from_str_radix(rest.strip_prefix(&rest[..4])?.strip_prefix("&PID_")?, 16).ok()?;
    Some((vid, pid))
}

/// Every present USB device with a COM port, as `find_usb_serial_ports` lists them.
pub fn usb_ports() -> Vec<Port> {
    let mut out = Vec::new();
    let usb = wide("USB");
    // SAFETY: every buffer handed to SetupAPI and the registry outlives its call.
    unsafe {
        let set = SetupDiGetClassDevsW(
            null(),
            usb.as_ptr(),
            null_mut(),
            DIGCF_ALLCLASSES | DIGCF_PRESENT,
        );
        if set == INVALID_HANDLE_VALUE as isize {
            return out;
        }
        let mut data: SP_DEVINFO_DATA = std::mem::zeroed();
        data.cbSize = size_of::<SP_DEVINFO_DATA>() as u32;
        let mut index = 0;
        while SetupDiEnumDeviceInfo(set, index, &mut data) != 0 {
            index += 1;
            let mut buf = [0u16; 512];
            let mut kind = 0;
            let mut size = 0;
            if SetupDiGetDeviceRegistryPropertyW(
                set,
                &data,
                SPDRP_HARDWAREID,
                &mut kind,
                buf.as_mut_ptr().cast(),
                size_of_val(&buf) as u32,
                &mut size,
            ) == 0
            {
                continue;
            }
            let hardware = text(&buf);
            let Some((vid, pid)) = ids(&hardware) else {
                continue;
            };
            let key = SetupDiOpenDevRegKey(set, &data, DICS_FLAG_GLOBAL, 0, DIREG_DEV, KEY_READ);
            if key.is_null() || key == INVALID_HANDLE_VALUE {
                continue;
            }
            let mut name = [0u16; 32];
            let mut size = size_of_val(&name) as u32;
            let mut kind = 0;
            let value = wide("PortName");
            let read = RegQueryValueExW(
                key,
                value.as_ptr(),
                null(),
                &mut kind,
                name.as_mut_ptr().cast(),
                &mut size,
            );
            RegCloseKey(key);
            let name = text(&name);
            if read == 0 && kind == REG_SZ && name.starts_with("COM") {
                out.push(Port { name, vid, pid });
            }
        }
        SetupDiDestroyDeviceInfoList(set);
    }
    out
}

fn text(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// The SMBIOS baseboard manufacturer.
pub fn board_manufacturer() -> String {
    use windows_sys::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
    let sub = wide(r"HARDWARE\DESCRIPTION\System\BIOS");
    let value = wide("BaseBoardManufacturer");
    let mut buf = [0u16; 256];
    let mut len = size_of_val(&buf) as u32;
    // SAFETY: both names are NUL-terminated and `len` is the byte size of `buf`.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            sub.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            null_mut(),
            buf.as_mut_ptr().cast(),
            &mut len,
        )
    };
    if status != 0 {
        return String::new();
    }
    text(&buf).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hardware_ids() {
        assert_eq!(
            ids(r"USB\VID_1A86&PID_7523&REV_0264"),
            Some((0x1A86, 0x7523))
        );
        assert_eq!(ids(r"USB\VID_3402&PID_0B01"), Some((0x3402, 0x0B01)));
        assert_eq!(ids(r"HID\VID_3402&PID_0B01"), None);
    }
}
