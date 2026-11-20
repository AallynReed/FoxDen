//! Fallbacks for any vendor: the level Windows keeps for Bluetooth devices (with the Fast Pair
//! left / right / case split when earbuds broadcast one), and standard HID battery usages.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_DevNode_PropertyW, CM_Get_Device_Interface_PropertyW, CM_LOCATE_DEVNODE_NORMAL,
    CM_Locate_DevNodeW, CR_SUCCESS,
};
use windows_sys::Win32::Devices::HumanInterfaceDevice::{
    HIDP_CAPS, HIDP_REPORT_TYPE, HIDP_STATUS_SUCCESS, HIDP_VALUE_CAPS, HidD_FreePreparsedData,
    HidD_GetFeature, HidD_GetInputReport, HidD_GetPreparsedData, HidP_Feature, HidP_GetCaps,
    HidP_GetUsageValue, HidP_GetValueCaps, HidP_Input, PHIDP_PREPARSED_DATA,
};
use windows_sys::Win32::Devices::Properties::{
    DEVPKEY_Device_ContainerId, DEVPKEY_Device_InstanceId, DEVPROP_TYPE_GUID,
};
use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};

use super::bt::{adverts, bud_level, nodes, split_detail};
use crate::hid::Coll;
use crate::util::{clamp100, guess_kind, kind_from_collections};
use crate::{Battery, Ctx, Kind};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct FastPair {
    left: Option<(u8, bool)>,
    right: Option<(u8, bool)>,
    case: Option<(u8, bool)>,
}

static FAST_PAIR: Mutex<Option<HashMap<u64, (FastPair, Instant)>>> = Mutex::new(None);

/// Fast Pair service data after the UUID: version 0, then fields with a length/type header byte.
/// Types 3 and 4 are the battery: left, right, case; bit 7 charging, 0x7F unknown.
fn parse_fast_pair(d: &[u8]) -> Option<FastPair> {
    let (&0, mut rest) = d.split_first()? else {
        return None;
    };
    let part = |b: u8| (b & 0x7F <= 100).then_some((b & 0x7F, b & 0x80 != 0));
    while let Some((&h, tail)) = rest.split_first() {
        let (len, ty) = ((h >> 4) as usize, h & 0x0F);
        let body = tail.get(..len)?;
        if matches!(ty, 3 | 4) && !body.is_empty() {
            let at = |i: usize| body.get(i).copied().and_then(part);
            let fp = FastPair {
                left: at(0),
                right: at(1),
                case: at(2),
            };
            return (fp != FastPair::default()).then_some(fp);
        }
        rest = &tail[len..];
    }
    None
}

pub(super) fn heard_fast_pair(addr: u64, data: &[u8]) {
    let Some(fp) = parse_fast_pair(data) else {
        return;
    };
    let mut g = FAST_PAIR.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(HashMap::new)
        .insert(addr, (fp, Instant::now()));
}

fn fast_pair(addr: u64) -> Option<FastPair> {
    let g = FAST_PAIR.lock().unwrap_or_else(|e| e.into_inner());
    let (fp, at) = g.as_ref()?.get(&addr)?;
    (at.elapsed() < Duration::from_secs(300)).then_some(*fp)
}

fn apply_fast_pair(b: &mut Battery, fp: &FastPair) {
    let side = |p: Option<(u8, bool)>| (p.map(|v| v.0), p.is_some_and(|v| v.1));
    if let Some((pct, chg)) = bud_level(&[side(fp.left), side(fp.right)]) {
        b.percent = Some(pct);
        b.charging = chg;
    }
    let parts: Vec<_> = [("L", fp.left), ("R", fp.right), ("case", fp.case)]
        .into_iter()
        .filter_map(|(l, p)| p.map(|(v, c)| (l, Some(v), c)))
        .collect();
    b.detail = split_detail(&parts);
}

fn kind_from_class(cod: u32) -> Kind {
    let (major, minor) = ((cod >> 8) & 0x1F, (cod >> 2) & 0x3F);
    match major {
        0x02 => Kind::Phone,
        0x04 => Kind::Headset,
        0x05 if matches!(minor & 0x0F, 0x01 | 0x02) => Kind::Gamepad,
        0x05 if minor >> 4 == 1 => Kind::Keyboard,
        0x05 if minor >> 4 == 2 => Kind::Mouse,
        _ => Kind::Other,
    }
}

/// LE devices carry no class of device, but Windows files them under a category.
fn kind_from_category(c: &str) -> Kind {
    match c {
        "Input.Keyboard" => Kind::Keyboard,
        "Input.Mouse" => Kind::Mouse,
        "Input.Gaming" => Kind::Gamepad,
        "Audio.Headphones" => Kind::Headset,
        _ if c == "Communication.Phone" || c.starts_with("Communication.Phone.") => Kind::Phone,
        _ if c.starts_with("Communication.Headset") => Kind::Headset,
        _ => Kind::Other,
    }
}

/// LE keyboards that also expose a mouse collection.
fn kind_from_ble_ids(vid: u16, pid: u16) -> Kind {
    match (vid, pid) {
        (0x3151, 0x4013) | (0x19F5, 0x324A) => Kind::Keyboard,
        _ => Kind::Other,
    }
}

fn strip_service_suffix(name: &str) -> &str {
    [
        " Hands-Free AG",
        " Hands-Free",
        " Avrcp Transport",
        " Stereo",
        " Audio Gateway",
    ]
    .iter()
    .find_map(|s| name.strip_suffix(s).filter(|n| !n.is_empty()))
    .unwrap_or(name)
}

/// Device container of the physical device an HID interface belongs to, formatted like `Node::container`.
fn container_of(path: &str) -> Option<String> {
    let p = nodes::wide(path);
    let mut buf = [0u16; 512];
    let mut size = (buf.len() * 2) as u32;
    let mut ty = 0;
    let r = unsafe {
        CM_Get_Device_Interface_PropertyW(
            p.as_ptr(),
            &DEVPKEY_Device_InstanceId,
            &mut ty,
            buf.as_mut_ptr() as *mut u8,
            &mut size,
            0,
        )
    };
    if r != CR_SUCCESS {
        return None;
    }
    let mut inst = 0;
    if unsafe { CM_Locate_DevNodeW(&mut inst, buf.as_ptr(), CM_LOCATE_DEVNODE_NORMAL) }
        != CR_SUCCESS
    {
        return None;
    }
    let mut g = [0u8; 16];
    let mut size = 16u32;
    let r = unsafe {
        CM_Get_DevNode_PropertyW(
            inst,
            &DEVPKEY_Device_ContainerId,
            &mut ty,
            g.as_mut_ptr(),
            &mut size,
            0,
        )
    };
    (r == CR_SUCCESS && ty == DEVPROP_TYPE_GUID)
        .then(|| nodes::guid_text(&nodes::guid_from_mem(&g)))
}

struct Level {
    pct: u8,
    name: String,
    connected: bool,
}

#[derive(Default)]
struct Container {
    name: String,
    address: Option<u64>,
    cod: Option<u32>,
    category: String,
    connected: Option<bool>,
}

pub fn poll_bluetooth(ctx: &mut Ctx) {
    let mut containers: HashMap<String, Container> = HashMap::new();
    let mut levels: HashMap<String, Level> = HashMap::new();
    let mut order = Vec::new();
    for n in ["BTHENUM", "BTHLE", "BTHLEDEVICE"]
        .iter()
        .flat_map(|e| nodes::list(e))
    {
        if n.is_device() {
            let c = containers.entry(n.container.clone()).or_default();
            c.name = n.name.clone();
            c.address = n.address().or(c.address);
            c.cod = n.cod.or(c.cod);
            if !n.category.is_empty() {
                c.category = n.category.clone();
            }
            // Classic and LE nodes can share a container: connected if either is.
            if let Some(on) = n.connected {
                c.connected = Some(c.connected.unwrap_or(false) || on);
            }
        }
        let Some(pct) = n.battery.filter(|&p| p <= 100) else {
            continue;
        };
        if n.problem {
            continue;
        }
        // Headphones keep the level on a service node that may read disconnected; the device node decides below.
        let connected = n.connected.unwrap_or(true);
        if levels
            .get(&n.container)
            .is_some_and(|l| l.connected && !connected)
        {
            continue;
        }
        if !levels.contains_key(&n.container) {
            order.push(n.container.clone());
        }
        levels.insert(
            n.container.clone(),
            Level {
                pct,
                name: strip_service_suffix(&n.name).to_string(),
                connected,
            },
        );
    }
    if levels.is_empty() {
        return;
    }

    // Devices a vendor driver already reported over an HID interface on the Bluetooth link itself.
    let mut reported = HashSet::new();
    let mut hid_ids: HashMap<String, (u16, u16)> = HashMap::new();
    for c in &ctx.colls {
        let Some(cont) = container_of(&c.path) else {
            continue;
        };
        if ctx.is_claimed(c.vid, c.pid) {
            reported.insert(cont.clone());
        }
        hid_ids.insert(cont, (c.vid, c.pid));
    }

    let mut watcher_checked = false;
    let mut waited = false;
    for cid in order {
        let l = &levels[&cid];
        if reported.contains(&cid) {
            continue;
        }
        let cont = containers.get(&cid);
        if !cont.and_then(|c| c.connected).unwrap_or(l.connected) {
            continue;
        }
        let name = cont
            .map(|c| c.name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| l.name.clone());
        let name = if name.is_empty() {
            "Bluetooth device".to_string()
        } else {
            name
        };
        let base = format!("bt-{cid}");
        if ctx
            .out
            .iter()
            .any(|o| o.key == base || (!o.key.starts_with("bt-") && o.name == name))
        {
            continue;
        }
        let ids = hid_ids.get(&cid).copied();
        let mut kind = cont
            .and_then(|c| c.cod)
            .map_or(Kind::Other, kind_from_class);
        if kind == Kind::Other {
            kind = cont.map_or(Kind::Other, |c| kind_from_category(&c.category));
        }
        if kind == Kind::Other
            && let Some((v, p)) = ids
        {
            kind = kind_from_ble_ids(v, p);
            if kind == Kind::Other {
                kind = kind_from_collections(&ctx.colls, v, p);
            }
        }
        if kind == Kind::Other {
            kind = guess_kind(&name);
        }
        let mut b = Battery {
            key: ctx.stable_key(&base),
            name,
            via: "Bluetooth".into(),
            kind,
            percent: Some(l.pct),
            vid: ids.map_or(0, |i| i.0),
            ..Default::default()
        };
        if kind == Kind::Headset
            && let Some(addr) = cont.and_then(|c| c.address)
        {
            let just_started = !watcher_checked && adverts::ensure();
            watcher_checked = true;
            let mut fp = fast_pair(addr);
            // Right after the watcher starts, wait once for broadcasts (about one every 1-2 s).
            if just_started && !waited {
                waited = true;
                for _ in 0..15 {
                    if fp.is_some() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(200));
                    fp = fast_pair(addr);
                }
            }
            if let Some(fp) = fp {
                apply_fast_pair(&mut b, &fp);
            }
        }
        ctx.out.push(b);
        ctx.bt_containers.insert(cid);
    }
}

fn is_battery_usage(page: u16, usage: u16) -> bool {
    matches!((page, usage), (0x06, 0x20) | (0x85, 0x65))
}

/// Percentage from a raw value and its logical range. Apple devices send a plain percentage
/// while declaring 0-255.
fn scale(vid: u16, val: u32, min: i32, max: i32) -> Option<u8> {
    if vid == 0x05AC && val <= 100 {
        Some(val as u8)
    } else if max > min {
        Some(clamp100(
            ((val as i64 - min as i64) * 100 / (max as i64 - min as i64)) as i32,
        ))
    } else {
        (val <= 100).then_some(val as u8)
    }
}

/// A standard battery usage in a feature or input report of the collection.
fn read_battery_usage(c: &Coll) -> Option<u8> {
    let path = nodes::wide(&c.path);
    let h = unsafe {
        CreateFileW(
            path.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        return None;
    }
    let mut pp: PHIDP_PREPARSED_DATA = 0;
    if !unsafe { HidD_GetPreparsedData(h, &mut pp) } {
        unsafe { CloseHandle(h) };
        return None;
    }
    let mut caps: HIDP_CAPS = unsafe { std::mem::zeroed() };
    let mut pct = None;
    if unsafe { HidP_GetCaps(pp, &mut caps) } == HIDP_STATUS_SUCCESS {
        let kinds: [(HIDP_REPORT_TYPE, u16, u16); 2] = [
            (
                HidP_Feature,
                caps.NumberFeatureValueCaps,
                caps.FeatureReportByteLength,
            ),
            (
                HidP_Input,
                caps.NumberInputValueCaps,
                caps.InputReportByteLength,
            ),
        ];
        for (ty, count, len) in kinds {
            if pct.is_some() || count == 0 || len == 0 {
                continue;
            }
            let mut n = count;
            let mut vc: Vec<HIDP_VALUE_CAPS> = vec![unsafe { std::mem::zeroed() }; n as usize];
            if unsafe { HidP_GetValueCaps(ty, vc.as_mut_ptr(), &mut n, pp) } != HIDP_STATUS_SUCCESS
            {
                continue;
            }
            for v in &vc[..n as usize] {
                let usage = unsafe {
                    if v.IsRange {
                        v.Anonymous.Range.UsageMin
                    } else {
                        v.Anonymous.NotRange.Usage
                    }
                };
                if !is_battery_usage(v.UsagePage, usage) {
                    continue;
                }
                let mut buf = vec![0u8; len as usize];
                buf[0] = v.ReportID;
                let got = unsafe {
                    if ty == HidP_Feature {
                        HidD_GetFeature(h, buf.as_mut_ptr() as _, len as u32)
                    } else {
                        HidD_GetInputReport(h, buf.as_mut_ptr() as _, len as u32)
                    }
                };
                let mut val = 0u32;
                if got
                    && unsafe {
                        HidP_GetUsageValue(
                            ty,
                            v.UsagePage,
                            v.LinkCollection,
                            usage,
                            &mut val,
                            pp,
                            buf.as_mut_ptr(),
                            len as u32,
                        )
                    } == HIDP_STATUS_SUCCESS
                {
                    pct = scale(c.vid, val, v.LogicalMin, v.LogicalMax);
                    if pct.is_some() {
                        break;
                    }
                }
            }
        }
    }
    unsafe {
        HidD_FreePreparsedData(pp);
        CloseHandle(h);
    }
    pct
}

pub fn poll_generic_hid(ctx: &mut Ctx) {
    // Drawing tablets are never read: polling one for a battery report can upset its input.
    let tablets: HashSet<(u16, u16)> = ctx
        .colls
        .iter()
        .filter(|c| c.page == 0x0D)
        .map(|c| (c.vid, c.pid))
        .collect();
    let colls = ctx.colls.clone();
    for c in &colls {
        if ctx.is_claimed(c.vid, c.pid)
            || matches!(c.page, 0x84 | 0x85)
            || tablets.contains(&(c.vid, c.pid))
        {
            continue;
        }
        if !ctx.bt_containers.is_empty()
            && container_of(&c.path).is_some_and(|k| ctx.bt_containers.contains(&k))
        {
            continue;
        }
        let Some(pct) = read_battery_usage(c) else {
            continue;
        };
        let name = if c.product.is_empty() {
            "Wireless device".to_string()
        } else {
            c.product.clone()
        };
        let mut kind = kind_from_collections(&ctx.colls, c.vid, c.pid);
        if kind == Kind::Other {
            kind = guess_kind(&name);
        }
        let p = c.path.to_lowercase();
        let bt = p.contains("00001124-0000-1000-8000-00805f9b34fb")
            || p.contains("00001812-0000-1000-8000-00805f9b34fb");
        let b = Battery {
            key: ctx.stable_key(&format!("hid-{:04x}:{:04x}", c.vid, c.pid)),
            name,
            via: if bt { "Bluetooth" } else { "USB" }.into(),
            kind,
            percent: Some(pct),
            ..Default::default()
        };
        ctx.add(b, c.vid, c.pid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_pair_battery_field() {
        // Version 0, salt field (type 1, 1 byte), battery shown (type 3, 3 bytes).
        let fp = parse_fast_pair(&[0x00, 0x11, 0xAB, 0x33, 0x50, 0xC6, 0x7F]).unwrap();
        assert_eq!(fp.left, Some((80, false)));
        assert_eq!(fp.right, Some((70, true)));
        assert_eq!(fp.case, None);
        // Discoverable adverts carry only a model id.
        assert_eq!(parse_fast_pair(&[0x2A, 0x41, 0x0F]), None);
        let mut b = Battery::default();
        apply_fast_pair(&mut b, &fp);
        assert_eq!((b.percent, b.charging), (Some(80), false));
        assert_eq!(b.detail, "L 80%, R 70% \u{26A1}");
    }

    #[test]
    fn device_kinds() {
        assert_eq!(kind_from_class(0x0204), Kind::Phone);
        assert_eq!(kind_from_class(0x0400), Kind::Headset);
        assert_eq!(kind_from_class(0x0540), Kind::Keyboard);
        assert_eq!(kind_from_class(0x0580), Kind::Mouse);
        assert_eq!(kind_from_class(0x0504), Kind::Gamepad);
        assert_eq!(kind_from_class(0), Kind::Other);
        assert_eq!(
            kind_from_category("Communication.Phone.Smartphone"),
            Kind::Phone
        );
        assert_eq!(
            kind_from_category("Communication.Headset.Bluetooth"),
            Kind::Headset
        );
        assert_eq!(kind_from_ble_ids(0x19F5, 0x324A), Kind::Keyboard);
        assert_eq!(
            strip_service_suffix("WH-1000XM5 Hands-Free AG"),
            "WH-1000XM5"
        );
    }

    #[test]
    fn hid_scaling() {
        assert_eq!(scale(0x05AC, 64, 0, 255), Some(64));
        assert_eq!(scale(0x1234, 128, 0, 255), Some(50));
        assert_eq!(scale(0x1234, 90, 0, 0), Some(90));
        assert_eq!(scale(0x1234, 300, 0, 0), None);
    }
}
