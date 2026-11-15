//! Native USB HID lighting drivers. Experimental: none of this has run against hardware yet.

mod families;
#[path = "families/generic/any/mod.rs"]
mod generic_any;
mod hidpp;
mod msi;
pub mod registry;

use foxrgb_core::Backend;
use hidapi::{BusType, HidApi};

fn supported(vid: u16, pid: u16) -> bool {
    matches!(
        (vid, pid),
        (msi::VID, msi::PID)
            | (
                hidpp::VID,
                hidpp::G502_RECEIVER | hidpp::G515_RECEIVER | hidpp::G502_WIRED | hidpp::G515_WIRED
            )
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

/// One collection of a device, for a device report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Collection {
    pub interface: i32,
    pub usage_page: u16,
    pub usage: u16,
    pub release: u16,
    pub name: String,
    pub manufacturer: String,
    /// The report descriptor, or why Windows would not give it.
    pub descriptor: Result<Vec<u8>, String>,
}

/// Every collection of one device with its report descriptor. Opens each one, writes nothing.
pub fn describe(vendor: u16, product: u16) -> Vec<Collection> {
    let Ok(api) = HidApi::new() else {
        return Vec::new();
    };
    let mut out: Vec<Collection> = api
        .device_list()
        .filter(|d| (d.vendor_id(), d.product_id()) == (vendor, product))
        .map(|d| {
            let descriptor = d
                .open_device(&api)
                .and_then(|dev| {
                    let mut buf = [0u8; 4096];
                    let n = dev.get_report_descriptor(&mut buf)?;
                    Ok(buf[..n].to_vec())
                })
                .map_err(|e| e.to_string());
            Collection {
                interface: d.interface_number(),
                usage_page: d.usage_page(),
                usage: d.usage(),
                release: d.release_number(),
                name: d.product_string().unwrap_or_default().trim().to_string(),
                manufacturer: d
                    .manufacturer_string()
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
                descriptor,
            }
        })
        .collect();
    out.sort_by_key(|c| (c.interface, c.usage_page, c.usage));
    out
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
    let detectors = families::detectors();
    let mut taken: Vec<String> = Vec::new();
    let mut infos = Vec::new();
    for raw in api.device_list() {
        let info = registry::Info::from_hidapi(raw);
        match (info.vid, info.pid) {
            (msi::VID, msi::PID) if !board => {
                if let Ok(hid) = raw.open_device(&api)
                    && let Ok(device) = msi::Board::open(hid)
                {
                    found.push(Box::new(device));
                    board = true;
                }
                continue;
            }
            (
                hidpp::VID,
                pid @ (hidpp::G502_RECEIVER
                | hidpp::G515_RECEIVER
                | hidpp::G502_WIRED
                | hidpp::G515_WIRED),
            ) => {
                if hidpp::is_long_channel(info.usage_page, info.usage)
                    && let Ok(hid) = raw.open_device(&api)
                    && let Ok(device) = hidpp::Lightspeed::open(hid, pid)
                {
                    found.push(Box::new(device));
                }
                continue;
            }
            _ => {}
        }
        // Virtual devices, such as G HUB's keyboard, can share a real device's ids.
        if !matches!(info.bus, registry::Bus::Usb | registry::Bus::Bluetooth) {
            continue;
        }
        // One backend per physical device: the first matching detector wins.
        let key = info.key();
        if let Some(detector) = detectors.iter().find(|d| d.matches(&info))
            && !taken.contains(&key)
            && let Ok(backend) = (detector.open)(&api, &info, detector)
        {
            taken.push(key);
            found.push(backend);
        }
        infos.push(info);
    }
    // Any-product matchers only see what the fixed detectors and hidpp.rs left.
    let mut claimed = Vec::new();
    found.extend(registry::sweep(
        &infos,
        &mut taken,
        &detectors,
        &[families::matchers(), generic_any::matchers()].concat(),
        |i| supported(i.vid, i.pid),
        |info, matcher| (matcher.open)(&api, info, &mut claimed),
    ));
    found
}

/// Names of every device the ported families support, for the detected-hardware list.
pub fn family_devices() -> Vec<(u16, u16, &'static str)> {
    families::detectors()
        .iter()
        .map(|d| (d.vid, d.pid, d.name))
        .collect()
}
