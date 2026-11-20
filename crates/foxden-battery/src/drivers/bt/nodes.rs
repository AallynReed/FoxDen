//! Bluetooth device nodes as Windows lists them (BTHENUM, BTHLE, BTHLEDEVICE).

use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_DevNode_Status, CR_SUCCESS, DIGCF_ALLCLASSES, DIGCF_PRESENT, DN_HAS_PROBLEM, HDEVINFO,
    SP_DEVINFO_DATA, SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiGetClassDevsW,
    SetupDiGetDeviceInstanceIdW, SetupDiGetDevicePropertyW,
};
use windows_sys::Win32::Devices::Properties::{
    DEVPKEY_Device_ContainerId, DEVPKEY_Device_DeviceDesc, DEVPKEY_Device_FriendlyName,
    DEVPKEY_DeviceContainer_Category, DEVPROP_TYPE_BOOLEAN, DEVPROP_TYPE_BYTE, DEVPROP_TYPE_GUID,
    DEVPROP_TYPE_STRING, DEVPROP_TYPE_STRING_LIST, DEVPROP_TYPE_UINT32, DEVPROPTYPE,
};
use windows_sys::Win32::Foundation::DEVPROPKEY;
use windows_sys::core::GUID;

const fn key(u: u128, pid: u32) -> DEVPROPKEY {
    DEVPROPKEY {
        fmtid: GUID::from_u128(u),
        pid,
    }
}

const BATTERY: DEVPROPKEY = key(0x104ea319_6ee2_4701_bd47_8ddbf425bbe5, 2);
const CONNECTED: DEVPROPKEY = key(0x83da6326_97a6_4088_9453_a1923f573b29, 15);
const CLASS_OF_DEVICE: DEVPROPKEY = key(0x2bd67d8b_8beb_48d5_87e0_6cda3428040a, 10);

#[derive(Clone, Debug, Default)]
pub struct Node {
    /// Instance id in upper case, e.g. `BTHENUM\DEV_6CFBED129847\...`.
    pub inst: String,
    /// Device container "{GUID}", or the instance id when there is none.
    pub container: String,
    pub name: String,
    pub connected: Option<bool>,
    pub battery: Option<u8>,
    pub cod: Option<u32>,
    pub category: String,
    pub problem: bool,
}

impl Node {
    /// The device itself rather than one of its services.
    pub fn is_device(&self) -> bool {
        self.inst.starts_with("BTHENUM\\DEV_") || self.inst.starts_with("BTHLE\\DEV_")
    }

    pub fn address(&self) -> Option<u64> {
        address_of(&self.inst)
    }
}

/// Bluetooth address from `...\DEV_<12 hex digits>`.
pub fn address_of(inst: &str) -> Option<u64> {
    let hex = inst.split("\\DEV_").nth(1)?.get(..12)?;
    u64::from_str_radix(hex, 16).ok().filter(|&a| a != 0)
}

pub fn guid_text(g: &GUID) -> String {
    let d = g.data4;
    format!(
        "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        g.data1, g.data2, g.data3, d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]
    )
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// A GUID from its in-memory bytes (little-endian fields).
pub fn guid_from_mem(b: &[u8]) -> GUID {
    GUID {
        data1: u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        data2: u16::from_le_bytes([b[4], b[5]]),
        data3: u16::from_le_bytes([b[6], b[7]]),
        data4: b[8..16].try_into().unwrap(),
    }
}

fn prop(set: HDEVINFO, dd: &SP_DEVINFO_DATA, key: &DEVPROPKEY) -> Option<(DEVPROPTYPE, Vec<u8>)> {
    let mut buf = vec![0u8; 512];
    let mut ty = 0;
    let mut need = 0;
    let ok = unsafe {
        SetupDiGetDevicePropertyW(
            set,
            dd,
            key,
            &mut ty,
            buf.as_mut_ptr(),
            buf.len() as u32,
            &mut need,
            0,
        )
    };
    if ok == 0 {
        return None;
    }
    buf.truncate(need as usize);
    Some((ty, buf))
}

pub fn utf16_text(bytes: &[u8]) -> String {
    let w: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    from_wide(&w)
}

fn read(set: HDEVINFO, dd: &SP_DEVINFO_DATA, inst: String) -> Node {
    let string = |k| match prop(set, dd, k) {
        Some((DEVPROP_TYPE_STRING | DEVPROP_TYPE_STRING_LIST, b)) => utf16_text(&b),
        _ => String::new(),
    };
    let container = match prop(set, dd, &DEVPKEY_Device_ContainerId) {
        Some((DEVPROP_TYPE_GUID, b)) if b.len() >= 16 => guid_text(&guid_from_mem(&b)),
        _ => inst.clone(),
    };
    let mut name = string(&DEVPKEY_Device_FriendlyName);
    if name.is_empty() {
        name = string(&DEVPKEY_Device_DeviceDesc);
    }
    let connected = match prop(set, dd, &CONNECTED) {
        Some((DEVPROP_TYPE_BOOLEAN, b)) if !b.is_empty() => Some(b[0] != 0),
        _ => None,
    };
    let battery = match prop(set, dd, &BATTERY) {
        Some((DEVPROP_TYPE_BYTE, b)) if !b.is_empty() => Some(b[0]),
        _ => None,
    };
    let cod = match prop(set, dd, &CLASS_OF_DEVICE) {
        Some((DEVPROP_TYPE_UINT32, b)) if b.len() >= 4 => {
            Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        }
        _ => None,
    };
    let (mut status, mut code) = (0, 0);
    let problem = unsafe { CM_Get_DevNode_Status(&mut status, &mut code, dd.DevInst, 0) }
        == CR_SUCCESS
        && status & DN_HAS_PROBLEM != 0;
    let category = string(&DEVPKEY_DeviceContainer_Category);
    Node {
        inst,
        container,
        name,
        connected,
        battery,
        cod,
        category,
        problem,
    }
}

/// Present nodes of one enumerator ("BTHENUM", "BTHLE", "BTHLEDEVICE").
pub fn list(enumerator: &str) -> Vec<Node> {
    let e = wide(enumerator);
    let flags = DIGCF_ALLCLASSES | DIGCF_PRESENT;
    let set =
        unsafe { SetupDiGetClassDevsW(std::ptr::null(), e.as_ptr(), std::ptr::null_mut(), flags) };
    if set == -1 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut dd: SP_DEVINFO_DATA = unsafe { std::mem::zeroed() };
    dd.cbSize = size_of::<SP_DEVINFO_DATA>() as u32;
    let mut i = 0;
    while unsafe { SetupDiEnumDeviceInfo(set, i, &mut dd) } != 0 {
        i += 1;
        let mut id = [0u16; 512];
        let got = unsafe {
            SetupDiGetDeviceInstanceIdW(
                set,
                &dd,
                id.as_mut_ptr(),
                id.len() as u32,
                std::ptr::null_mut(),
            )
        };
        if got != 0 {
            out.push(read(set, &dd, from_wide(&id).to_uppercase()));
        }
    }
    unsafe { SetupDiDestroyDeviceInfoList(set) };
    out
}

/// Name and address of each paired classic device Windows says is connected.
pub fn connected_classic() -> Vec<(String, u64)> {
    list("BTHENUM")
        .into_iter()
        .filter(|n| n.is_device() && n.connected == Some(true) && !n.name.is_empty())
        .filter_map(|n| Some((n.name.clone(), n.address()?)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_and_container_come_from_the_node() {
        assert_eq!(
            address_of("BTHENUM\\DEV_6CFBED129847\\7&1"),
            Some(0x6CFB_ED12_9847)
        );
        assert_eq!(address_of("BTHENUM\\{0000110B}_VID&0001004C"), None);
        let mem = [4, 3, 2, 1, 6, 5, 8, 7, 9, 10, 11, 12, 13, 14, 15, 16];
        assert_eq!(
            guid_text(&guid_from_mem(&mem)),
            "{01020304-0506-0708-090A-0B0C0D0E0F10}"
        );
    }
}
