// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Backend, Color, Device, DeviceKind};

const DEVS: u32 = 0x5356_4544;
const DSTS: u32 = 0x5354_5344;
const KBD_BACKLIGHT: u32 = 0x0005_0021;
const TUF_RGB_MODE: u32 = 0x0010_0056;
const MAGIC_USE: u32 = 0xB3;
const MODE_STATIC: u32 = 0x00;
const SPEED_NORMAL: u32 = 0xEB;
const DEVICE_ID: &str = "acpi:asus-tuf-keyboard";

/// The ATKACPI DeviceIoControl, so tests can record it.
pub trait Acpi: Send {
    /// Sends `input`, returns the answer.
    fn ioctl(&mut self, input: &[u8]) -> Result<Vec<u8>, String>;
}

/// deviceIoControlWrapper's input: command, payload size, payload.
pub fn wrap(command: u32, args: &[u32]) -> Vec<u8> {
    let mut b = command.to_le_bytes().to_vec();
    b.extend(((args.len() * 4) as u32).to_le_bytes());
    for a in args {
        b.extend(a.to_le_bytes());
    }
    b
}

/// setMode in Static with the default speed, not saved.
pub fn set_mode(c: Color) -> Vec<u8> {
    let (r, g, b) = (c.0 as u32, c.1 as u32, c.2 as u32);
    let high = MAGIC_USE | (MODE_STATIC << 8) | (r << 16) | (g << 24);
    let low = b | (SPEED_NORMAL << 8);
    wrap(DEVS, &[TUF_RGB_MODE, high, low])
}

pub struct Tuf<A: Acpi> {
    acpi: A,
    device: Device,
    brightness: u8,
    started: bool,
    sent: Option<Color>,
}

impl<A: Acpi> Tuf<A> {
    /// ReadConfiguration: Static starts at the keyboard's current backlight level.
    pub fn new(mut acpi: A) -> Self {
        let status = acpi
            .ioctl(&wrap(DSTS, &[KBD_BACKLIGHT]))
            .ok()
            .filter(|r| r.len() >= 4)
            .map_or(0, |r| u32::from_le_bytes([r[0], r[1], r[2], r[3]]));
        let device = Device::grid(
            DEVICE_ID,
            "ASUS TUF Laptop Keyboard",
            DeviceKind::Keyboard,
            [0.06, 0.58, 0.54, 0.32],
            1,
            1,
        );
        Tuf {
            acpi,
            device,
            brightness: (status & 0x7F) as u8,
            started: false,
            sent: None,
        }
    }
}

impl<A: Acpi> Backend for Tuf<A> {
    fn name(&self) -> &str {
        "asus tuf laptop"
    }
    fn devices(&self) -> Vec<Device> {
        vec![self.device.clone()]
    }
    fn push(&mut self, device_id: &str, colors: &[Color]) -> Result<(), String> {
        if device_id != DEVICE_ID {
            return Err(format!("unknown device {device_id}"));
        }
        let c = colors.first().copied().unwrap_or(Color::BLACK);
        if self.sent == Some(c) {
            return Ok(());
        }
        if !self.started {
            // DeviceUpdateMode: setBrightness before the colour
            let level = 0x80 | (self.brightness as u32 & 0x7F);
            self.acpi.ioctl(&wrap(DEVS, &[KBD_BACKLIGHT, level]))?;
            self.started = true;
        }
        self.sent = None;
        self.acpi.ioctl(&set_mode(c))?;
        self.sent = Some(c);
        Ok(())
    }
}

#[cfg(windows)]
mod win {
    use super::Acpi;
    use std::ffi::c_void;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateFileW(
            name: *const u16,
            access: u32,
            share: u32,
            security: *mut c_void,
            disposition: u32,
            flags: u32,
            template: *mut c_void,
        ) -> *mut c_void;
        fn DeviceIoControl(
            device: *mut c_void,
            code: u32,
            input: *const c_void,
            input_len: u32,
            output: *mut c_void,
            output_len: u32,
            returned: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegOpenKeyExW(
            key: *mut c_void,
            sub: *const u16,
            options: u32,
            access: u32,
            out: *mut *mut c_void,
        ) -> i32;
        fn RegCloseKey(key: *mut c_void) -> i32;
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain([0]).collect()
    }

    /// The ACPI\PNP0C14\ATK device checkWMIType looks for.
    fn atk_present() -> bool {
        let hklm = 0x8000_0002u32 as i32 as isize as *mut c_void;
        let sub = wide("SYSTEM\\CurrentControlSet\\Enum\\ACPI\\PNP0C14\\ATK");
        let mut key = std::ptr::null_mut();
        // SAFETY: `sub` is NUL-terminated and `key` receives the handle we close below.
        let ok = unsafe { RegOpenKeyExW(hklm, sub.as_ptr(), 0, 0x2_0019, &mut key) } == 0;
        if ok {
            // SAFETY: `key` was opened above.
            unsafe { RegCloseKey(key) };
        }
        ok
    }

    pub struct Atk(*mut c_void);

    // SAFETY: the handle is only used through `&mut self`.
    unsafe impl Send for Atk {}

    impl Atk {
        pub fn open() -> Option<Atk> {
            if !atk_present() {
                return None;
            }
            let name = wide("\\\\.\\ATKACPI");
            // SAFETY: `name` is NUL-terminated; the rest are plain values.
            let h = unsafe {
                CreateFileW(
                    name.as_ptr(),
                    0xC000_0000,
                    3,
                    std::ptr::null_mut(),
                    3,
                    0,
                    std::ptr::null_mut(),
                )
            };
            (h as isize != -1 && !h.is_null()).then_some(Atk(h))
        }
    }

    impl Acpi for Atk {
        fn ioctl(&mut self, input: &[u8]) -> Result<Vec<u8>, String> {
            let mut out = vec![0u8; 1024];
            let mut returned = 0u32;
            // SAFETY: both buffers live across the call and their lengths are passed with them.
            let ok = unsafe {
                DeviceIoControl(
                    self.0,
                    0x0022_240C,
                    input.as_ptr().cast(),
                    input.len() as u32,
                    out.as_mut_ptr().cast(),
                    out.len() as u32,
                    &mut returned,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err("ATKACPI request failed".into());
            }
            out.truncate(returned as usize);
            Ok(out)
        }
    }

    impl Drop for Atk {
        fn drop(&mut self) {
            // SAFETY: the handle came from CreateFileW and is closed once.
            unsafe { CloseHandle(self.0) };
        }
    }
}

/// DetectAsusTUFLaptopWMIControllers: a "TUF Gaming" system with the ATK ACPI device.
#[cfg(windows)]
pub fn probe() -> Option<Box<dyn Backend>> {
    let name = super::dmi::bios("SystemProductName")?;
    if !name.contains("TUF Gaming") {
        return None;
    }
    let atk = win::Atk::open()?;
    Some(Box::new(Tuf::new(atk)))
}

#[cfg(not(windows))]
pub fn probe() -> Option<Box<dyn Backend>> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Log(Arc<Mutex<Vec<Vec<u8>>>>);

    impl Acpi for Log {
        fn ioctl(&mut self, input: &[u8]) -> Result<Vec<u8>, String> {
            self.0.lock().unwrap().push(input.to_vec());
            Ok(vec![0x02, 0, 0, 0])
        }
    }

    fn words(b: &[u8]) -> Vec<u32> {
        b.chunks(4)
            .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
            .collect()
    }

    // ReadConfiguration, DeviceUpdateMode(Static), then setMode per colour
    #[test]
    fn brightness_then_mode() {
        let log = Log::default();
        let mut t = Tuf::new(log.clone());
        t.push(DEVICE_ID, &[Color(0x11, 0x22, 0x33)]).unwrap();
        t.push(DEVICE_ID, &[Color(0x11, 0x22, 0x33)]).unwrap();
        let sent: Vec<Vec<u32>> = log.0.lock().unwrap().iter().map(|b| words(b)).collect();
        assert_eq!(
            sent,
            vec![
                vec![DSTS, 4, 0x0005_0021],
                vec![DEVS, 8, 0x0005_0021, 0x82],
                vec![DEVS, 12, 0x0010_0056, 0x2211_00B3, 0xEB33],
            ]
        );
    }
}
