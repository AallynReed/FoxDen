// SPDX-License-Identifier: GPL-2.0-or-later
//! USB device access through WinUSB.

use std::ptr::{null, null_mut};

use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    DICS_FLAG_GLOBAL, DIGCF_ALLCLASSES, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, DIREG_DEV,
    SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W, SP_DEVINFO_DATA,
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiEnumDeviceInterfaces,
    SetupDiGetClassDevsW, SetupDiGetDeviceInstanceIdW, SetupDiGetDeviceInterfaceDetailW,
    SetupDiOpenDevRegKey,
};
use windows_sys::Win32::Devices::Usb::{
    GUID_DEVINTERFACE_USB_DEVICE, PIPE_TRANSFER_TIMEOUT, USB_INTERFACE_DESCRIPTOR,
    WINUSB_INTERFACE_HANDLE, WINUSB_PIPE_INFORMATION, WINUSB_SETUP_PACKET, WinUsb_ControlTransfer,
    WinUsb_Free, WinUsb_GetAssociatedInterface, WinUsb_GetDescriptor, WinUsb_Initialize,
    WinUsb_QueryInterfaceSettings, WinUsb_QueryPipe, WinUsb_ReadPipe, WinUsb_ResetPipe,
    WinUsb_SetPipePolicy, WinUsb_WritePipe,
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

/// How long a bulk transfer waits.
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
        let path = find_all(&GUID_DEVINTERFACE_USB_DEVICE, &want)
            .into_iter()
            .next()
            .ok_or("not present")?;
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
        let guid = interface_guids(&format!("{mi}\\"))
            .into_iter()
            .next()
            .ok_or("not bound to WinUSB")?;
        let path = find_all(&guid, &format!("{mi}#"))
            .into_iter()
            .next()
            .ok_or("not present")?;
        WinUsb::at(&path, out_ep, in_ep)
    }

    /// Every WinUSB path of a device with these ids: the whole device and each bound interface.
    pub fn paths(vid: u16, pid: u16) -> Vec<String> {
        let ids = format!("vid_{vid:04x}&pid_{pid:04x}");
        let mut out = find_all(&GUID_DEVINTERFACE_USB_DEVICE, &format!("{ids}#"));
        let mut guids = interface_guids(&format!("{ids}&mi_"));
        let key = |g: &GUID| (g.data1, g.data2, g.data3, g.data4);
        guids.sort_by_key(key);
        guids.dedup_by_key(|g| key(g));
        for g in guids {
            for p in find_all(&g, &format!("{ids}&mi_")) {
                if !out.contains(&p) {
                    out.push(p);
                }
            }
        }
        out
    }

    /// The paths of `paths` that WinUSB itself is bound to, open or not.
    pub fn bound_paths(vid: u16, pid: u16) -> Vec<String> {
        let ids = format!("vid_{vid:04x}&pid_{pid:04x}");
        let mut guids = interface_guids(&ids);
        let key = |g: &GUID| (g.data1, g.data2, g.data3, g.data4);
        guids.sort_by_key(key);
        guids.dedup_by_key(|g| key(g));
        let mut out = Vec::new();
        for g in guids {
            for p in find_all(&g, &ids) {
                if !out.contains(&p) {
                    out.push(p);
                }
            }
        }
        out
    }

    /// Opens one path from `paths`.
    pub fn at(path: &str, out_ep: u8, in_ep: u8) -> Result<WinUsb, String> {
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

/// Interface GUIDs the WinUSB INF gave devices whose instance id starts `USB\<prefix>`.
fn interface_guids(prefix: &str) -> Vec<GUID> {
    let want = format!("usb\\{prefix}");
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
            return Vec::new();
        }
        let mut found = Vec::new();
        let mut data: SP_DEVINFO_DATA = std::mem::zeroed();
        data.cbSize = size_of::<SP_DEVINFO_DATA>() as u32;
        let mut index = 0;
        while SetupDiEnumDeviceInfo(set, index, &mut data) != 0 {
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
                if read == 0 {
                    found.extend(parse_guid(&text(&buf)));
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

fn find_all(guid: &GUID, want: &str) -> Vec<String> {
    // SAFETY: every buffer handed to SetupAPI outlives its call.
    unsafe {
        let set = SetupDiGetClassDevsW(
            guid,
            null(),
            null_mut(),
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        );
        if set == INVALID_HANDLE_VALUE as isize {
            return Vec::new();
        }
        let mut found = Vec::new();
        let mut data: SP_DEVICE_INTERFACE_DATA = std::mem::zeroed();
        data.cbSize = size_of::<SP_DEVICE_INTERFACE_DATA>() as u32;
        let mut index = 0;
        while SetupDiEnumDeviceInterfaces(set, null(), guid, index, &mut data) != 0 {
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
                found.push(path);
            }
        }
        SetupDiDestroyDeviceInfoList(set);
        found
    }
}

impl WinUsb {
    /// Whether the bound interface has this endpoint.
    pub fn has_pipe(&self, ep: u8) -> bool {
        // SAFETY: both structs are plain data written by WinUSB.
        unsafe {
            let mut desc: USB_INTERFACE_DESCRIPTOR = std::mem::zeroed();
            if WinUsb_QueryInterfaceSettings(self.usb, 0, &mut desc) == 0 {
                return false;
            }
            (0..desc.bNumEndpoints).any(|i| {
                let mut pipe: WINUSB_PIPE_INFORMATION = std::mem::zeroed();
                WinUsb_QueryPipe(self.usb, 0, i, &mut pipe) != 0 && pipe.PipeId == ep
            })
        }
    }

    /// Bulk or interrupt read on the IN endpoint; empty when nothing came within `timeout_ms`.
    pub fn read_pipe(&mut self, len: usize, timeout_ms: u32) -> Result<Vec<u8>, String> {
        self.in_timeout(timeout_ms);
        let mut buf = vec![0u8; len];
        let got = Io::read(self, &mut buf, timeout_ms as i32);
        self.in_timeout(TIMEOUT_MS);
        match got {
            Ok(n) => {
                buf.truncate(n);
                Ok(buf)
            }
            Err(_) => Ok(Vec::new()),
        }
    }

    fn in_timeout(&mut self, ms: u32) {
        // SAFETY: the policy value outlives the call.
        unsafe {
            WinUsb_SetPipePolicy(
                self.usb,
                self.in_ep,
                PIPE_TRANSFER_TIMEOUT,
                4,
                (&raw const ms).cast(),
            );
        }
    }

    pub fn write_pipe(&mut self, data: &[u8]) -> Result<(), String> {
        Io::write(self, data).map(|_| ())
    }

    /// A control transfer with a data stage, OUT or IN by bit 7 of `kind`; returns the bytes moved.
    pub fn control_data(
        &mut self,
        kind: u8,
        request: u8,
        value: u16,
        index: u16,
        data: &mut [u8],
    ) -> Result<usize, String> {
        let setup = WINUSB_SETUP_PACKET {
            RequestType: kind,
            Request: request,
            Value: value,
            Index: index,
            Length: data.len() as u16,
        };
        let mut done = 0;
        // SAFETY: `data` outlives the synchronous call and is `Length` bytes long.
        let ok = unsafe {
            WinUsb_ControlTransfer(
                self.usb,
                setup,
                data.as_mut_ptr(),
                data.len() as u32,
                &mut done,
                null(),
            )
        };
        if ok == 0 {
            return Err("control transfer failed".into());
        }
        Ok(done as usize)
    }
}

/// Class SET_REPORT of an output report: request type, request, value and index.
pub fn set_report_setup(interface: u8, report_id: u8) -> (u8, u8, u16, u16) {
    (
        0x21,
        0x09,
        0x0200 | u16::from(report_id),
        u16::from(interface),
    )
}

impl WinUsb {
    fn interface_number(&self) -> Option<u8> {
        // SAFETY: plain data written by WinUSB.
        unsafe {
            let mut desc: USB_INTERFACE_DESCRIPTOR = std::mem::zeroed();
            (WinUsb_QueryInterfaceSettings(self.usb, 0, &mut desc) != 0)
                .then_some(desc.bInterfaceNumber)
        }
    }

    /// A HID output report sent over the control pipe to `interface`, report id first.
    pub fn set_report(&mut self, interface: u8, report: &[u8]) -> Result<(), String> {
        let Some(&id) = report.first() else {
            return Err("empty report".into());
        };
        let own = self.interface_number().unwrap_or(interface);
        let mut other = null_mut();
        let handle = if own == interface {
            self.usb
        } else {
            let index = interface
                .checked_sub(own.saturating_add(1))
                .ok_or("interface not reachable")?;
            // SAFETY: `other` is freed below.
            if unsafe { WinUsb_GetAssociatedInterface(self.usb, index, &mut other) } == 0 {
                return Err("interface not reachable".into());
            }
            other
        };
        let (kind, request, value, index) = set_report_setup(interface, id);
        let setup = WINUSB_SETUP_PACKET {
            RequestType: kind,
            Request: request,
            Value: value,
            Index: index,
            Length: report.len() as u16,
        };
        let mut data = report.to_vec();
        let mut done = 0;
        // SAFETY: `data` outlives the synchronous call and is `Length` bytes long.
        let ok = unsafe {
            WinUsb_ControlTransfer(
                handle,
                setup,
                data.as_mut_ptr(),
                data.len() as u32,
                &mut done,
                null(),
            )
        };
        if !other.is_null() {
            // SAFETY: an associated handle from above, freed once.
            unsafe { WinUsb_Free(other) };
        }
        if ok == 0 {
            return Err("SET_REPORT failed".into());
        }
        Ok(())
    }

    /// The product string descriptor, in US English.
    pub fn product(&mut self) -> Option<String> {
        let mut device = [0u8; 18];
        let mut n = 0;
        // SAFETY: both buffers outlive their synchronous calls.
        unsafe {
            if WinUsb_GetDescriptor(self.usb, 1, 0, 0, device.as_mut_ptr(), 18, &mut n) == 0
                || n < 18
                || device[15] == 0
            {
                return None;
            }
            let mut text = [0u8; 255];
            let len = text.len() as u32;
            if WinUsb_GetDescriptor(
                self.usb,
                3,
                device[15],
                0x0409,
                text.as_mut_ptr(),
                len,
                &mut n,
            ) == 0
            {
                return None;
            }
            Some(string_descriptor(&text[..n as usize]))
        }
    }
}

/// A USB string descriptor's UTF-16LE text.
pub fn string_descriptor(d: &[u8]) -> String {
    let end = d.first().map_or(0, |&l| usize::from(l).min(d.len()));
    let units: Vec<u16> = d
        .get(2..end)
        .unwrap_or_default()
        .as_chunks::<2>().0.iter()
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16_lossy(&units)
}

/// The instance part of a device path: the USB serial number when the device has a unique one.
pub fn instance(path: &str) -> &str {
    path.split('#').nth(2).unwrap_or_default()
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
        Ok((done as usize).min(buf.len()))
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

    // HID 1.11 7.2.2: SET_REPORT, report type 2 (output) in the high byte of wValue.
    #[test]
    fn set_report_and_strings() {
        assert_eq!(set_report_setup(1, 0xE0), (0x21, 0x09, 0x02E0, 1));
        let d = [12, 3, b'v', 0, b'1', 0, b'.', 0, b'8', 0, b'x', 0, 0, 0];
        assert_eq!(string_descriptor(&d), "v1.8x");
        assert_eq!(string_descriptor(&[]), "");
    }
}
