//! What every USB HID driver family declares: which collections it drives and how it opens one.

use foxrgb_core::Backend;
use hidapi::{DeviceInfo, HidApi, HidDevice};

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
}

impl Info {
    pub fn from_hidapi(d: &DeviceInfo) -> Info {
        Info {
            vid: d.vendor_id(),
            pid: d.product_id(),
            interface: d.interface_number(),
            usage_page: d.usage_page(),
            usage: d.usage(),
            path: d.path().to_string_lossy().into_owned(),
            serial: d.serial_number().unwrap_or_default().to_string(),
            product: d.product_string().unwrap_or_default().trim().to_string(),
        }
    }

    /// Opens this collection.
    pub fn open(&self, api: &HidApi) -> Result<HidDevice, String> {
        let path = std::ffi::CString::new(self.path.clone()).map_err(|e| e.to_string())?;
        api.open_path(&path).map_err(|e| e.to_string())
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
