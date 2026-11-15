//! Native USB HID lighting drivers. Experimental: none of this has run against hardware yet.

mod families;
#[path = "families/generic/any/mod.rs"]
mod generic_any;
mod hidpp;
mod lengths;
mod msi;
pub mod port;
pub mod registry;

pub use families::{
    LedProtocol, LedStrip, UsbReports, led_strips, serial_ports, unihub_over_usb, winusb,
};
use foxrgb_core::Backend;
use hidapi::{BusType, HidApi};
pub use lengths::{Zone, key as length_key, resizable, set_lengths};

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
/// Opens that took longer than this are kept for the log.
const SLOW: std::time::Duration = std::time::Duration::from_millis(250);
static SLOW_OPENS: std::sync::Mutex<Vec<(String, u128)>> = std::sync::Mutex::new(Vec::new());

fn note_slow(what: &str, start: std::time::Instant) {
    let took = start.elapsed();
    if took >= SLOW {
        SLOW_OPENS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((what.to_string(), took.as_millis()));
    }
}

/// What made the last probe slow: each open over a quarter second, with its time in ms.
pub fn slow_opens() -> Vec<(String, u128)> {
    std::mem::take(&mut *SLOW_OPENS.lock().unwrap_or_else(|e| e.into_inner()))
}

pub fn probe() -> Vec<Box<dyn Backend>> {
    probe_with(&[])
}

/// Which devices a probe opens.
#[derive(Clone, Copy)]
pub enum Pick<'a> {
    All,
    /// Everything but these ids.
    Skip(&'a [(u16, u16)]),
    /// Only the fixed detectors of these ids: no generic matchers and no serial or WinUSB devices.
    Only(&'a [(u16, u16)]),
}

impl Pick<'_> {
    pub fn wants(&self, vid: u16, pid: u16) -> bool {
        match self {
            Pick::All => true,
            Pick::Skip(ids) => !ids.contains(&(vid, pid)),
            Pick::Only(ids) => ids.contains(&(vid, pid)),
        }
    }
}

/// Like `probe`, with `extra` detectors from other crates tried after the ported families'.
pub fn probe_with(extra: &[&'static registry::Detector]) -> Vec<Box<dyn Backend>> {
    probe_picked(extra, Pick::All)
}

/// Like `probe_with`, opening only what `pick` wants.
pub fn probe_picked(extra: &[&'static registry::Detector], pick: Pick) -> Vec<Box<dyn Backend>> {
    lengths::clear();
    SLOW_OPENS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    let Ok(api) = HidApi::new() else {
        return Vec::new();
    };
    let mut found: Vec<Box<dyn Backend>> = Vec::new();
    let mut board = false;
    let detectors = [families::detectors(), extra.to_vec()].concat();
    let mut taken: Vec<String> = Vec::new();
    let mut infos = Vec::new();
    for raw in api.device_list() {
        let info = registry::Info::from_hidapi(raw);
        if !pick.wants(info.vid, info.pid) {
            continue;
        }
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
        {
            let start = std::time::Instant::now();
            let shared = port::device_for(&info);
            let hold = shared.as_ref().map(|d| d.hold());
            let opened = (detector.open)(&api, &info, detector);
            drop(hold);
            note_slow(detector.name, start);
            if let Ok(backend) = opened {
                taken.push(key);
                found.push(match shared {
                    Some(d) => port::guard(backend, d),
                    None => backend,
                });
            }
        }
        infos.push(info);
    }
    if matches!(pick, Pick::Only(_)) {
        return found;
    }
    // Any-product matchers only see what the fixed detectors and hidpp.rs left.
    let mut claimed = Vec::new();
    found.extend(registry::sweep(
        &infos,
        &mut taken,
        &detectors,
        &[families::matchers(), generic_any::matchers()].concat(),
        |i| supported(i.vid, i.pid),
        |info, matcher| {
            let start = std::time::Instant::now();
            let opened = (matcher.open)(&api, info, &mut claimed);
            note_slow(
                &format!("{} {:04x}:{:04x}", matcher.name, info.vid, info.pid),
                start,
            );
            opened
        },
    ));
    let start = std::time::Instant::now();
    found.extend(families::other());
    note_slow("serial, WinUSB and SCSI devices", start);
    found
}

/// Every fixed detector of the ported families.
pub fn detectors() -> Vec<&'static registry::Detector> {
    families::detectors()
}

/// Vendor-wide and generic matchers of the ported families.
pub fn matchers() -> Vec<&'static registry::Matcher> {
    [families::matchers(), generic_any::matchers()].concat()
}

/// Devices driven outside the HID detectors: serial, WinUSB, the MSI board and Logitech HID++.
pub fn other_devices() -> Vec<(u16, u16, &'static str)> {
    let mut v = families::other_ids();
    v.push((msi::VID, msi::PID, "MSI Mystic Light"));
    for pid in [
        hidpp::G502_RECEIVER,
        hidpp::G515_RECEIVER,
        hidpp::G502_WIRED,
        hidpp::G515_WIRED,
    ] {
        v.push((hidpp::VID, pid, "Logitech Lightspeed"));
    }
    v
}

/// Names of every device the ported families support, for the detected-hardware list.
pub fn family_devices() -> Vec<(u16, u16, &'static str)> {
    families::detectors()
        .iter()
        .map(|d| (d.vid, d.pid, d.name))
        .collect()
}
