//! Native USB HID lighting drivers.

mod families;
#[path = "families/generic/any/mod.rs"]
mod generic_any;
mod hidpp;
mod lengths;
mod msi;
pub mod port;
pub mod registry;
mod scan;

pub use families::{
    LedProtocol, LedStrip, UsbReports, led_strips, serial_ports, unihub_over_usb, winusb,
};
use foxden_core::Backend;
use hidapi::HidApi;
pub use lengths::{
    Zone, key as length_key, length as zone_length, length_step as zone_length_step, resizable,
    set_lengths,
};

fn supported(vid: u16, pid: u16) -> bool {
    (vid, pid) == (msi::VID, msi::PID) || (vid == hidpp::VID && hidpp::PIDS.contains(&pid))
}

static LISTING: std::sync::Mutex<Vec<scan::Dev>> = std::sync::Mutex::new(Vec::new());

/// An hidapi context that never lists devices itself: hidapi asks every device for its strings,
/// and one that never answers costs 5 s a string. The collections come from `device_list`.
pub fn hid_api(want: impl Fn(&registry::Info) -> bool) -> Option<HidApi> {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(HidApi::disable_device_discovery);
    let api = HidApi::new().ok()?;
    let found: Vec<scan::Dev> = scan::with_serials(std::time::Duration::from_millis(500))
        .into_iter()
        .filter(|c| want(c))
        .map(scan::Dev::new)
        .collect();
    *LISTING.lock().unwrap_or_else(|e| e.into_inner()) = found;
    Some(api)
}

/// The collections the last `hid_api` call picked, in place of hidapi's own list.
pub fn device_list(_api: &HidApi) -> Vec<scan::Dev> {
    LISTING.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

pub use scan::Dev;

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

/// Every HID collection on the system, from Windows' records: no device is asked anything.
pub fn enumerate() -> Vec<HidInfo> {
    scan::collections()
        .into_iter()
        .map(|c| HidInfo {
            vendor: c.vid,
            product: c.pid,
            name: c.product,
            manufacturer: String::new(),
            usage_page: c.usage_page,
            usage: c.usage,
            usb: c.bus == registry::Bus::Usb,
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
    let Some(api) = hid_api(|c| (c.vid, c.pid) == (vendor, product)) else {
        return Vec::new();
    };
    let mut out: Vec<Collection> = crate::device_list(&api).iter()
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
    let Some(api) = hid_api(|c| supported(c.vid, c.pid)) else {
        return Vec::new();
    };
    crate::device_list(&api).iter()
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
    let Some(api) = hid_api(|c| supported(c.vid, c.pid)) else {
        return vec!["hidapi failed".into()];
    };
    let mut out = Vec::new();
    for info in crate::device_list(&api).iter()
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
                drop(hid);
                let opened = hidpp_devices(&api, &registry::Info::from_hidapi(info));
                let failed: Vec<String> = opened
                    .iter()
                    .filter_map(|o| o.as_ref().err().cloned())
                    .collect();
                match (opened.len() - failed.len(), failed.is_empty()) {
                    (0, true) => "hid++: nothing paired".into(),
                    (n, true) => format!("hid++ ok, {n} device(s)"),
                    (n, false) => format!("hid++ {n} ok, failed: {}", failed.join("; ")),
                }
            }
            Ok(_) => "opened".into(),
        };
        out.push(format!("{what}: {result}"));
    }
    out
}

/// A receiver's pairing table, asked over the short HID++ collection beside `info`; `None` when it is no receiver.
fn pairing(api: &HidApi, info: &registry::Info) -> Option<Vec<(u8, hidpp::Paired)>> {
    let key = generic_any::receiver::path_key(&info.path);
    let short = crate::device_list(api)
        .iter()
        .find(|d| {
            d.vendor_id() == info.vid
                && d.product_id() == info.pid
                && d.usage_page() == 0xFF00
                && d.usage() == 0x0001
                && generic_any::receiver::path_key(&d.path().to_string_lossy()) == key
        })?
        .open_device(api)
        .ok()?;
    let long = info.open(api).ok()?;
    let slots = hidpp::pairing(|n| hidpp::read_slot(&short, &long, n))?;
    Some(
        slots
            .into_iter()
            .map(|(n, wpid, kind)| {
                let name = hidpp::read_name(&short, &long, n);
                (n, hidpp::Paired { wpid, kind, name })
            })
            .collect(),
    )
}

/// Every device a Logitech HID++ long collection reaches, or why each could not be driven.
fn hidpp_devices(api: &HidApi, info: &registry::Info) -> Vec<Result<hidpp::Lightspeed, String>> {
    let slots = if hidpp::cable(info.pid) {
        None
    } else {
        pairing(api, info)
    };
    hidpp::targets(info.pid, &info.product, slots)
        .into_iter()
        .map(|target| hidpp::Lightspeed::open(info.open(api)?, target))
        .collect()
}

/// Logitech devices the last probe drove itself, by USB product id and device id.
static HIDPP_ROUTES: std::sync::Mutex<Vec<(u16, String)>> = std::sync::Mutex::new(Vec::new());

/// Whether the last probe drove `device_id` through the Logitech USB device `pid`.
pub fn logitech_drives(pid: u16, device_id: &str) -> bool {
    HIDPP_ROUTES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .any(|(p, id)| *p == pid && id == device_id)
}

/// Which HID++ devices to keep: one per device id, the cable one when a device shows on both.
fn keep_hidpp(devices: &[(&str, bool)]) -> Vec<bool> {
    devices
        .iter()
        .enumerate()
        .map(|(i, &(id, wired))| {
            let first = devices.iter().position(|d| d.0 == id && d.1) == Some(i);
            let unique = devices.iter().position(|d| d.0 == id) == Some(i);
            if wired {
                first
            } else {
                unique && !devices.iter().any(|d| d.0 == id && d.1)
            }
        })
        .collect()
}

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

/// Detects supported USB HID lighting devices and returns one backend per device, opening only
/// what `pick` wants. `extra` detectors from other crates are tried after the ported families'.
pub fn probe_picked(extra: &[&'static registry::Detector], pick: Pick) -> Vec<Box<dyn Backend>> {
    lengths::clear();
    SLOW_OPENS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    let detectors = [families::detectors(), extra.to_vec()].concat();
    let matchers = [families::matchers(), generic_any::matchers()].concat();
    let wanted = |c: &registry::Info| {
        pick.wants(c.vid, c.pid)
            && (supported(c.vid, c.pid)
                || (c.vid == hidpp::VID && c.usage_page == 0xFF00)
                || detectors.iter().any(|d| d.matches(c))
                || matchers.iter().any(|m| m.matches(c)))
    };
    let Some(api) = hid_api(wanted) else {
        return Vec::new();
    };
    let mut found: Vec<Box<dyn Backend>> = Vec::new();
    let mut board = false;
    let mut lightspeed: Vec<(u16, hidpp::Lightspeed)> = Vec::new();
    HIDPP_ROUTES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
    let mut taken: Vec<String> = Vec::new();
    let mut infos = Vec::new();
    for raw in crate::device_list(&api).iter() {
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
            (hidpp::VID, pid) if hidpp::PIDS.contains(&pid) => {
                if hidpp::is_long_channel(info.usage_page, info.usage) {
                    let start = std::time::Instant::now();
                    for device in hidpp_devices(&api, &info).into_iter().flatten() {
                        lightspeed.push((pid, device));
                    }
                    note_slow("Logitech HID++", start);
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
    // Any other Logitech HID++ 2.0 device no fixed detector drives, before the generic matchers.
    for info in &infos {
        let key = info.key();
        if info.vid != hidpp::VID
            || info.bus != registry::Bus::Usb
            || !hidpp::is_long_channel(info.usage_page, info.usage)
            || taken.contains(&key)
            || detectors
                .iter()
                .any(|d| (d.vid, d.pid) == (info.vid, info.pid))
        {
            continue;
        }
        let start = std::time::Instant::now();
        let before = lightspeed.len();
        for device in hidpp_devices(&api, info).into_iter().flatten() {
            lightspeed.push((info.pid, device));
        }
        note_slow(&format!("Logitech HID++ {:04x}", info.pid), start);
        if lightspeed.len() > before {
            taken.push(key);
        }
    }
    // Any-product matchers only see what the fixed detectors and hidpp.rs left.
    // A keyboard or mouse on its cable is driven there; the receiver's copy of it is left out.
    let ids: Vec<(String, bool)> = lightspeed
        .iter()
        .map(|(_, d)| (d.id().to_string(), d.wired()))
        .collect();
    let refs: Vec<(&str, bool)> = ids.iter().map(|(id, w)| (id.as_str(), *w)).collect();
    let keep = keep_hidpp(&refs);
    for ((pid, device), keep) in lightspeed.into_iter().zip(keep) {
        if keep {
            HIDPP_ROUTES
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push((pid, device.id().to_string()));
            found.push(Box::new(device));
        }
    }
    let mut claimed = Vec::new();
    found.extend(registry::sweep(
        &infos,
        &mut taken,
        &detectors,
        &matchers,
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
    for pid in hidpp::PIDS {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_on_both_its_cable_and_receiver_is_driven_once() {
        let devices = [
            ("hid:logitech-c539", false),
            ("hid:logitech-4099", false),
            ("hid:logitech-c539", true),
            ("hid:logitech-c547", false),
            ("hid:logitech-c547", false),
        ];
        assert_eq!(keep_hidpp(&devices), [false, true, true, true, false]);
    }

    #[test]
    fn routes_name_the_usb_device() {
        HIDPP_ROUTES
            .lock()
            .unwrap()
            .push((0xC548, "hid:logitech-40a5".into()));
        assert!(logitech_drives(0xC548, "hid:logitech-40a5"));
        assert!(!logitech_drives(0xC547, "hid:logitech-40a5"));
    }
}
