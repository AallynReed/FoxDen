// SPDX-License-Identifier: GPL-2.0-or-later

/// Baseboard manufacturer and product.
#[derive(Clone, Debug, Default)]
pub struct Dmi {
    pub manufacturer: String,
    pub mainboard: String,
}

pub fn read() -> Dmi {
    Dmi {
        manufacturer: bios_value("BaseBoardManufacturer").unwrap_or_default(),
        mainboard: bios_value("BaseBoardProduct").unwrap_or_default(),
    }
}

#[cfg(windows)]
fn bios_value(name: &str) -> Option<String> {
    use std::ffi::c_void;
    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegGetValueW(
            key: *mut c_void,
            sub_key: *const u16,
            value: *const u16,
            flags: u32,
            kind: *mut u32,
            data: *mut c_void,
            len: *mut u32,
        ) -> i32;
    }
    let hklm = 0x8000_0002u32 as i32 as isize as *mut c_void;
    let sub: Vec<u16> = "HARDWARE\\DESCRIPTION\\System\\BIOS\0"
        .encode_utf16()
        .collect();
    let value: Vec<u16> = name.encode_utf16().chain([0]).collect();
    let mut buf = [0u16; 256];
    let mut len = (buf.len() * 2) as u32;
    // SAFETY: both names are NUL-terminated and `len` is the byte size of `buf`.
    let status = unsafe {
        RegGetValueW(
            hklm,
            sub.as_ptr(),
            value.as_ptr(),
            0x2,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut len,
        )
    };
    if status != 0 {
        return None;
    }
    let n = (len as usize / 2).min(buf.len());
    let text = String::from_utf16_lossy(&buf[..n]);
    Some(text.trim_end_matches('\0').trim().to_string())
}

#[cfg(not(windows))]
fn bios_value(_name: &str) -> Option<String> {
    None
}
