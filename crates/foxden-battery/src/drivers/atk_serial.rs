//! ATK N9 Pro and first-generation N9 Ultra headsets: asked over the dongle's USB COM port
//! (9600 8N1) with 64-byte frames `[2][cmd][sub]` -> `[len][cmd][sub][data...]`.

use std::time::{Duration, Instant};

use windows_sys::Win32::Devices::Communication::{
    COMMTIMEOUTS, DCB, GetCommState, NOPARITY, ONESTOPBIT, PURGE_RXCLEAR, PURGE_TXCLEAR, PurgeComm,
    SetCommState, SetCommTimeouts,
};
use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    DICS_FLAG_GLOBAL, DIGCF_PRESENT, DIREG_DEV, GUID_DEVCLASS_PORTS, SP_DEVINFO_DATA,
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiGetClassDevsW,
    SetupDiGetDeviceInstanceIdW, SetupDiOpenDevRegKey,
};
use windows_sys::Win32::Foundation::{
    CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{CreateFileW, OPEN_EXISTING, ReadFile, WriteFile};
use windows_sys::Win32::System::Registry::{HKEY, KEY_READ, REG_SZ, RegCloseKey, RegQueryValueExW};

use super::bt::nodes::{from_wide, wide};
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x373B;
const FRAME: usize = 64;
const STATUS: (u8, u8) = (0x00, 0x04);
const BATTERY: (u8, u8) = (0x00, 0x03);

/// (pid, name, wired)
const MODELS: [(u16, &str, bool); 4] = [
    (0x11CB, "ATK N9 Pro", false),
    (0x11CA, "ATK N9 Pro", true),
    (0x1204, "ATK N9 Ultra", false),
    (0x1203, "ATK N9 Ultra", true),
];

fn request((cmd, sub): (u8, u8)) -> [u8; FRAME] {
    let mut f = [0u8; FRAME];
    f[..3].copy_from_slice(&[2, cmd, sub]);
    f
}

/// Data bytes of the first reply frame for `(cmd, sub)`; other frames the headset pushes are skipped.
fn find_reply(buf: &[u8], (cmd, sub): (u8, u8)) -> Option<&[u8]> {
    buf.chunks_exact(FRAME)
        .find(|f| f[1] == cmd && f[2] == sub)
        .map(|f| &f[3..])
}

fn linked(data: &[u8]) -> bool {
    data[1] == 1
}

fn level(data: &[u8]) -> (u8, bool) {
    ((data[1] & 0x7F).min(100), data[1] & 0x80 != 0)
}

/// COM port names of the supported headsets.
fn ports() -> Vec<(String, (u16, &'static str, bool))> {
    let set = unsafe {
        SetupDiGetClassDevsW(
            &GUID_DEVCLASS_PORTS,
            std::ptr::null(),
            std::ptr::null_mut(),
            DIGCF_PRESENT,
        )
    };
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
        if unsafe {
            SetupDiGetDeviceInstanceIdW(
                set,
                &dd,
                id.as_mut_ptr(),
                id.len() as u32,
                std::ptr::null_mut(),
            )
        } == 0
        {
            continue;
        }
        let id = from_wide(&id).to_uppercase();
        let Some(model) = MODELS
            .iter()
            .find(|m| id.contains(&format!("VID_{VID:04X}&PID_{:04X}", m.0)))
        else {
            continue;
        };
        let key: HKEY =
            unsafe { SetupDiOpenDevRegKey(set, &dd, DICS_FLAG_GLOBAL, 0, DIREG_DEV, KEY_READ) };
        if key as isize == -1 || key.is_null() {
            continue;
        }
        let mut name = [0u16; 32];
        let mut size = ((name.len() - 1) * 2) as u32;
        let mut ty = 0;
        let name_w = wide("PortName");
        let r = unsafe {
            RegQueryValueExW(
                key,
                name_w.as_ptr(),
                std::ptr::null(),
                &mut ty,
                name.as_mut_ptr() as *mut u8,
                &mut size,
            )
        };
        unsafe { RegCloseKey(key) };
        if r == 0 && ty == REG_SZ {
            out.push((from_wide(&name), *model));
        }
    }
    unsafe { SetupDiDestroyDeviceInfoList(set) };
    out
}

struct Port(HANDLE);

impl Drop for Port {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

impl Port {
    /// Fails while the vendor app holds the port.
    fn open(name: &str) -> Option<Self> {
        let path = wide(&format!("\\\\.\\{name}"));
        let h = unsafe {
            CreateFileW(
                path.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };
        if h == INVALID_HANDLE_VALUE {
            return None;
        }
        let port = Port(h);
        let mut dcb: DCB = unsafe { std::mem::zeroed() };
        dcb.DCBlength = size_of::<DCB>() as u32;
        if unsafe { GetCommState(h, &mut dcb) } == 0 {
            return None;
        }
        dcb.BaudRate = 9600;
        dcb.ByteSize = 8;
        dcb.Parity = NOPARITY;
        dcb.StopBits = ONESTOPBIT;
        // Binary mode, DTR and RTS on, no flow control.
        dcb._bitfield = 0x0001 | (1 << 4) | (1 << 12);
        let to = COMMTIMEOUTS {
            ReadIntervalTimeout: 50,
            ReadTotalTimeoutMultiplier: 0,
            ReadTotalTimeoutConstant: 300,
            WriteTotalTimeoutMultiplier: 0,
            WriteTotalTimeoutConstant: 300,
        };
        if unsafe { SetCommState(h, &dcb) } == 0 || unsafe { SetCommTimeouts(h, &to) } == 0 {
            return None;
        }
        unsafe { PurgeComm(h, PURGE_RXCLEAR | PURGE_TXCLEAR) };
        Some(port)
    }

    fn ask(&self, cmd: (u8, u8)) -> Option<Vec<u8>> {
        let req = request(cmd);
        let mut n = 0;
        if unsafe {
            WriteFile(
                self.0,
                req.as_ptr(),
                FRAME as u32,
                &mut n,
                std::ptr::null_mut(),
            )
        } == 0
            || n as usize != FRAME
        {
            return None;
        }
        let end = Instant::now() + Duration::from_millis(800);
        let mut buf = Vec::new();
        while Instant::now() < end {
            let mut chunk = [0u8; 128];
            if unsafe {
                ReadFile(
                    self.0,
                    chunk.as_mut_ptr(),
                    chunk.len() as u32,
                    &mut n,
                    std::ptr::null_mut(),
                )
            } == 0
            {
                return None;
            }
            buf.extend_from_slice(&chunk[..(n as usize).min(chunk.len())]);
            if buf.len() > 4096 {
                return None;
            }
            if let Some(d) = find_reply(&buf, cmd) {
                return Some(d.to_vec());
            }
        }
        None
    }
}

pub fn poll_atk_serial(ctx: &mut Ctx) {
    for (name, (pid, model, wired)) in ports() {
        if ctx.is_claimed(VID, pid) {
            continue;
        }
        let Some(port) = Port::open(&name) else {
            continue;
        };
        let Some(status) = port.ask(STATUS) else {
            continue;
        };
        let mut b = Battery {
            key: ctx.stable_key(&format!("atk-{VID:04x}:{pid:04x}")),
            name: model.into(),
            via: if wired {
                "USB cable"
            } else {
                "2.4 GHz receiver"
            }
            .into(),
            kind: Kind::Headset,
            connected: linked(&status),
            ..Default::default()
        };
        if b.connected
            && let Some(d) = port.ask(BATTERY)
        {
            let (pct, chg) = level(&d);
            b.percent = Some(pct);
            b.charging = chg;
        }
        ctx.add(b, VID, pid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(cmd: (u8, u8), value: u8) -> Vec<u8> {
        let mut f = vec![0u8; FRAME];
        f[..5].copy_from_slice(&[3, cmd.0, cmd.1, 0, value]);
        f
    }

    #[test]
    fn replies_are_matched_by_command() {
        assert_eq!(&request(BATTERY)[..4], &[2, 0, 3, 0]);
        let mut s = reply((0x01, 0x07), 0xFF);
        s.extend(reply(BATTERY, 0x80 | 70));
        let d = find_reply(&s, BATTERY).unwrap();
        assert_eq!(level(d), (70, true));
        assert!(linked(find_reply(&reply(STATUS, 1), STATUS).unwrap()));
        assert!(find_reply(&s[..100], BATTERY).is_none());
    }
}
