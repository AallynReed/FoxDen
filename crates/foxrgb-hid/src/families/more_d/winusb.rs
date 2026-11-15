//! Ported from OpenRGB's libusb device access (GPL-2.0-or-later): WinUSB in place of UsbDK.

use std::ptr::{null, null_mut};

use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, SP_DEVICE_INTERFACE_DATA,
    SP_DEVICE_INTERFACE_DETAIL_DATA_W, SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces,
    SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW,
};
use windows_sys::Win32::Devices::Usb::{
    GUID_DEVINTERFACE_USB_DEVICE, PIPE_TRANSFER_TIMEOUT, WINUSB_INTERFACE_HANDLE,
    WINUSB_SETUP_PACKET, WinUsb_ControlTransfer, WinUsb_Free, WinUsb_Initialize, WinUsb_ReadPipe,
    WinUsb_ResetPipe, WinUsb_SetPipePolicy, WinUsb_WritePipe,
};
use windows_sys::Win32::Foundation::{
    CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_OVERLAPPED, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};

use super::common::Io;

/// What libusb_bulk_transfer waits in OpenRGB.
const TIMEOUT_MS: u32 = 1000;

/// A device bound to WinUSB, with the bulk endpoints a driver talks on.
pub struct WinUsb {
    file: HANDLE,
    usb: WINUSB_INTERFACE_HANDLE,
    out_ep: u8,
    in_ep: u8,
}

// SAFETY: both handles are only used through `&mut self`.
unsafe impl Send for WinUsb {}

impl WinUsb {
    /// The first present device with these ids, as `libusb_open_device_with_vid_pid` picks it.
    pub fn open(vid: u16, pid: u16, out_ep: u8, in_ep: u8) -> Result<WinUsb, String> {
        let path = find(vid, pid).ok_or("not present")?;
        let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
        // SAFETY: `wide` is NUL-terminated; the handles are closed on every failure path.
        unsafe {
            let file = CreateFileW(
                wide.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                null(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OVERLAPPED,
                null_mut(),
            );
            if file == INVALID_HANDLE_VALUE {
                return Err("open failed".into());
            }
            let mut usb = null_mut();
            if WinUsb_Initialize(file, &mut usb) == 0 {
                CloseHandle(file);
                return Err("not bound to WinUSB".into());
            }
            let timeout = TIMEOUT_MS;
            for ep in [out_ep, in_ep] {
                WinUsb_SetPipePolicy(
                    usb,
                    ep,
                    PIPE_TRANSFER_TIMEOUT,
                    4,
                    (&raw const timeout).cast(),
                );
            }
            Ok(WinUsb {
                file,
                usb,
                out_ep,
                in_ep,
            })
        }
    }
}

fn find(vid: u16, pid: u16) -> Option<String> {
    let want = format!("vid_{vid:04x}&pid_{pid:04x}#");
    // SAFETY: every buffer handed to SetupAPI outlives its call.
    unsafe {
        let set = SetupDiGetClassDevsW(
            &GUID_DEVINTERFACE_USB_DEVICE,
            null(),
            null_mut(),
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        );
        if set == INVALID_HANDLE_VALUE as isize {
            return None;
        }
        let mut found = None;
        let mut data: SP_DEVICE_INTERFACE_DATA = std::mem::zeroed();
        data.cbSize = size_of::<SP_DEVICE_INTERFACE_DATA>() as u32;
        let mut index = 0;
        while found.is_none()
            && SetupDiEnumDeviceInterfaces(
                set,
                null(),
                &GUID_DEVINTERFACE_USB_DEVICE,
                index,
                &mut data,
            ) != 0
        {
            index += 1;
            let mut buf = [0u64; 128];
            let detail = buf.as_mut_ptr().cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
            (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
            if SetupDiGetDeviceInterfaceDetailW(
                set,
                &data,
                detail,
                size_of_val(&buf) as u32,
                null_mut(),
                null_mut(),
            ) == 0
            {
                continue;
            }
            let start = (&raw const (*detail).DevicePath).cast::<u16>();
            let max = (size_of_val(&buf) - 4) / 2;
            let chars = std::slice::from_raw_parts(start, max);
            let end = chars.iter().position(|&c| c == 0).unwrap_or(max);
            let path = String::from_utf16_lossy(&chars[..end]);
            if path.to_ascii_lowercase().contains(&want) {
                found = Some(path);
            }
        }
        SetupDiDestroyDeviceInfoList(set);
        found
    }
}

impl Drop for WinUsb {
    fn drop(&mut self) {
        // SAFETY: both handles are open and released once.
        unsafe {
            WinUsb_Free(self.usb);
            CloseHandle(self.file);
        }
    }
}

impl Io for WinUsb {
    fn write(&mut self, data: &[u8]) -> Result<usize, String> {
        let mut done = 0;
        // SAFETY: `data` outlives the synchronous call.
        let ok = unsafe {
            WinUsb_WritePipe(
                self.usb,
                self.out_ep,
                data.as_ptr(),
                data.len() as u32,
                &mut done,
                null(),
            )
        };
        if ok == 0 {
            return Err("bulk write failed".into());
        }
        Ok(done as usize)
    }
    fn read(&mut self, buf: &mut [u8], _timeout_ms: i32) -> Result<usize, String> {
        let mut done = 0;
        // SAFETY: `buf` outlives the synchronous call.
        let ok = unsafe {
            WinUsb_ReadPipe(
                self.usb,
                self.in_ep,
                buf.as_mut_ptr(),
                buf.len() as u32,
                &mut done,
                null(),
            )
        };
        if ok == 0 {
            return Err("bulk read failed".into());
        }
        Ok(done as usize)
    }
    fn control(&mut self, kind: u8, request: u8, value: u16, index: u16) -> Result<(), String> {
        let setup = WINUSB_SETUP_PACKET {
            RequestType: kind,
            Request: request,
            Value: value,
            Index: index,
            Length: 0,
        };
        let mut done = 0;
        // SAFETY: no data stage, so no buffer.
        let ok =
            unsafe { WinUsb_ControlTransfer(self.usb, setup, null_mut(), 0, &mut done, null()) };
        if ok == 0 {
            return Err("control transfer failed".into());
        }
        Ok(())
    }
    /// WinUSB cannot reset the device the way libusb_reset_device does; resetting both pipes is the nearest.
    fn reset(&mut self) -> Result<(), String> {
        // SAFETY: the interface handle is open.
        unsafe {
            WinUsb_ResetPipe(self.usb, self.out_ep);
            WinUsb_ResetPipe(self.usb, self.in_ep);
        }
        Ok(())
    }
}
