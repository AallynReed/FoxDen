//! What every USB HID driver family declares: which collections it drives and how it opens one.

use foxrgb_core::Backend;
use hidapi::{HidApi, HidDevice};

/// How a collection is attached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bus {
    Usb,
    Bluetooth,
    Other,
}

/// One HID collection as the system lists it, with what a driver needs to open it again.
#[derive(Clone, Debug)]
pub struct Info {
    pub vid: u16,
    pub pid: u16,
    pub interface: i32,
    pub usage_page: u16,
    pub usage: u16,
    pub path: String,
    pub serial: String,
    pub product: String,
    pub bus: Bus,
    pub release: u16,
}

impl Info {
    pub fn from_hidapi(d: &crate::scan::Dev) -> Info {
        d.info().clone()
    }

    /// Opens this collection.
    pub fn open(&self, api: &HidApi) -> Result<HidDevice, String> {
        let path = std::ffi::CString::new(self.path.clone()).map_err(|e| e.to_string())?;
        api.open_path(&path).map_err(|e| e.to_string())
    }

    /// Opens this collection, or reuses the handle another driver of a shared device holds.
    pub fn port(&self, api: &HidApi) -> Result<crate::port::Port, String> {
        crate::port::Port::open(self, || Ok(Box::new(self.open(api)?)))
    }

    /// One physical device: collections of the same device share it.
    pub fn key(&self) -> String {
        format!("{:04x}{:04x}{}", self.vid, self.pid, self.serial)
    }

    /// A device id that stays the same across runs and tells two identical devices apart.
    pub fn device_id(&self, family: &str) -> String {
        let tail = if self.serial.is_empty() {
            format!("{:08x}", fnv(self.path.as_bytes()))
        } else {
            self.serial.clone()
        };
        format!("hid:{family}-{:04x}{:04x}-{tail}", self.vid, self.pid)
    }
}

fn fnv(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811c_9dc5u32, |h, b| {
        (h ^ *b as u32).wrapping_mul(0x0100_0193)
    })
}

/// Opens a matching collection. `api` lets a driver open sibling collections of the same device.
pub type Open = fn(&HidApi, &Info, &Detector) -> Result<Box<dyn Backend>, String>;

/// One supported device, as an OpenRGB `REGISTER_HID_DETECTOR*` line declares it.
pub struct Detector {
    pub name: &'static str,
    pub vid: u16,
    pub pid: u16,
    pub interface: Option<i32>,
    pub usage_page: Option<u16>,
    pub usage: Option<u16>,
    pub open: Open,
}

impl Detector {
    pub fn matches(&self, info: &Info) -> bool {
        self.vid == info.vid
            && self.pid == info.pid
            && self.interface.is_none_or(|i| i == info.interface)
            && self.usage_page.is_none_or(|p| p == info.usage_page)
            && self.usage.is_none_or(|u| u == info.usage)
    }
}

/// Opens every device a collection reaches. `claimed` holds identities already built this pass.
pub type OpenAny = fn(&HidApi, &Info, &mut Vec<String>) -> Vec<Box<dyn Backend>>;

/// An OpenRGB detector registered with `HID_PID_ANY`: any product of `vid`, or of any vendor when `vid` is `None`.
pub struct Matcher {
    pub name: &'static str,
    pub vid: Option<u16>,
    pub interface: Option<i32>,
    pub usage_page: Option<u16>,
    pub usage: Option<u16>,
    pub bus: Option<Bus>,
    pub open: OpenAny,
}

impl Matcher {
    pub fn matches(&self, info: &Info) -> bool {
        self.vid.is_none_or(|v| v == info.vid)
            && self.interface.is_none_or(|i| i == info.interface)
            && self.usage_page.is_none_or(|p| p == info.usage_page)
            && self.usage.is_none_or(|u| u == info.usage)
            && self.bus.is_none_or(|b| b == info.bus)
    }
}

/// Runs `matchers` over collections the fixed detectors left, the way OpenRGB's DetectionManager
/// orders them: a vendor-wide matcher before a generic one, and no generic matcher on a vid/pid
/// any specific detector claims. `skip` filters collections other drivers own.
pub fn sweep<T>(
    infos: &[Info],
    taken: &mut Vec<String>,
    fixed: &[&Detector],
    matchers: &[&Matcher],
    skip: impl Fn(&Info) -> bool,
    mut open: impl FnMut(&Info, &Matcher) -> Vec<T>,
) -> Vec<T> {
    let mut found = Vec::new();
    for info in infos {
        if !matches!(info.bus, Bus::Usb | Bus::Bluetooth) || skip(info) {
            continue;
        }
        let specific = fixed.iter().any(|d| d.vid == info.vid && d.pid == info.pid)
            || matchers.iter().any(|m| m.vid == Some(info.vid));
        let ordered = matchers
            .iter()
            .filter(|m| m.vid.is_some())
            .chain(matchers.iter().filter(|m| m.vid.is_none() && !specific));
        for matcher in ordered {
            if taken.contains(&info.key()) {
                break;
            }
            if !matcher.matches(info) {
                continue;
            }
            let built = open(info, matcher);
            if !built.is_empty() {
                taken.push(info.key());
                found.extend(built);
            }
            break;
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(vid: u16, pid: u16, page: u16, usage: u16, bus: Bus) -> Info {
        Info {
            vid,
            pid,
            interface: 0,
            usage_page: page,
            usage,
            path: format!("{vid:04x}{pid:04x}{page:04x}{usage}"),
            serial: String::new(),
            product: String::new(),
            bus,
            release: 0,
        }
    }

    fn no_open(_: &HidApi, _: &Info, _: &Detector) -> Result<Box<dyn Backend>, String> {
        Err("unused".into())
    }

    fn no_any(_: &HidApi, _: &Info, _: &mut Vec<String>) -> Vec<Box<dyn Backend>> {
        Vec::new()
    }

    const LAMP: Matcher = Matcher {
        name: "lamp",
        vid: None,
        interface: None,
        usage_page: Some(0x59),
        usage: Some(0x01),
        bus: None,
        open: no_any,
    };
    const LOGI: Matcher = Matcher {
        name: "logi",
        vid: Some(0x046D),
        interface: None,
        usage_page: Some(0xFF00),
        usage: Some(2),
        bus: Some(Bus::Usb),
        open: no_any,
    };
    const FIXED: Detector = Detector {
        name: "fixed",
        vid: 0x1234,
        pid: 0x0001,
        interface: None,
        usage_page: None,
        usage: None,
        open: no_open,
    };

    // DetectionManager::DetectHIDDevices: specific detectors first, generic ones only on unclaimed vid/pid.
    #[test]
    fn sweep_follows_openrgb_order() {
        let infos = [
            info(0x1234, 0x0001, 0x59, 1, Bus::Usb),
            info(0x1234, 0x0002, 0x59, 1, Bus::Usb),
            info(0x046D, 0xC000, 0x59, 1, Bus::Usb),
            info(0x046D, 0xC001, 0xFF00, 2, Bus::Usb),
            info(0x046D, 0xC001, 0xFF00, 2, Bus::Bluetooth),
            info(0x2222, 0x0003, 0x59, 1, Bus::Other),
            info(0x3333, 0x0004, 0x59, 1, Bus::Bluetooth),
        ];
        let mut taken = vec![info(0x3333, 0x0004, 0, 0, Bus::Usb).key()];
        let mut calls = Vec::new();
        let out = sweep(
            &infos,
            &mut taken,
            &[&FIXED],
            &[&LOGI, &LAMP],
            |_| false,
            |i, m| {
                calls.push((i.pid, m.name));
                vec![i.pid]
            },
        );
        assert_eq!(calls, [(0x0002, "lamp"), (0xC001, "logi")]);
        assert_eq!(out, [0x0002, 0xC001]);
        assert!(taken.contains(&infos[3].key()));
    }

    // One backend per physical device; an empty result leaves the device to its next collection.
    #[test]
    fn sweep_never_takes_a_device_twice() {
        let a = info(0x046D, 0xC001, 0xFF00, 2, Bus::Usb);
        let mut b = a.clone();
        b.path.push('b');
        let fixed_took = info(0x046D, 0xC002, 0xFF00, 2, Bus::Usb);
        let skipped = info(0x046D, 0xC539, 0xFF00, 2, Bus::Usb);
        let mut taken = vec![fixed_took.key()];
        let mut tries = 0;
        let out = sweep(
            &[a.clone(), b, fixed_took, skipped],
            &mut taken,
            &[],
            &[&LOGI],
            |i| i.pid == 0xC539,
            |_, _| {
                tries += 1;
                if tries == 1 {
                    Vec::<u8>::new()
                } else {
                    vec![1]
                }
            },
        );
        assert_eq!((tries, out.len()), (2, 1));
        let mut again = Vec::new();
        let out = sweep(
            &[a.clone(), a],
            &mut again,
            &[],
            &[&LOGI],
            |_| false,
            |_, _| vec![1],
        );
        assert_eq!(out.len(), 1);
    }
}
