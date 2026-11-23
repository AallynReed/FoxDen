use std::ffi::CString;
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_IO_PENDING, GENERIC_READ, GetLastError, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileA, FILE_FLAG_OVERLAPPED, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::{
    CancelIoEx, DeviceIoControl, GetOverlappedResult, OVERLAPPED,
};
use windows_sys::Win32::System::Threading::{CreateEventA, WaitForSingleObject};

use crate::hid::{Coll, Dev};
use crate::util::hex4;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x0ECB;
const BATTERY_FEATURE: u16 = 0x000D;
/// CTL_CODE(FILE_DEVICE_KEYBOARD, 100, METHOD_OUT_DIRECT, FILE_ANY_ACCESS).
const IOCTL_HID_GET_FEATURE: u32 = 0x000B_0192;

/// HIDV3 models: (pid, name, on the 2.4 GHz dongle).
const HIDV3: &[(u16, &str, bool)] = &[
    (0x2125, "JBL Quantum 650 Wireless", true),
    (0x2126, "JBL Quantum 650 Wireless", false),
    (0x213B, "JBL Quantum 650X Wireless", true),
    (0x213D, "JBL Quantum 650X Wireless", false),
    (0x2127, "JBL Quantum 950 Wireless", true),
    (0x2128, "JBL Quantum 950 Wireless", false),
    (0x213E, "JBL Quantum 950X Wireless", true),
    (0x2140, "JBL Quantum 950X Wireless", false),
    (0x218D, "JBL Quantum 520 Wireless", true),
    (0x218E, "JBL Quantum 520 Wireless", false),
    (0x21A7, "JBL Quantum 520X Wireless", true),
    (0x21A9, "JBL Quantum 520X Wireless", false),
    (0x219A, "JBL Quantum 850 Wireless", true),
    (0x219B, "JBL Quantum 850 Wireless", false),
];

/// HIDV1 models: (pid, interface, required usage page with usage 0 or 0 for any, name).
const HIDV1: &[(u16, i32, u16, &str)] = &[
    (0x206B, 3, 0x0001, "JBL Quantum 350 Wireless"),
    (0x205C, 5, 0, "JBL Quantum 610 Wireless"),
    (0x2115, 5, 0x0001, "JBL Quantum 360 Wireless"),
];

fn key(pid: u16) -> String {
    format!("jbl-{}:{}", hex4(VID), hex4(pid))
}

/// A numbered 2-byte feature report, read with an exact-size buffer.
pub fn get_feature_byte(c: &Coll, id: u8) -> Option<u8> {
    let path = CString::new(c.path.as_str()).ok()?;
    unsafe {
        let h = CreateFileA(
            path.as_ptr() as _,
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_OVERLAPPED,
            std::ptr::null_mut(),
        );
        if h == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut buf = [id, 0xFF];
        let mut ov: OVERLAPPED = std::mem::zeroed();
        ov.hEvent = CreateEventA(std::ptr::null(), 1, 0, std::ptr::null());
        let mut got = 0u32;
        let p = buf.as_mut_ptr();
        let mut ok = DeviceIoControl(
            h,
            IOCTL_HID_GET_FEATURE,
            p as _,
            2,
            p as _,
            2,
            &mut got,
            &mut ov,
        ) != 0;
        if !ok && GetLastError() == ERROR_IO_PENDING {
            if WaitForSingleObject(ov.hEvent, 200) != WAIT_OBJECT_0 {
                CancelIoEx(h, &ov);
            }
            ok = GetOverlappedResult(h, &ov, &mut got, 1) != 0;
        }
        CloseHandle(ov.hEvent);
        CloseHandle(h);
        (ok && got == 2 && buf[0] == id).then_some(buf[1])
    }
}

fn poll_hidv1(ctx: &mut Ctx) {
    for &(pid, iface, page, name) in HIDV1 {
        if ctx.is_claimed(VID, pid) {
            continue;
        }
        let colls = ctx.find(VID, pid, |c| {
            c.iface == iface && c.feat_len >= 2 && (page == 0 || (c.page == page && c.usage == 0))
        });
        for c in colls {
            // 68 = wireless link (01 = headset on), 49 = battery percent.
            if get_feature_byte(&c, 0x68) != Some(1) {
                continue;
            }
            let Some(percent) = get_feature_byte(&c, 0x49).filter(|&p| p <= 100) else {
                continue;
            };
            let b = Battery {
                key: ctx.stable_key(&key(pid)),
                name: name.into(),
                via: "2.4 GHz receiver".into(),
                kind: Kind::Headset,
                percent: Some(percent),
                ..Battery::default()
            };
            ctx.add(b, VID, pid);
            break;
        }
    }
}

/// Feature 88 "get device info" for the device itself (DD00), command 0001, one packet, feature id payload.
fn info_request(feature: u16) -> [u8; 11] {
    let [lo, hi] = feature.to_le_bytes();
    [0x88, 0x00, 0xDD, 0x01, 0x00, 0x01, 0x00, 0x02, 0x00, lo, hi]
}

/// Input report 86: header echo, `[9..11]` status (bit 15 = error), `[11..13]` feature, `[13..15]` size, value.
/// `None` = not this reply, `Some(None)` = error.
fn parse_info(r: &[u8], feature: u16) -> Option<Option<u8>> {
    if r.len() < 16 || r[..6] != [0x86, 0x00, 0xDD, 0x01, 0x00, 0x01] {
        return None;
    }
    if r[10] & 0x80 != 0 {
        return Some(None);
    }
    if u16::from_le_bytes([r[11], r[12]]) != feature || (r[13] == 0 && r[14] == 0) {
        return None;
    }
    Some(Some(r[15]))
}

/// Battery value: bit 7 charging, bits 0..6 percent.
fn parse_battery(v: u8) -> Option<(u8, bool)> {
    let p = v & 0x7F;
    (p <= 100).then_some((p, v & 0x80 != 0))
}

fn hidv3_battery(d: &Dev) -> Option<(u8, bool)> {
    d.flush();
    if !d.set_feature(&info_request(BATTERY_FEATURE)) {
        return None;
    }
    d.read_until(Duration::from_millis(800), |r| {
        parse_info(r, BATTERY_FEATURE)
    })?
    .and_then(parse_battery)
}

pub fn poll_jbl(ctx: &mut Ctx) {
    poll_hidv1(ctx);
    for &(pid, name, dongle) in HIDV3 {
        if ctx.is_claimed(VID, pid) {
            continue;
        }
        for c in ctx.find(VID, pid, |c| {
            c.page == 0xFF12 && c.feat_len >= 11 && c.in_len >= 16
        }) {
            let Some((percent, charging)) = Dev::open(&c).and_then(|d| hidv3_battery(&d)) else {
                continue;
            };
            let b = Battery {
                key: ctx.stable_key(&key(pid)),
                name: name.into(),
                via: if dongle {
                    "2.4 GHz receiver"
                } else {
                    "USB cable"
                }
                .into(),
                kind: Kind::Headset,
                percent: Some(percent),
                charging,
                ..Battery::default()
            };
            ctx.add(b, VID, pid);
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidv3_battery_reply() {
        assert_eq!(info_request(BATTERY_FEATURE)[9..], [0x0D, 0x00]);
        let mut r = [0u8; 64];
        r[..16].copy_from_slice(&[
            0x86,
            0x00,
            0xDD,
            0x01,
            0x00,
            0x01,
            0x00,
            0x07,
            0x00,
            0x00,
            0x00,
            0x0D,
            0x00,
            0x01,
            0x00,
            0x80 | 64,
        ]);
        let v = parse_info(&r, BATTERY_FEATURE).unwrap().unwrap();
        assert_eq!(parse_battery(v), Some((64, true)));
        r[10] = 0x80;
        assert_eq!(parse_info(&r, BATTERY_FEATURE), Some(None));
        r[10] = 0;
        r[11] = 0x0E;
        assert_eq!(parse_info(&r, BATTERY_FEATURE), None);
        assert_eq!(parse_battery(0x7F), None);
    }
}
