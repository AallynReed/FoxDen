//! Native USB HID lighting drivers. Experimental: none of this has run against hardware yet.

mod hidpp;
mod msi;

use foxrgb_core::Backend;
use hidapi::{BusType, HidApi};

fn supported(vid: u16, pid: u16) -> bool {
    matches!(
        (vid, pid),
        (msi::VID, msi::PID) | (hidpp::VID, hidpp::G502_RECEIVER | hidpp::G515_RECEIVER)
    )
}

/// One HID collection as the system lists it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HidInfo {
    pub vendor: u16,
    pub product: u16,
    pub name: String,
    pub manufacturer: String,
    pub usage_page: u16,
    pub usage: u16,
    /// Plugged in over USB, not Bluetooth or a virtual bus.
    pub usb: bool,
}

/// Every HID collection on the system, read from its device list without opening any.
pub fn enumerate() -> Vec<HidInfo> {
    let Ok(api) = HidApi::new() else {
        return Vec::new();
    };
    api.device_list()
        .map(|d| HidInfo {
            vendor: d.vendor_id(),
            product: d.product_id(),
            name: d.product_string().unwrap_or_default().trim().to_string(),
            manufacturer: d
                .manufacturer_string()
                .unwrap_or_default()
                .trim()
                .to_string(),
            usage_page: d.usage_page(),
            usage: d.usage(),
            usb: matches!(d.bus_type(), BusType::Usb),
        })
        .collect()
}

/// HID collections of the supported devices, read from the system's device list without opening any.
pub fn list() -> Vec<String> {
    let Ok(api) = HidApi::new() else {
        return Vec::new();
    };
    api.device_list()
        .filter(|d| supported(d.vendor_id(), d.product_id()))
        .map(|d| {
            format!(
                "{:04x}:{:04x} usage {:04x}:{:04x} interface {}",
                d.vendor_id(),
                d.product_id(),
                d.usage_page(),
                d.usage(),
                d.interface_number()
            )
        })
        .collect()
}

/// What happens when each supported collection is opened, for diagnosis.
pub fn diagnose() -> Vec<String> {
    let Ok(api) = HidApi::new() else {
        return vec!["hidapi failed".into()];
    };
    let mut out = Vec::new();
    for info in api
        .device_list()
        .filter(|d| supported(d.vendor_id(), d.product_id()))
    {
        let what = format!(
            "{:04x}:{:04x} {:04x}:{:04x} if{}",
            info.vendor_id(),
            info.product_id(),
            info.usage_page(),
            info.usage(),
            info.interface_number()
        );
        let result = match info.open_device(&api) {
            Err(e) => format!("open failed: {e}"),
            Ok(hid)
                if info.vendor_id() == hidpp::VID
                    && hidpp::is_long_channel(info.usage_page(), info.usage()) =>
            {
                match hidpp::Lightspeed::open(hid, info.product_id()) {
                    Ok(_) => "hid++ ok".into(),
                    Err(e) => format!("hid++ failed: {e}"),
                }
            }
            Ok(_) => "opened".into(),
        };
        out.push(format!("{what}: {result}"));
    }
    out
}

/// Detects supported USB HID lighting devices and returns one backend per device.
pub fn probe() -> Vec<Box<dyn Backend>> {
    let Ok(api) = HidApi::new() else {
        return Vec::new();
    };
    let mut found: Vec<Box<dyn Backend>> = Vec::new();
    let mut board = false;
    for info in api.device_list() {
        match (info.vendor_id(), info.product_id()) {
            (msi::VID, msi::PID) if !board => {
                if let Ok(hid) = info.open_device(&api)
                    && let Ok(device) = msi::Board::open(hid)
                {
                    found.push(Box::new(device));
                    board = true;
                }
            }
            (hidpp::VID, pid @ (hidpp::G502_RECEIVER | hidpp::G515_RECEIVER))
                if hidpp::is_long_channel(info.usage_page(), info.usage()) =>
            {
                if let Ok(hid) = info.open_device(&api)
                    && let Ok(device) = hidpp::Lightspeed::open(hid, pid)
                {
                    found.push(Box::new(device));
                }
            }
            _ => {}
        }
    }
    found
}
