//! Ported from OpenRGB's libusb device access (GPL-2.0-or-later): WinUSB in place of UsbDK.

use std::ptr::{null, null_mut};

use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    DICS_FLAG_GLOBAL, DIGCF_ALLCLASSES, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, DIREG_DEV,
    SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W, SP_DEVINFO_DATA,
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiEnumDeviceInterfaces,
    SetupDiGetClassDevsW, SetupDiGetDeviceInstanceIdW, SetupDiGetDeviceInterfaceDetailW,
    SetupDiOpenDevRegKey,
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
use windows_sys::Win32::System::Registry::{KEY_READ, RegCloseKey, RegQueryValueExW};
use windows_sys::core::GUID;

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
        let want = format!("vid_{vid:04x}&pid_{pid:04x}#");
        let path = find(&GUID_DEVINTERFACE_USB_DEVICE, &want).ok_or("not present")?;
        WinUsb::at(&path, out_ep, in_ep)
    }

    /// One interface of a composite device, with WinUSB bound to that interface alone.
    pub fn open_interface(
        vid: u16,
        pid: u16,
        interface: u8,
        out_ep: u8,
        in_ep: u8,
    ) -> Result<WinUsb, String> {
        let mi = format!("vid_{vid:04x}&pid_{pid:04x}&mi_{interface:02x}");
        let guid = interface_guid(&mi).ok_or("not bound to WinUSB")?;
        let path = find(&guid, &format!("{mi}#")).ok_or("not present")?;
        WinUsb::at(&path, out_ep, in_ep)
    }

    fn at(path: &str, out_ep: u8, in_ep: u8) -> Result<WinUsb, String> {
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

/// `{xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx}` to a GUID.
pub fn parse_guid(text: &str) -> Option<GUID> {
    let hex: String = text
        .trim()
        .strip_prefix('{')?
        .strip_suffix('}')?
        .split('-')
        .collect();
    if hex.len() != 32 {
        return None;
    }
    u128::from_str_radix(&hex, 16).ok().map(GUID::from_u128)
}

/// The interface GUID the WinUSB INF gave the device whose instance id starts `USB\<mi>\`.
fn interface_guid(mi: &str) -> Option<GUID> {
    let want = format!("usb\\{mi}\\");
    let usb: Vec<u16> = "USB".encode_utf16().chain(Some(0)).collect();
    // SAFETY: every buffer handed to SetupAPI and the registry outlives its call.
    unsafe {
        let set = SetupDiGetClassDevsW(
            null(),
            usb.as_ptr(),
            null_mut(),
            DIGCF_ALLCLASSES | DIGCF_PRESENT,
        );
        if set == INVALID_HANDLE_VALUE as isize {
            return None;
        }
        let mut found = None;
        let mut data: SP_DEVINFO_DATA = std::mem::zeroed();
        data.cbSize = size_of::<SP_DEVINFO_DATA>() as u32;
        let mut index = 0;
        while found.is_none() && SetupDiEnumDeviceInfo(set, index, &mut data) != 0 {
            index += 1;
            let mut id = [0u16; 256];
            if SetupDiGetDeviceInstanceIdW(set, &data, id.as_mut_ptr(), 256, null_mut()) == 0
                || !text(&id).to_ascii_lowercase().starts_with(&want)
            {
                continue;
            }
            let key = SetupDiOpenDevRegKey(set, &data, DICS_FLAG_GLOBAL, 0, DIREG_DEV, KEY_READ);
            if key.is_null() || key == INVALID_HANDLE_VALUE {
                continue;
            }
            for name in ["DeviceInterfaceGUIDs", "DeviceInterfaceGUID"] {
                let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
                let mut buf = [0u16; 256];
                let mut size = size_of_val(&buf) as u32;
                let read = RegQueryValueExW(
                    key,
                    name.as_ptr(),
                    null(),
                    null_mut(),
                    buf.as_mut_ptr().cast(),
                    &mut size,
                );
                if read == 0 && found.is_none() {
                    found = parse_guid(&text(&buf));
                }
            }
            RegCloseKey(key);
        }
        SetupDiDestroyDeviceInfoList(set);
        found
    }
}

fn text(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

fn find(guid: &GUID, want: &str) -> Option<String> {
    // SAFETY: every buffer handed to SetupAPI outlives its call.
    unsafe {
        let set = SetupDiGetClassDevsW(
            guid,
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
            && SetupDiEnumDeviceInterfaces(set, null(), guid, index, &mut data) != 0
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
            if path.to_ascii_lowercase().contains(want) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guids() {
        let g = parse_guid("{A5DCBF10-6530-11D2-901F-00C04FB951ED}").unwrap();
        let usb = GUID_DEVINTERFACE_USB_DEVICE;
        assert_eq!(
            (g.data1, g.data2, g.data3, g.data4),
            (usb.data1, usb.data2, usb.data3, usb.data4)
        );
        assert!(parse_guid("A5DCBF10-6530-11D2-901F-00C04FB951ED").is_none());
        assert!(parse_guid("{A5DCBF10-6530-11D2-901F}").is_none());
    }
}
