//! HID collections from Windows' device records, without asking any device for its strings:
//! a device that never answers costs hidapi 5 s per string on every listing.

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    CM_GET_DEVICE_INTERFACE_LIST_PRESENT, CM_Get_DevNode_PropertyW, CM_Get_Device_IDW,
    CM_Get_Device_Interface_ListW, CM_Get_Device_Interface_List_SizeW,
    CM_Get_Device_Interface_PropertyW, CM_Get_Parent, CM_LOCATE_DEVNODE_NORMAL,
    CM_Locate_DevNodeW, CR_SUCCESS,
};
use windows_sys::Win32::Devices::HumanInterfaceDevice::{
    HIDD_ATTRIBUTES, HIDP_CAPS, HidD_FreePreparsedData, HidD_GetAttributes, HidD_GetHidGuid,
    HidD_GetPreparsedData, HidD_GetSerialNumberString, HidP_GetCaps,
};
use windows_sys::Win32::Devices::Properties::{
    DEVPKEY_Device_BusReportedDeviceDesc, DEVPKEY_Device_InstanceId, DEVPROPTYPE,
};
use windows_sys::Win32::Foundation::{CloseHandle, DEVPROPKEY, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::core::GUID;

use crate::registry::{Bus, Info};

const HIDP_STATUS_SUCCESS: i32 = 0x0011_0000;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn text(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    OsString::from_wide(&buf[..end]).to_string_lossy().into_owned()
}

fn interface_paths() -> Vec<String> {
    let mut guid: GUID = unsafe { std::mem::zeroed() };
    unsafe { HidD_GetHidGuid(&mut guid) };
    for _ in 0..3 {
        let mut len = 0u32;
        let flags = CM_GET_DEVICE_INTERFACE_LIST_PRESENT;
        if unsafe { CM_Get_Device_Interface_List_SizeW(&mut len, &guid, null(), flags) } != CR_SUCCESS {
            return Vec::new();
        }
        let mut buf = vec![0u16; len as usize];
        let got = unsafe {
            CM_Get_Device_Interface_ListW(&guid, null(), buf.as_mut_ptr(), len, flags)
        };
        if got == CR_SUCCESS {
            return buf
                .split(|&c| c == 0)
                .filter(|s| !s.is_empty())
                .map(|s| OsString::from_wide(s).to_string_lossy().into_owned())
                .collect();
        }
    }
    Vec::new()
}

fn string_property(node: u32, key: &DEVPROPKEY) -> String {
    let mut kind: DEVPROPTYPE = 0;
    let mut buf = [0u16; 256];
    let mut size = (buf.len() * 2) as u32;
    let ok = unsafe {
        CM_Get_DevNode_PropertyW(node, key, &mut kind, buf.as_mut_ptr().cast(), &mut size, 0)
    };
    if ok == CR_SUCCESS { text(&buf) } else { String::new() }
}

fn node_id(node: u32) -> String {
    let mut buf = [0u16; 512];
    if unsafe { CM_Get_Device_IDW(node, buf.as_mut_ptr(), buf.len() as u32, 0) } == CR_SUCCESS {
        text(&buf)
    } else {
        String::new()
    }
}

fn parent(node: u32) -> Option<u32> {
    let mut up = 0u32;
    (unsafe { CM_Get_Parent(&mut up, node, 0) } == CR_SUCCESS).then_some(up)
}

/// The bus and the product name Windows recorded for the device a collection belongs to.
fn origin(path: &str) -> (Bus, String) {
    let mut kind: DEVPROPTYPE = 0;
    let mut buf = [0u16; 512];
    let mut size = (buf.len() * 2) as u32;
    let p = wide(path);
    let ok = unsafe {
        CM_Get_Device_Interface_PropertyW(
            p.as_ptr(),
            &DEVPKEY_Device_InstanceId,
            &mut kind,
            buf.as_mut_ptr().cast(),
            &mut size,
            0,
        )
    };
    if ok != CR_SUCCESS {
        return (Bus::Other, String::new());
    }
    let mut node = 0u32;
    if unsafe { CM_Locate_DevNodeW(&mut node, buf.as_ptr(), CM_LOCATE_DEVNODE_NORMAL) } != CR_SUCCESS {
        return (Bus::Other, String::new());
    }
    let Some(up) = parent(node) else {
        return (Bus::Other, String::new());
    };
    let id = node_id(up).to_ascii_uppercase();
    let bus = if id.starts_with("USB\\") {
        Bus::Usb
    } else if id.starts_with("BTHENUM\\") || id.starts_with("BTHLEDEVICE\\") || id.contains("00001124") {
        Bus::Bluetooth
    } else {
        Bus::Other
    };
    let mut name = string_property(up, &DEVPKEY_Device_BusReportedDeviceDesc);
    if name.is_empty()
        && let Some(top) = parent(up)
    {
        name = string_property(top, &DEVPKEY_Device_BusReportedDeviceDesc);
    }
    (bus, name.trim().to_string())
}

fn interface_number(path: &str) -> i32 {
    let lower = path.to_ascii_lowercase();
    lower
        .find("&mi_")
        .and_then(|at| lower.get(at + 4..at + 6))
        .and_then(|n| i32::from_str_radix(n, 16).ok())
        .unwrap_or(-1)
}

/// One collection: ids and usage from the HID driver's cached data, no device I/O.
fn read(path: &str) -> Option<Info> {
    let p = wide(path);
    let handle = unsafe {
        CreateFileW(
            p.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            null(),
            OPEN_EXISTING,
            0,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return None;
    }
    let mut attrs: HIDD_ATTRIBUTES = unsafe { std::mem::zeroed() };
    attrs.Size = std::mem::size_of::<HIDD_ATTRIBUTES>() as u32;
    let got = unsafe { HidD_GetAttributes(handle, &mut attrs) };
    let mut caps: HIDP_CAPS = unsafe { std::mem::zeroed() };
    let mut parsed = 0isize;
    let usage = if unsafe { HidD_GetPreparsedData(handle, &mut parsed) } {
        let ok = unsafe { HidP_GetCaps(parsed, &mut caps) } == HIDP_STATUS_SUCCESS;
        unsafe { HidD_FreePreparsedData(parsed) };
        ok.then_some((caps.UsagePage, caps.Usage))
    } else {
        None
    };
    unsafe { CloseHandle(handle) };
    if !got {
        return None;
    }
    let (usage_page, usage) = usage.unwrap_or((0, 0));
    let (bus, product) = origin(path);
    Some(Info {
        vid: attrs.VendorID,
        pid: attrs.ProductID,
        interface: interface_number(path),
        usage_page,
        usage,
        path: path.to_string(),
        serial: String::new(),
        product,
        bus,
        release: attrs.VersionNumber,
    })
}

/// Every HID collection Windows lists now. Serial numbers are left empty.
pub fn collections() -> Vec<Info> {
    interface_paths().iter().filter_map(|p| read(p)).collect()
}

/// The serial number string of one collection; a device that does not answer gets none.
fn serial(path: &str) -> String {
    let p = wide(path);
    let handle = unsafe {
        CreateFileW(p.as_ptr(), 0, FILE_SHARE_READ | FILE_SHARE_WRITE, null(), OPEN_EXISTING, 0, null_mut())
    };
    if handle == INVALID_HANDLE_VALUE {
        return String::new();
    }
    let mut buf = [0u16; 128];
    let ok = unsafe { HidD_GetSerialNumberString(handle, buf.as_mut_ptr().cast(), 256) };
    unsafe { CloseHandle(handle) };
    if ok { text(&buf).trim().to_string() } else { String::new() }
}

/// Collections with serial numbers, each asked at once and given at most `wait` to answer.
pub fn with_serials(wait: std::time::Duration) -> Vec<Info> {
    let mut list = collections();
    let (tx, rx) = std::sync::mpsc::channel();
    for (i, c) in list.iter().enumerate() {
        let (tx, path) = (tx.clone(), c.path.clone());
        std::thread::Builder::new().name("foxden-hid-serial".into()).spawn(move || {
            let _ = tx.send((i, serial(&path)));
        }).expect("thread start");
    }
    drop(tx);
    let deadline = std::time::Instant::now() + wait;
    while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
        match rx.recv_timeout(left) {
            Ok((i, serial)) => list[i].serial = serial,
            Err(_) => break,
        }
    }
    list
}

/// A collection with the methods drivers used on hidapi's `crate::Dev`.
#[derive(Clone, Debug)]
pub struct Dev {
    info: Info,
    path: std::ffi::CString,
}

impl Dev {
    pub fn new(info: Info) -> Dev {
        let path = std::ffi::CString::new(info.path.clone()).unwrap_or_default();
        Dev { info, path }
    }
    pub fn info(&self) -> &Info {
        &self.info
    }
    pub fn vendor_id(&self) -> u16 {
        self.info.vid
    }
    pub fn product_id(&self) -> u16 {
        self.info.pid
    }
    pub fn path(&self) -> &std::ffi::CStr {
        &self.path
    }
    pub fn serial_number(&self) -> Option<&str> {
        Some(&self.info.serial)
    }
    pub fn product_string(&self) -> Option<&str> {
        Some(&self.info.product)
    }
    pub fn manufacturer_string(&self) -> Option<&str> {
        Some("")
    }
    pub fn interface_number(&self) -> i32 {
        self.info.interface
    }
    pub fn usage_page(&self) -> u16 {
        self.info.usage_page
    }
    pub fn usage(&self) -> u16 {
        self.info.usage
    }
    pub fn release_number(&self) -> u16 {
        self.info.release
    }
    pub fn open_device(&self, api: &hidapi::HidApi) -> hidapi::HidResult<hidapi::HidDevice> {
        api.open_path(&self.path)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn interface_numbers_come_from_the_path() {
        let path = r"\\?\hid#vid_046d&pid_c547&mi_02&col01#8&1a2b&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}";
        assert_eq!(super::interface_number(path), 2);
        assert_eq!(super::interface_number(r"\\?\hid#vid_1462&pid_7d99#7&abc#{guid}"), -1);
    }
}
