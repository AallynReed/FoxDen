//! Finds lighting hardware without touching it: the HID device list, WMI and one TCP connect.

use std::net::{TcpStream, ToSocketAddrs};
use std::sync::OnceLock;
use std::time::Duration;

use foxden_core::{Device, DeviceKind};
use foxden_hid::HidInfo;
use serde::{Deserialize, Serialize};
use wmi::WMIConnection;

use crate::config::Settings;

/// How often the watcher looks again.
pub const POLL: Duration = Duration::from_secs(3);
const PROBE: Duration = Duration::from_millis(300);
/// Passes to sit out after OpenRGB refused us, about fifteen seconds.
const RETRY_PASSES: u32 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Bus {
    Usb,
    Board,
    Memory,
    Gpu,
    Network,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Route {
    Native,
    Openrgb,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Active,
    Available,
    Unsupported,
    /// No lighting; listed for its battery.
    Battery,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Detected {
    pub id: String,
    pub name: String,
    pub kind: &'static str,
    pub bus: Bus,
    pub route: Route,
    pub state: State,
    pub note: String,
}

/// A piece of hardware as found, before we know what drives it.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub id: String,
    pub name: String,
    pub kind: &'static str,
    pub bus: Bus,
    /// Engine id of the device FoxDen's own driver offers for it, or [`GPU`] for a graphics card.
    pub native: Option<&'static str>,
    /// Brand OpenRGB has drivers for.
    pub brand: Option<&'static str>,
    /// No lighting, but a vendor channel such as a wireless receiver's: listed for its battery.
    pub wireless: bool,
}

/// USB devices with a driver in foxden-hid: ids, name, kind, engine device id.
const NATIVE: &[(u16, u16, &str, &str, &str)] = &[
    (
        0x046D,
        0xC539,
        "Logitech G502 Lightspeed receiver",
        "mouse",
        "hid:logitech-c539",
    ),
    (
        0x046D,
        0xC547,
        "Logitech G515 Lightspeed TKL receiver",
        "keyboard",
        "hid:logitech-c547",
    ),
    (
        0x046D,
        0xC08D,
        "Logitech G502 Lightspeed (cable)",
        "mouse",
        "hid:logitech-c539",
    ),
    (
        0x046D,
        0xC355,
        "Logitech G515 Lightspeed TKL (cable)",
        "keyboard",
        "hid:logitech-c547",
    ),
    (
        0x1462,
        0x7D99,
        "MSI Mystic Light controller",
        "motherboard",
        "hid:msi-7d99",
    ),
];

/// USB vendor ids OpenRGB has lighting drivers for, checked against its detectors and usb.ids.
const VENDORS: &[(u16, &str)] = &[
    (0x03F0, "HyperX"),
    (0x0414, "Gigabyte"),
    (0x046D, "Logitech"),
    (0x0951, "HyperX"),
    (0x0B05, "ASUS"),
    (0x0DB0, "MSI"),
    (0x1038, "SteelSeries"),
    (0x1044, "Gigabyte"),
    (0x1462, "MSI"),
    (0x1532, "Razer"),
    (0x187C, "Alienware"),
    (0x1B1C, "Corsair"),
    (0x1E71, "NZXT"),
    (0x1E7D, "Roccat"),
    (0x2516, "Cooler Master"),
    (0x264A, "Thermaltake"),
    (0x26CE, "ASRock"),
    (0x31E3, "Wooting"),
    (0x3842, "EVGA"),
];

/// Board makers as WMI spells them.
const BOARDS: &[(&str, &str)] = &[
    ("micro-star", "MSI"),
    ("asustek", "ASUS"),
    ("gigabyte", "Gigabyte"),
    ("asrock", "ASRock"),
];

/// Memory makers with lit modules OpenRGB drives.
const MEMORY: &[(&str, &str)] = &[
    ("corsair", "Corsair"),
    ("crucial", "Crucial"),
    ("kingston", "Kingston"),
    ("patriot", "Patriot"),
    ("g.skill", "G.Skill"),
    ("g skill", "G.Skill"),
    ("teamgroup", "TeamGroup"),
    ("team group", "TeamGroup"),
    ("adata", "ADATA XPG"),
    ("xpg", "ADATA XPG"),
    ("biwin", "Acer Predator"),
    ("predator", "Acer Predator"),
    ("acer", "Acer Predator"),
    ("lexar", "Lexar"),
];

/// PCI subsystem vendors of the card makers in OpenRGB's pci_ids.h.
const CARD_MAKERS: &[(u16, &str)] = &[
    (0x1043, "ASUS"),
    (0x10B0, "Gainward"),
    (0x10DE, "NVIDIA"),
    (0x1458, "Gigabyte"),
    (0x1462, "MSI"),
    (0x148C, "PowerColor"),
    (0x1569, "Palit"),
    (0x174B, "Sapphire"),
    (0x1849, "ASRock"),
    (0x196E, "PNY"),
    (0x19DA, "Zotac"),
    (0x1B4C, "Galax"),
    (0x1DA2, "Sapphire"),
    (0x3842, "EVGA"),
    (0x7377, "Colorful"),
];

/// The lighting brand behind a USB vendor id, if it is one.
pub fn brand(vendor: u16, product_name: &str) -> Option<&'static str> {
    // HP's id is mostly printers and office gear; only its HyperX range is lit.
    if vendor == 0x03F0 && !product_name.to_lowercase().contains("hyperx") {
        return None;
    }
    VENDORS
        .iter()
        .find(|(id, _)| *id == vendor)
        .map(|(_, name)| *name)
}

/// Words plain office gear names itself with.
const PLAIN_WORDS: &[&str] = &[
    "usb",
    "optical",
    "wireless",
    "receiver",
    "mouse",
    "keyboard",
    "unifying",
    "nano",
    "combo",
    "hid",
    "input",
    "composite",
    "dongle",
    "2.4g",
    "and",
];

/// Words of Logitech's gaming range, besides "G" model numbers.
const LOGITECH_GAMING: &[&str] = &["gaming", "pro", "lightspeed", "powerplay", "litra", "astro"];

/// A lit brand's device no driver knows, whose name says it is plain office gear.
/// Well-known gear from lighting brands that has no lighting at all.
const UNLIT_MODELS: &[&str] = &["solocast", "procast", "seraph"];

fn unlit_usb(vendor: u16, brand: &str, label: &str) -> bool {
    let name = label.to_lowercase();
    let words: Vec<&str> = name
        .split(|c: char| !c.is_alphanumeric() && c != '.')
        .filter(|w| !w.is_empty())
        .collect();
    // nothing to judge by, so it keeps its chance
    if words.is_empty() || name == "device" {
        return false;
    }
    if words.iter().any(|w| UNLIT_MODELS.contains(w)) {
        return true;
    }
    let brand = brand.to_lowercase();
    if words
        .iter()
        .all(|w| PLAIN_WORDS.contains(w) || brand.split(' ').any(|b| b == *w))
    {
        return true;
    }
    let gaming = |w: &&str| {
        LOGITECH_GAMING.contains(w)
            || w.strip_prefix('g')
                .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
    };
    vendor == 0x046D && !words.iter().any(gaming)
}

fn titled(brand: &str, name: &str) -> String {
    if name.to_lowercase().contains(&brand.to_lowercase()) {
        name.to_string()
    } else {
        format!("{brand} {name}")
    }
}

fn usb_kind(name: &str, parts: &[&HidInfo]) -> &'static str {
    let name = name.to_lowercase();
    let says = |words: &[&str]| words.iter().any(|word| name.contains(word));
    let has = |usage| {
        parts
            .iter()
            .any(|d| d.usage_page == 0x01 && d.usage == usage)
    };
    if says(&["headset", "headphone", "arctis"]) {
        "headset"
    } else if says(&["microphone", "solocast", "quadcast"]) {
        "microphone"
    } else if says(&["keyboard"]) || (has(0x06) && !has(0x02)) {
        "keyboard"
    } else if says(&["mouse"]) || (has(0x02) && !has(0x06)) {
        "mouse"
    } else {
        "other"
    }
}

/// Ported controllers that light the motherboard itself.
const BOARD_CONTROLLERS: &[&str] = &[
    "MSI Mystic Light",
    "ASUS Aura Motherboard",
    "ASUS Aura Addressable",
    "ASRock Polychrome USB",
    "Gigabyte RGB Fusion 2 USB",
];

/// Vendor and product id of every USB HID device, sorted, one per device.
fn usb_ids(hid: &[HidInfo]) -> Vec<(u16, u16)> {
    let mut ids: Vec<(u16, u16)> = hid
        .iter()
        .filter(|d| d.usb)
        .map(|d| (d.vendor, d.product))
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// Devices whose lighting no software can reach: they expose only standard keyboard and mouse interfaces.
const ONBOARD_ONLY: &[&str] = &["usb:0951:172d"];

/// Devices of FoxDen's ported families, matched on vendor and product id.
const PORTED: &str = "hid:";

/// The OpenRGB ports' HID devices, then foxden-xpack's.
fn devices() -> Vec<(u16, u16, &'static str)> {
    let mut list = foxden_hid::family_devices();
    for d in foxden_xpack::detectors() {
        list.push((d.vid, d.pid, d.name));
    }
    list
}

fn ported(vendor: u16, product: u16) -> Option<&'static str> {
    static LIST: OnceLock<Vec<(u16, u16, &'static str)>> = OnceLock::new();
    LIST.get_or_init(devices)
        .iter()
        .find(|d| (d.0, d.1) == (vendor, product))
        .map(|d| d.2)
}

/// A headset whose battery and settings FoxDen handles with its own code.
fn headset(vendor: u16, product: u16) -> bool {
    vendor == 0x1038 && foxden_headset::proto::model(product).is_some()
}

/// One entry per USB device we recognise, however many HID collections it shows.
pub fn usb(hid: &[HidInfo]) -> Vec<Found> {
    let mut found = Vec::new();
    for (vendor, product) in usb_ids(hid) {
        let parts: Vec<&HidInfo> = hid
            .iter()
            .filter(|d| d.usb && (d.vendor, d.product) == (vendor, product))
            .collect();
        let label = parts
            .iter()
            .map(|d| d.name.as_str())
            .find(|name| !name.is_empty())
            .unwrap_or("device");
        let brand = brand(vendor, label);
        let family = ported(vendor, product);
        if brand.is_none() && family.is_none() {
            continue;
        }
        let known = NATIVE.iter().find(|n| (n.0, n.1) == (vendor, product));
        // Plain gear without lighting is left out, unless it has a vendor channel that may carry
        // its battery, such as a wireless receiver; that stays so its info can be sent.
        let unlit = known.is_none()
            && family.is_none()
            && brand.is_some_and(|b| unlit_usb(vendor, b, label));
        let vendor_channel = parts.iter().any(|d| d.usage_page >= 0xFF00);
        if unlit && !vendor_channel && !headset(vendor, product) {
            continue;
        }
        let wireless = unlit || headset(vendor, product);
        let name = match (known, family, brand) {
            (Some(n), _, _) => n.2.to_string(),
            (None, Some(f), _) => f.to_string(),
            (None, None, Some(b)) => titled(b, label),
            (None, None, None) => label.to_string(),
        };
        found.push(Found {
            id: format!("usb:{vendor:04x}:{product:04x}"),
            name,
            kind: match (known, family) {
                (Some(n), _) => n.3,
                (None, Some(f)) if BOARD_CONTROLLERS.iter().any(|b| f.starts_with(b)) => {
                    "motherboard"
                }
                _ => usb_kind(label, &parts),
            },
            bus: Bus::Usb,
            native: known.map(|n| n.4).or(family.map(|_| PORTED)),
            brand,
            wireless,
        });
    }
    found
}

#[derive(Deserialize)]
#[serde(rename = "Win32_BaseBoard", rename_all = "PascalCase")]
struct BaseBoard {
    manufacturer: Option<String>,
    product: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename = "Win32_PhysicalMemory", rename_all = "PascalCase")]
struct PhysicalMemory {
    manufacturer: Option<String>,
    part_number: Option<String>,
    capacity: Option<u64>,
    device_locator: Option<String>,
    #[serde(rename = "SMBIOSMemoryType")]
    memory_type: Option<u32>,
}

#[derive(Deserialize)]
#[serde(rename = "Win32_VideoController", rename_all = "PascalCase")]
struct VideoController {
    name: Option<String>,
    #[serde(rename = "PNPDeviceID")]
    path: Option<String>,
}

fn lookup(table: &[(&str, &'static str)], maker: &str) -> Option<&'static str> {
    let maker = maker.to_lowercase();
    table
        .iter()
        .find(|(key, _)| maker.contains(key))
        .map(|(_, brand)| *brand)
}

fn board(b: &BaseBoard) -> Option<Found> {
    let brand = lookup(BOARDS, b.manufacturer.as_deref()?)?;
    let product = b.product.as_deref()?.trim();
    // "B760M GAMING PLUS WIFI (MS-7D99)" goes by the code in brackets
    let code = product
        .rsplit_once('(')
        .map_or(product, |(_, code)| code.trim_end_matches(')'));
    Some(Found {
        id: format!("board:{}", code.trim().replace(' ', "-")),
        name: titled(brand, product),
        kind: "motherboard",
        bus: Bus::Board,
        native: None,
        brand: Some(brand),
        wireless: false,
    })
}

/// Part-number families without lighting. Unknown parts keep their chance.
fn unlit_memory(brand: &str, part: &str) -> bool {
    let part = part.trim().to_uppercase();
    // Crucial's SPD adds a ".M8FE1"-style suffix
    let part = part.split('.').next().unwrap_or_default();
    let starts = |list: &[&str]| list.iter().any(|p| part.starts_with(p));
    let tail = part.rsplit('-').next().unwrap_or_default();
    match brand {
        // Vengeance LPX and DDR5, SODIMM, ValueSelect, DDR3 Vengeance, Vengeance LP and XMS
        "Corsair" => starts(&["CMK", "CMSX", "CMSO", "CMV", "CMZ", "CMY", "CML", "CMX"]),
        "G.Skill" => {
            // "16GTZR" or "TZ5RK": the series code after the size
            let series = tail.trim_start_matches(|c: char| c.is_ascii_digit());
            let series = if series.len() < tail.len() {
                series.strip_prefix('G').unwrap_or(series)
            } else {
                series
            };
            // Ripjaws, Flare X, Sniper X, Aegis, plain Trident Z and Z5
            series == "TZ"
                || [
                    "V", "RK", "RB", "RS", "FX", "SX", "IS", "TZK", "TZS", "TZ5K", "TZ5S",
                ]
                .iter()
                .any(|p| series.starts_with(p))
        }
        // Ballistix without "L" at the end, Ballistix Sport, Crucial and Crucial Pro
        "Crucial" => {
            starts(&["CT", "CP", "BLS"])
                || (part.starts_with("BL") && part.ends_with(['B', 'W', 'R']))
        }
        // ValueRAM, client and server lines and OEM SPD numbers
        "Kingston" => starts(&["KVR", "KCP", "KSM", "9905", "ACR"]),
        // value modules and Gammix D10 and D30
        "ADATA XPG" => starts(&["AD3", "AD4", "AD5"]) || ["SB10", "SR10", "SR30"].contains(&tail),
        // Elite, Elite Plus and Vulcan
        "TeamGroup" => starts(&["TED", "TPD", "TLZ", "FLB", "FLR"]),
        "Patriot" => starts(&["PSD"]),
        _ => false,
    }
}

fn module(m: &PhysicalMemory) -> Option<Found> {
    let brand = lookup(MEMORY, m.manufacturer.as_deref()?)?;
    let part = m.part_number.as_deref().unwrap_or_default().trim();
    if unlit_memory(brand, part) {
        return None;
    }
    let label = if part.is_empty() { "memory" } else { part };
    let slot = m
        .device_locator
        .as_deref()
        .unwrap_or_default()
        .trim()
        .replace(' ', "-");
    let size = m
        .capacity
        .map(|bytes| format!(", {} GB", bytes >> 30))
        .unwrap_or_default();
    let generation = match m.memory_type {
        Some(26) => " DDR4",
        Some(34) => " DDR5",
        _ => "",
    };
    Some(Found {
        // sticks of one kit share a part number, so the slot tells them apart
        id: format!("memory:{part}:{slot}")
            .trim_end_matches(':')
            .to_string(),
        name: format!("{}{size}{generation}", titled(brand, label)),
        kind: "memory",
        bus: Bus::Memory,
        native: None,
        brand: Some(brand),
        wireless: false,
    })
}

/// Prefix of every graphics card device foxden-gpu offers; their ids are only known once probed.
const GPU: &str = "gpu:";

/// Vendor, device, subsystem vendor and device from a path like `PCI\VEN_1002&DEV_7550&SUBSYS_061A1043&REV_C0\..`.
fn pci_ids(path: &str) -> Option<[u16; 4]> {
    let hex = |key: &str, skip: usize| {
        let at = path.find(key)? + key.len() + skip;
        u16::from_str_radix(path.get(at..at + 4)?, 16).ok()
    };
    Some([
        hex("VEN_", 0)?,
        hex("DEV_", 0)?,
        hex("SUBSYS_", 4)?,
        hex("SUBSYS_", 0)?,
    ])
}

fn card(v: &VideoController) -> Option<Found> {
    let name = v.name.as_deref()?.trim();
    let ids = pci_ids(v.path.as_deref()?)?;
    let [vendor, device, maker, _] = ids;
    // Processor graphics from Intel and AMD have nothing to light.
    if vendor != 0x10DE && !(vendor == 0x1002 && name.contains("RX")) {
        return None;
    }
    let brand = CARD_MAKERS
        .iter()
        .find(|(id, _)| *id == maker)
        .map(|(_, brand)| *brand);
    // the card maker's name takes the place of the chip maker's
    let model = name
        .strip_prefix("AMD ")
        .or_else(|| name.strip_prefix("NVIDIA "))
        .unwrap_or(name);
    Some(Found {
        id: format!("gpu:{vendor:04x}:{device:04x}"),
        name: brand.map_or_else(|| name.to_string(), |brand| titled(brand, model)),
        kind: "gpu",
        bus: Bus::Gpu,
        native: foxden_gpu::known(ids).then_some(GPU),
        brand,
        wireless: false,
    })
}

/// Board, memory modules and graphics cards as WMI describes them. They do not change while we run.
pub fn system() -> Vec<Found> {
    let wmi = match WMIConnection::new() {
        Ok(wmi) => wmi,
        Err(e) => {
            crate::log::warn("devices", format!("Windows hardware list unavailable: {e}"));
            return Vec::new();
        }
    };
    let boards: Vec<BaseBoard> = wmi.query().unwrap_or_default();
    let modules: Vec<PhysicalMemory> = wmi.query().unwrap_or_default();
    let cards: Vec<VideoController> = wmi.query().unwrap_or_default();
    let parts = boards.iter().filter_map(board);
    let parts = parts
        .chain(modules.iter().filter_map(module))
        .chain(cards.iter().filter_map(card));
    // two cards of one model, or sticks with no slot name, would share an id
    let mut found: Vec<Found> = Vec::new();
    for part in parts {
        if !found.iter().any(|f| f.id == part.id) {
            found.push(part);
        }
    }
    found
}

fn reachable(host: &str, port: u16) -> bool {
    let Ok(addresses) = (host, port).to_socket_addrs() else {
        return false;
    };
    addresses
        .take(2)
        .any(|address| TcpStream::connect_timeout(&address, PROBE).is_ok())
}

/// What two passes are compared by.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fingerprint {
    /// Vendor and product id of every USB HID device, sorted.
    pub usb: Vec<(u16, u16)>,
    /// The OpenRGB server answered.
    pub openrgb: bool,
}

/// One pass over the hardware. A working OpenRGB connection spares its server the probe.
pub fn scan(system: &[Found], settings: &Settings, openrgb_on: bool) -> (Vec<Found>, Fingerprint) {
    let hid = foxden_hid::enumerate();
    let mut found = usb(&hid);
    // The board's lighting is its USB controller when we have a driver for that.
    let controller = found
        .iter()
        .find(|f| f.kind == "motherboard")
        .and_then(|f| f.native);
    found.extend(system.iter().cloned().map(|mut part| {
        if part.bus == Bus::Board {
            part.native = controller;
        }
        part
    }));
    let openrgb = openrgb_on || reachable(&settings.openrgb_host, settings.openrgb_port);
    (
        found,
        Fingerprint {
            usb: usb_ids(&hid),
            openrgb,
        },
    )
}

/// What a pass means for the app.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Change {
    /// Something was plugged in or removed, or the OpenRGB server came or went.
    pub seen: bool,
    /// The native drivers should probe again.
    pub native: bool,
    /// OpenRGB should be connected again.
    pub openrgb: bool,
}

/// Remembers the last pass so the next can tell what changed.
#[derive(Default)]
pub struct Watch {
    last: Option<Fingerprint>,
    /// Passes to sit out before OpenRGB is tried again.
    wait: u32,
    /// A native driver was failing at the last pass.
    failing: bool,
}

impl Watch {
    /// True once when a native driver starts failing. A quick replug or a hub reset kills its handle
    /// without changing the USB list, so that is the only sign that it needs a fresh probe.
    pub fn died(&mut self, failing: bool) -> bool {
        let before = std::mem::replace(&mut self.failing, failing);
        failing && !before
    }

    /// Call after every connection attempt, so a server with nothing for us is not hammered.
    /// `settled` means it connected and listed devices.
    pub fn tried(&mut self, settled: bool) {
        self.wait = if settled { 0 } else { RETRY_PASSES };
    }

    /// Compares `now` with the last pass. Only backends the user switched on are ever asked for.
    /// `openrgb_on` is false while OpenRGB is not connected or has listed no devices yet.
    pub fn step(&mut self, now: Fingerprint, settings: &Settings, openrgb_on: bool) -> Change {
        let Some(old) = self.last.replace(now.clone()) else {
            return Change {
                seen: true,
                ..Change::default()
            };
        };
        let ours = |print: &Fingerprint| -> Vec<(u16, u16)> {
            print
                .usb
                .iter()
                .copied()
                .filter(|id| NATIVE.iter().any(|n| (n.0, n.1) == *id))
                .collect()
        };
        if now.openrgb && !old.openrgb {
            self.wait = 0;
        }
        let native = settings.native_drivers && ours(&old) != ours(&now);
        let mut change = Change {
            seen: old != now,
            native,
            openrgb: false,
        };
        if settings.openrgb_enabled && now.openrgb && !openrgb_on {
            change.openrgb = self.wait == 0;
            self.wait = if change.openrgb {
                RETRY_PASSES
            } else {
                self.wait - 1
            };
        }
        change
    }
}

/// What the backends are doing right now.
pub struct Live<'a> {
    pub settings: &'a Settings,
    /// Backend label and device of everything a working backend offers.
    pub driven: Vec<(&'a str, &'a Device)>,
    /// The OpenRGB server answered the last probe.
    /// The OpenRGB backend is connected.
    pub openrgb_on: bool,
}

impl Live<'_> {
    /// How many devices the OpenRGB server lists.
    fn listed(&self) -> usize {
        self.driven
            .iter()
            .filter(|(label, _)| *label == "openrgb")
            .count()
    }
}

fn kind_name(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Motherboard => "motherboard",
        DeviceKind::Memory => "memory",
        DeviceKind::Gpu => "gpu",
        DeviceKind::Keyboard => "keyboard",
        DeviceKind::Mouse => "mouse",
        DeviceKind::Strip => "strip",
        DeviceKind::Other => "other",
    }
}

/// Whether an OpenRGB device looks like this piece of hardware. Parts go by kind, USB devices by brand too.
fn same(found: &Found, device: &Device) -> bool {
    let kind = kind_name(device.kind);
    if found.bus != Bus::Usb {
        return kind == found.kind;
    }
    let name = device.name.to_lowercase();
    let branded = found
        .brand
        .is_some_and(|brand| name.contains(&brand.to_lowercase()));
    let fits = match found.kind {
        // memory and graphics cards are never USB devices
        "other" => !matches!(kind, "memory" | "gpu"),
        "headset" | "microphone" => kind == "other",
        wanted => kind == wanted,
    };
    branded && fits
}

/// Probed graphics cards carry no PCI ids, so any card FoxDen lights counts for a known one.
fn drives(found: &Found, native: &str, device: &Device) -> bool {
    let usb = found
        .id
        .strip_prefix("usb:")
        .map(|ids| format!("-{}-", ids.replace(':', "")));
    device.id == native
        || (native == GPU && device.kind == DeviceKind::Gpu && device.id.starts_with(GPU))
        || (native == PORTED
            && device.id.starts_with(PORTED)
            && usb.is_some_and(|ids| device.id.contains(&ids)))
        // Memory sticks are probed on the bus, so any stick a native driver lights counts.
        || (found.bus == Bus::Memory
            && device.kind == DeviceKind::Memory
            && !device.id.starts_with("openrgb"))
        // The board row goes by its lighting controller, a USB device of its own.
        || (native == PORTED
            && found.bus == Bus::Board
            && device.kind == DeviceKind::Motherboard
            && device.id.starts_with(PORTED))
}

fn status(found: &Found, live: &Live) -> (Route, State, String) {
    let settings = live.settings;
    if found.native.is_some_and(|id| {
        live.driven
            .iter()
            .any(|(_, device)| drives(found, id, device))
    }) || live
        .driven
        .iter()
        .any(|(_, device)| drives(found, PORTED, device))
    {
        return (
            Route::Native,
            State::Active,
            "Driven by FoxDen's own driver.".into(),
        );
    }
    if live
        .driven
        .iter()
        .any(|(label, device)| *label == "openrgb" && same(found, device))
    {
        return (
            Route::Openrgb,
            State::Active,
            "Driven through OpenRGB.".into(),
        );
    }
    if found.native.is_some() {
        let note = if settings.native_drivers {
            "The native driver could not open it. Close other lighting software, then rescan."
        } else {
            "FoxDen has its own driver for this, still experimental. Switch on native drivers in Settings."
        };
        return (Route::Native, State::Available, note.into());
    }
    if found.wireless {
        let ids = found
            .id
            .strip_prefix("usb:")
            .and_then(|i| i.split_once(':'));
        let is_headset = ids.is_some_and(|(v, p)| {
            matches!((u16::from_str_radix(v, 16), u16::from_str_radix(p, 16)), (Ok(v), Ok(p)) if headset(v, p))
        });
        let note = if is_headset {
            "Its battery shows in the tray and its settings are under Headset above. It has no lighting."
        } else {
            "No lighting. If it has a battery, its level shows in the tray."
        };
        return (Route::None, State::Battery, note.into());
    }
    if ONBOARD_ONLY.contains(&found.id.as_str()) {
        let note = "Its lighting is set on the device itself, with Fn key combinations. It has no way for software to control it.";
        return (Route::None, State::Unsupported, note.into());
    }
    let note = if found.bus == Bus::Gpu {
        "FoxDen does not control lighting on this card. Many cards have none."
    } else {
        "FoxDen does not control lighting on it. If it has lighting, send its device info."
    };
    (Route::None, State::Unsupported, note.into())
}

/// The detected list: what was found, joined with what drives it now.
pub fn resolve(found: &[Found], live: &Live) -> Vec<Detected> {
    let mut list: Vec<Detected> = found
        .iter()
        .map(|f| {
            let (route, state, note) = status(f, live);
            Detected {
                id: f.id.clone(),
                name: f.name.clone(),
                kind: f.kind,
                bus: f.bus,
                route,
                state,
                note,
            }
        })
        .collect();
    let settings = live.settings;
    if settings.openrgb_enabled || live.openrgb_on {
        let (state, note) = if live.openrgb_on && live.listed() == 0 {
            (State::Active, "Connected, but it lists no devices yet.")
        } else if live.openrgb_on {
            (
                State::Active,
                "Connected. Its devices are in the device list.",
            )
        } else {
            (State::Available, "Not connected. Rescan to try again.")
        };
        list.push(Detected {
            id: "network:openrgb".into(),
            name: format!(
                "OpenRGB server at {}:{}",
                settings.openrgb_host, settings.openrgb_port
            ),
            kind: "server",
            bus: Bus::Network,
            route: Route::Openrgb,
            state,
            note: note.into(),
        });
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hid(
        vendor: u16,
        product: u16,
        usage_page: u16,
        usage: u16,
        name: &str,
        usb: bool,
    ) -> HidInfo {
        HidInfo {
            vendor,
            product,
            usage_page,
            usage,
            name: name.into(),
            manufacturer: String::new(),
            usb,
        }
    }

    /// The collections the development machine lists, one line each.
    fn desk() -> Vec<HidInfo> {
        vec![
            hid(0x046D, 0xC539, 0x0001, 0x0080, "USB Receiver", true),
            hid(0x046D, 0xC539, 0x0001, 0x0006, "USB Receiver", true),
            hid(0x03F0, 0x0B8B, 0xFF90, 0x00FF, "HyperX SoloCast", true),
            hid(0x046D, 0xC547, 0xFF00, 0x0001, "USB Receiver", true),
            hid(0x1038, 0x1FFE, 0xFF00, 0x0001, "Arctis Nova 7", true),
            hid(0x046D, 0xC539, 0xFFBC, 0x0088, "USB Receiver", true),
            hid(0x046D, 0xC547, 0x0001, 0x0006, "USB Receiver", true),
            hid(0x1038, 0x1FFE, 0xFFC0, 0x0001, "Arctis Nova 7", true),
            hid(
                0x2565,
                0x0578,
                0x000C,
                0x0001,
                "MSI FORCE GC300 WIRELESS",
                true,
            ),
            hid(0x1038, 0x1FFE, 0x000C, 0x0001, "Arctis Nova 7", true),
            hid(0x046D, 0xC539, 0xFF00, 0x0002, "USB Receiver", true),
            hid(0x1462, 0x7D99, 0x0001, 0x0000, "MYSTIC LIGHT", true),
            hid(0x046D, 0xC232, 0x0001, 0x0006, "", false),
            hid(0x046D, 0xC547, 0x0001, 0x0002, "USB Receiver", true),
            hid(0x03F0, 0x0B8B, 0x000C, 0x0001, "HyperX SoloCast", true),
        ]
    }

    fn settings(native_drivers: bool, openrgb_enabled: bool) -> Settings {
        Settings {
            native_drivers,
            openrgb_enabled,
            ..Settings::default()
        }
    }

    fn print(usb: &[(u16, u16)], openrgb: bool) -> Fingerprint {
        Fingerprint {
            usb: usb.to_vec(),
            openrgb,
        }
    }

    #[test]
    fn vendor_table_names_the_brand() {
        assert_eq!(brand(0x1B1C, "K70 RGB"), Some("Corsair"));
        assert_eq!(brand(0x1532, ""), Some("Razer"));
        assert_eq!(brand(0x1462, "MYSTIC LIGHT"), Some("MSI"));
        assert_eq!(brand(0x03F0, "HyperX SoloCast"), Some("HyperX"));
        assert_eq!(brand(0x03F0, "HP LaserJet"), None);
        // hubs, storage and controllers come from vendors that are not in the table
        assert_eq!(brand(0x2565, "MSI FORCE GC300 WIRELESS"), None);
        assert_eq!(brand(0x0BDA, "USB3.0 Hub"), None);

        let mut ids: Vec<u16> = VENDORS.iter().map(|(id, _)| *id).collect();
        assert!(ids.is_sorted());
        ids.dedup();
        assert_eq!(ids.len(), VENDORS.len());
        assert!(NATIVE.iter().all(|n| brand(n.0, "").is_some()));
    }

    #[test]
    fn hid_collections_collapse_into_one_entry_per_device() {
        let found = usb(&desk());
        let ids: Vec<&str> = found.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "usb:03f0:0b8b",
                "usb:046d:c539",
                "usb:046d:c547",
                "usb:1038:1ffe",
                "usb:1462:7d99"
            ]
        );
        let names: Vec<&str> = found.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names[0], "HyperX SoloCast");
        assert_eq!(names[3], "SteelSeries Arctis Nova 7");
        let kinds: Vec<&str> = found.iter().map(|f| f.kind).collect();
        assert_eq!(
            kinds,
            ["microphone", "mouse", "keyboard", "headset", "motherboard"]
        );
        assert_eq!(found[0].native, None);
        assert_eq!(found[1].native, Some("hid:logitech-c539"));
        assert_eq!(found[4].native, Some("hid:msi-7d99"));
        assert!(found.iter().all(|f| f.bus == Bus::Usb && f.brand.is_some()));
        assert_eq!(usb_ids(&desk()).len(), 6);
    }

    #[test]
    fn unnamed_devices_fall_back_to_their_hid_usage() {
        // made-up product ids
        let keys = [
            hid(0x1B1C, 1, 0x0001, 0x0006, "", true),
            hid(0x1B1C, 1, 0xFFC2, 0x0004, "", true),
        ];
        let found = usb(&keys);
        assert_eq!(
            (found[0].name.as_str(), found[0].kind),
            ("Corsair device", "keyboard")
        );
        let both = [
            hid(0x1532, 2, 0x0001, 0x0006, "Razer Thing", true),
            hid(0x1532, 2, 0x0001, 0x0002, "", true),
        ];
        let found = usb(&both);
        assert_eq!(
            (found[0].name.as_str(), found[0].kind),
            ("Razer Thing", "other")
        );
    }

    #[test]
    fn the_first_pass_reports_and_reloads_nothing() {
        let mut watch = Watch::default();
        let change = watch.step(
            print(&[(0x046D, 0xC539)], true),
            &settings(true, true),
            false,
        );
        assert_eq!(
            change,
            Change {
                seen: true,
                native: false,
                openrgb: false
            }
        );
    }

    #[test]
    fn plugging_devices_reloads_only_enabled_native_drivers() {
        let quiet = print(&[(0x1038, 0x1FFE)], false);
        let headset_gone = print(&[], false);
        let mouse = print(&[(0x046D, 0xC539)], false);

        let mut watch = Watch::default();
        watch.step(quiet.clone(), &settings(true, false), false);
        assert_eq!(
            watch.step(quiet.clone(), &settings(true, false), false),
            Change::default()
        );
        // a device without a native driver changes the list, not the backends
        let change = watch.step(headset_gone.clone(), &settings(true, false), false);
        assert_eq!(
            change,
            Change {
                seen: true,
                native: false,
                openrgb: false
            }
        );
        let change = watch.step(mouse.clone(), &settings(true, false), false);
        assert_eq!(
            change,
            Change {
                seen: true,
                native: true,
                openrgb: false
            }
        );
        let change = watch.step(headset_gone.clone(), &settings(true, false), false);
        assert_eq!(
            change,
            Change {
                seen: true,
                native: true,
                openrgb: false
            }
        );
        // with native drivers off the mouse is only reported
        let change = watch.step(mouse, &settings(false, false), false);
        assert_eq!(
            change,
            Change {
                seen: true,
                native: false,
                openrgb: false
            }
        );
    }

    #[test]
    fn openrgb_is_reconnected_when_it_appears_and_not_hammered() {
        let (down, up) = (print(&[], false), print(&[], true));
        let on = settings(false, true);

        let mut watch = Watch::default();
        watch.step(down.clone(), &on, false);
        assert_eq!(watch.step(down.clone(), &on, false), Change::default());
        // the server starts: connect at once
        assert_eq!(
            watch.step(up.clone(), &on, false),
            Change {
                seen: true,
                native: false,
                openrgb: true
            }
        );
        // the attempt failed: wait a few passes, then try again
        watch.tried(false);
        for _ in 0..RETRY_PASSES {
            assert_eq!(watch.step(up.clone(), &on, false), Change::default());
        }
        assert!(watch.step(up.clone(), &on, false).openrgb);
        // connected: nothing to do until the backend fails, then reconnect straight away
        watch.tried(true);
        assert_eq!(watch.step(up.clone(), &on, true), Change::default());
        assert!(watch.step(up.clone(), &on, false).openrgb);
        // a restart we did see go down is picked up the moment it is back
        watch.tried(false);
        assert_eq!(
            watch.step(down.clone(), &on, false),
            Change {
                seen: true,
                native: false,
                openrgb: false
            }
        );
        assert!(watch.step(up.clone(), &on, false).openrgb);
    }

    #[test]
    fn a_driver_that_starts_failing_is_probed_again_once() {
        let mut watch = Watch::default();
        assert!(!watch.died(false));
        assert!(watch.died(true));
        // still failing after the fresh probe: left alone until it has worked again
        assert!(!watch.died(true));
        assert!(!watch.died(true));
        assert!(!watch.died(false));
        assert!(watch.died(true));
    }

    #[test]
    fn openrgb_is_never_switched_on_for_the_user() {
        let mut watch = Watch::default();
        watch.step(print(&[], false), &settings(false, false), false);
        let change = watch.step(print(&[], true), &settings(false, false), false);
        assert_eq!(
            change,
            Change {
                seen: true,
                native: false,
                openrgb: false
            }
        );
        assert_eq!(
            watch.step(print(&[], true), &settings(false, false), false),
            Change::default()
        );
    }

    #[test]
    fn a_ported_board_controller_lights_the_board() {
        let found = usb(&[hid(0x1462, 0x7D75, 0x0001, 0x0000, "MYSTIC LIGHT ", true)]);
        assert_eq!(found[0].kind, "motherboard");
        assert_eq!(found[0].native, Some(PORTED));
    }

    #[test]
    fn wmi_rows_become_parts() {
        let b = BaseBoard {
            manufacturer: Some("Micro-Star International Co., Ltd.".into()),
            product: Some("B760M GAMING PLUS WIFI (MS-7D99)".into()),
        };
        let b = board(&b).unwrap();
        assert_eq!(
            (b.id.as_str(), b.name.as_str()),
            ("board:MS-7D99", "MSI B760M GAMING PLUS WIFI (MS-7D99)")
        );
        assert_eq!(
            (b.kind, b.bus, b.brand),
            ("motherboard", Bus::Board, Some("MSI"))
        );
        let office = BaseBoard {
            manufacturer: Some("Dell Inc.".into()),
            product: Some("0X8DXD".into()),
        };
        assert_eq!(board(&office), None);

        let m = PhysicalMemory {
            manufacturer: Some("Corsair".into()),
            part_number: Some("CMH32GX5M2B6000Z30  ".into()),
            capacity: Some(17_179_869_184),
            device_locator: Some("Controller0-DIMMA2".into()),
            memory_type: Some(34),
        };
        let stick = module(&m).unwrap();
        assert_eq!(stick.id, "memory:CMH32GX5M2B6000Z30:Controller0-DIMMA2");
        assert_eq!(stick.name, "Corsair CMH32GX5M2B6000Z30, 16 GB DDR5");
        assert_eq!(
            (stick.kind, stick.bus, stick.brand),
            ("memory", Bus::Memory, Some("Corsair"))
        );
        let bare = PhysicalMemory {
            manufacturer: Some("Kingston".into()),
            part_number: None,
            capacity: None,
            device_locator: None,
            memory_type: None,
        };
        let bare = module(&bare).unwrap();
        assert_eq!(
            (bare.id.as_str(), bare.name.as_str()),
            ("memory", "Kingston memory")
        );
        assert_eq!(
            module(&PhysicalMemory {
                manufacturer: Some("Samsung".into()),
                ..m
            }),
            None
        );

        let radeon = VideoController {
            name: Some("AMD Radeon RX 9070 XT".into()),
            path: Some(
                r"PCI\VEN_1002&DEV_7550&SUBSYS_061A1043&REV_C0\6&13014B44&0&00000008".into(),
            ),
        };
        let gpu = card(&radeon).unwrap();
        assert_eq!(
            (gpu.id.as_str(), gpu.name.as_str()),
            ("gpu:1002:7550", "ASUS Radeon RX 9070 XT")
        );
        assert_eq!(
            (gpu.kind, gpu.bus, gpu.brand),
            ("gpu", Bus::Gpu, Some("ASUS"))
        );
        let founders = VideoController {
            name: Some("NVIDIA GeForce RTX 4080".into()),
            path: Some(r"PCI\VEN_10DE&DEV_2704&SUBSYS_167A10DE&REV_A1\4&1A2B3C4D&0&0008".into()),
        };
        let founders = card(&founders).unwrap();
        assert_eq!(founders.name, "NVIDIA GeForce RTX 4080");
        // NVIDIA Illumination lists the 4080 Founders Edition, nothing lists this ASUS card
        assert_eq!((founders.native, gpu.native), (Some(GPU), None));
        let unbranded = VideoController {
            name: Some("NVIDIA GeForce RTX 4080".into()),
            path: Some(r"PCI\VEN_10DE&DEV_2704&SUBSYS_00000000&REV_A1\4&1A2B3C4D&0&0008".into()),
        };
        let unbranded = card(&unbranded).unwrap();
        assert_eq!(
            (unbranded.name.as_str(), unbranded.brand),
            ("NVIDIA GeForce RTX 4080", None)
        );
        let onboard = VideoController {
            name: Some("Intel(R) UHD Graphics 770".into()),
            path: Some(r"PCI\VEN_8086&DEV_A780&SUBSYS_7D991462&REV_04\3&11583659&0&10".into()),
        };
        assert_eq!(card(&onboard), None);
        let remote = VideoController {
            name: Some("Remote".into()),
            path: Some(r"ROOT\DISPLAY\0000".into()),
        };
        assert_eq!(card(&remote), None);
    }

    #[test]
    fn a_supported_headset_is_not_called_unsupported() {
        let found = usb(&[hid(0x1038, 0x22A1, 0xFFC0, 0x0001, "Arctis Nova 7", true)]);
        let both = settings(true, true);
        let live = Live {
            settings: &both,
            driven: Vec::new(),
            openrgb_on: false,
        };
        let row = &resolve(&found, &live)[0];
        assert_eq!(row.state, State::Battery);
        assert!(row.note.contains("Headset"));
    }

    #[test]
    fn hardware_without_lighting_is_left_out() {
        let office = vec![
            hid(0x046D, 0xC077, 0x0001, 0x0002, "USB Optical Mouse", true),
            hid(0x046D, 0xC52B, 0x0001, 0x0006, "USB Receiver", true),
            hid(0x046D, 0xC52B, 0xFF00, 0x0002, "USB Receiver", true),
            hid(0x046D, 0xC539, 0x0001, 0x0006, "USB Receiver", true),
            hid(0x046D, 0xC53F, 0xFF00, 0x0002, "USB Receiver", true),
            hid(0x046D, 0xC0FF, 0x0001, 0x0002, "G305 Gaming Mouse", true),
            hid(0x046D, 0xC0FE, 0x0001, 0x0002, "PRO X Wireless", true),
            hid(0x046D, 0xC0FD, 0x0001, 0x0002, "MX Master 3", true),
            hid(0x046D, 0xC0FC, 0x0001, 0x0002, "", true),
            hid(0x1532, 0x0FFE, 0x0001, 0x0002, "Razer Viper", true),
            hid(0x1B1C, 0x0FFE, 0x0001, 0x0006, "Keyboard", true),
        ];
        let found = usb(&office);
        // Receivers without lighting stay, for their battery and device info.
        assert!(
            found
                .iter()
                .filter(|f| f.wireless)
                .all(|f| f.id.contains("c52b") || f.id.contains("c53f"))
        );
        let ids: Vec<String> = found.into_iter().map(|f| f.id).collect();
        assert_eq!(
            ids,
            [
                "usb:046d:c0fc",
                "usb:046d:c0fe",
                "usb:046d:c0ff",
                "usb:046d:c52b",
                "usb:046d:c539",
                "usb:046d:c53f",
                "usb:1532:0ffe"
            ]
        );

        let stick = |maker: &str, part: &str, slot: &str| PhysicalMemory {
            manufacturer: Some(maker.into()),
            part_number: Some(part.into()),
            capacity: Some(17_179_869_184),
            device_locator: Some(slot.into()),
            memory_type: Some(26),
        };
        for slot in ["DIMM_A2", "DIMM_B2"] {
            assert_eq!(module(&stick("Corsair", "CMK32GX4M2Z3600C18", slot)), None);
        }
        let lit = [
            ("Corsair", "CMW32GX4M2E3200C16"),
            ("Corsair", "CMT32GX5M2B5600C36"),
            ("Corsair", "CMD16GX4M2B3200C16"),
            ("G.Skill", "F4-3600C16D-32GTZR"),
            ("G.Skill", "F4-3600C18D-32GTZN"),
            ("G.Skill", "F5-6000J3038F16GX2-TZ5RK"),
            ("G.Skill", "F5-6000J3038F16GX2-TZ5RS"),
            ("Crucial", "BL8G36C16U4BL.M8FE1"),
            ("Kingston", "KF432C16BBA/16"),
            ("Patriot", "PVSR416G320C6K"),
        ];
        let plain = [
            ("G.Skill", "F4-3200C16D-16GVK"),
            ("G.Skill", "F4-3200C16D-16GTZ"),
            ("G.Skill", "F4-3200C16D-16GTZKW"),
            ("G.Skill", "F5-6000J3636F16GX2-TZ5K"),
            ("G.Skill", "F5-6000J3636F16GX2-TZ5S"),
            ("Crucial", "BL8G32C16U4B.M8FE1"),
            ("Crucial", "CT16G4DFRA32A.C16FP"),
            ("Kingston", "KVR32N22S8/8"),
            ("ADATA", "AX4U320016G16A-SB10"),
            ("TeamGroup", "TLZGD416G3200HC16CDC01"),
        ];
        for (maker, part) in lit {
            assert!(module(&stick(maker, part, "A1")).is_some(), "{part}");
        }
        for (maker, part) in plain {
            assert_eq!(module(&stick(maker, part, "A1")), None, "{part}");
        }

        let asus = BaseBoard {
            manufacturer: Some("ASUSTeK COMPUTER INC.".into()),
            product: Some("PRIME X370-PRO".into()),
        };
        let found = vec![board(&asus).unwrap()];
        let off = Settings::default();
        let live = Live {
            settings: &off,
            driven: Vec::new(),
            openrgb_on: false,
        };
        let list = resolve(&found, &live);
        assert_eq!(list[0].name, "ASUS PRIME X370-PRO");
        assert_eq!(
            (list[0].route, list[0].state),
            (Route::None, State::Unsupported)
        );
        assert_eq!(
            list[0].note,
            "FoxDen does not control lighting on it. If it has lighting, send its device info."
        );
    }

    fn part(id: &str, kind: &'static str, bus: Bus, brand: Option<&'static str>) -> Found {
        Found {
            id: id.into(),
            name: id.into(),
            kind,
            bus,
            native: None,
            brand,
            wireless: false,
        }
    }

    #[test]
    fn onboard_only_devices_do_not_point_at_openrgb() {
        let live = Live {
            settings: &Settings::default(),
            driven: vec![],
            openrgb_on: false,
        };
        let found = part("usb:0951:172d", "keyboard", Bus::Usb, Some("HyperX"));
        let (route, state, note) = status(&found, &live);
        assert_eq!((route, state), (Route::None, State::Unsupported));
        assert!(note.contains("on the device itself"));
    }

    #[test]
    fn idle_hardware_without_a_driver_is_not_supported_yet() {
        let mut found = usb(&desk());
        found.push(part("memory:x", "memory", Bus::Memory, Some("Corsair")));
        found.push(part("gpu:1002:7550", "gpu", Bus::Gpu, Some("ASUS")));
        let off = Settings::default();
        let bridge = settings(false, true);
        let idle = |openrgb_enabled| Live {
            settings: if openrgb_enabled { &bridge } else { &off },
            driven: Vec::new(),
            openrgb_on: false,
        };
        let list = resolve(&found, &idle(false));
        let line = |id: &str| list.iter().find(|d| d.id == id).unwrap();
        assert_eq!(list.len(), found.len());
        assert_eq!(
            (line("usb:046d:c539").route, line("usb:046d:c539").state),
            (Route::Native, State::Available)
        );
        for id in ["usb:1038:1ffe", "memory:x", "gpu:1002:7550"] {
            assert_eq!(
                (line(id).route, line(id).state),
                (Route::None, State::Unsupported)
            );
            assert!(!line(id).note.contains("OpenRGB"), "{}", line(id).note);
        }
        assert_eq!(
            line("gpu:1002:7550").note,
            "FoxDen does not control lighting on this card. Many cards have none."
        );

        // the bridge's server is listed only once it is switched on
        let list = resolve(&found, &idle(true));
        let server = list.last().unwrap();
        assert_eq!(
            (server.id.as_str(), server.kind, server.bus),
            ("network:openrgb", "server", Bus::Network)
        );
        assert_eq!(
            (server.route, server.state),
            (Route::Openrgb, State::Available)
        );
        assert_eq!(server.name, "OpenRGB server at 127.0.0.1:6742");
    }

    #[test]
    fn hardware_a_backend_drives_is_active() {
        let found = {
            let mut found = usb(&desk());
            found.push(part("memory:x", "memory", Bus::Memory, Some("Corsair")));
            found.push(part("gpu:1002:7550", "gpu", Bus::Gpu, Some("ASUS")));
            found
        };
        let device = |id, name, kind| Device::grid(id, name, kind, [0.0; 4], 1, 2);
        let mouse = device(
            "hid:logitech-c539",
            "Logitech G502 Lightspeed",
            DeviceKind::Mouse,
        );
        let keys = device("openrgb:0", "Logitech G915 TKL", DeviceKind::Keyboard);
        let ram = device("openrgb:1", "ENE DRAM", DeviceKind::Memory);
        let both = settings(true, true);
        let live = Live {
            settings: &both,
            driven: vec![("openrgb", &keys), ("openrgb", &ram), ("hid", &mouse)],
            openrgb_on: true,
        };
        let list = resolve(&found, &live);
        let line = |id: &str| list.iter().find(|d| d.id == id).unwrap();
        assert_eq!(
            (line("usb:046d:c539").route, line("usb:046d:c539").state),
            (Route::Native, State::Active)
        );
        assert_eq!(
            (line("usb:046d:c547").route, line("usb:046d:c547").state),
            (Route::Openrgb, State::Active)
        );
        assert_eq!(
            (line("memory:x").route, line("memory:x").state),
            (Route::Openrgb, State::Active)
        );
        // switched on but not opened
        assert_eq!(
            (line("usb:1462:7d99").route, line("usb:1462:7d99").state),
            (Route::Native, State::Available)
        );
        assert!(line("usb:1462:7d99").note.contains("could not open"));
        assert_eq!(
            (line("usb:1038:1ffe").route, line("usb:1038:1ffe").state),
            (Route::None, State::Unsupported)
        );
        assert_eq!(
            (line("gpu:1002:7550").route, line("gpu:1002:7550").state),
            (Route::None, State::Unsupported)
        );
        assert!(!line("gpu:1002:7550").note.contains("OpenRGB"));
        assert_eq!(line("network:openrgb").state, State::Active);
        assert!(line("network:openrgb").note.contains("device list"));
    }

    #[test]
    fn a_stick_a_native_driver_lights_is_active() {
        let stick = part("memory:x", "memory", Bus::Memory, Some("Acer Predator"));
        let light = Device::grid(
            "smbus:ene-dram-0-71",
            "ENE DRAM",
            DeviceKind::Memory,
            [0.0; 4],
            8,
            1,
        );
        let both = settings(true, true);
        let live = Live {
            settings: &both,
            driven: vec![("smbus", &light)],
            openrgb_on: false,
        };
        let row = &resolve(&[stick], &live)[0];
        assert_eq!((row.route, row.state), (Route::Native, State::Active));
    }

    #[test]
    fn a_board_lit_by_a_ported_controller_is_active() {
        let board = Found {
            native: Some(PORTED),
            ..part("board:MS-7D75", "motherboard", Bus::Board, Some("MSI"))
        };
        let light = Device::grid(
            "hid:msi-board-1462-7d75-0",
            "MSI Mystic Light MS_7D75",
            DeviceKind::Motherboard,
            [0.0; 4],
            1,
            1,
        );
        let both = settings(true, true);
        let live = Live {
            settings: &both,
            driven: vec![("hid", &light)],
            openrgb_on: false,
        };
        let row = &resolve(&[board], &live)[0];
        assert_eq!((row.route, row.state), (Route::Native, State::Active));
    }

    #[test]
    fn known_graphics_cards_go_native() {
        let known = Found {
            native: Some(GPU),
            ..part("gpu:10de:2704", "gpu", Bus::Gpu, None)
        };
        let found = vec![known, part("gpu:10de:2504", "gpu", Bus::Gpu, None)];
        let card = Device::grid(
            "gpu:nvidia-illumination:0",
            "NVIDIA GeForce RTX 4080 FE",
            DeviceKind::Gpu,
            [0.0; 4],
            2,
            1,
        );
        let line = |settings: &Settings, driven| {
            let live = Live {
                settings,
                driven,
                openrgb_on: false,
            };
            resolve(&found, &live)
                .into_iter()
                .map(|d| (d.route, d.state, d.note))
                .collect::<Vec<_>>()
        };
        let on = settings(true, false);
        let list = line(&on, vec![("hid", &card)]);
        assert_eq!((list[0].0, list[0].1), (Route::Native, State::Active));
        assert_eq!((list[1].0, list[1].1), (Route::None, State::Unsupported));
        assert!(list[1].2.contains("does not control lighting"), "{}", list[1].2);
        let list = line(&on, Vec::new());
        assert_eq!((list[0].0, list[0].1), (Route::Native, State::Available));
        let off = settings(false, false);
        let list = line(&off, Vec::new());
        assert_eq!((list[0].0, list[0].1), (Route::Native, State::Available));
        assert!(list[0].2.contains("Switch on native drivers"));
    }

    #[test]
    fn an_empty_openrgb_list_decides_nothing() {
        let found = usb(&desk());
        let on = settings(false, true);
        let live = Live {
            settings: &on,
            driven: Vec::new(),
            openrgb_on: true,
        };
        let list = resolve(&found, &live);
        let line = |id: &str| list.iter().find(|d| d.id == id).unwrap();
        assert_eq!(
            (line("usb:1038:1ffe").route, line("usb:1038:1ffe").state),
            (Route::None, State::Unsupported)
        );
        assert_eq!(line("network:openrgb").state, State::Active);
        assert!(
            line("network:openrgb")
                .note
                .contains("lists no devices yet")
        );
    }

    #[test]
    fn a_usb_device_is_not_matched_to_memory_or_a_card_of_its_brand() {
        // made-up product id; no keyboard or mouse usage, so its kind is "other"
        let found = usb(&[hid(0x1B1C, 1, 0xFF42, 0x0001, "Lighting Node", true)]);
        assert_eq!(found[0].kind, "other");
        let device = |id, name, kind| Device::grid(id, name, kind, [0.0; 4], 1, 2);
        let ram = device("openrgb:0", "Corsair Vengeance RGB", DeviceKind::Memory);
        let node = device("openrgb:1", "Corsair Lighting Node Pro", DeviceKind::Strip);
        let on = settings(false, true);
        let state_with = |driven| {
            let live = Live {
                settings: &on,
                driven,
                openrgb_on: true,
            };
            resolve(&found, &live)[0].state
        };
        assert_eq!(state_with(vec![("openrgb", &ram)]), State::Unsupported);
        assert_eq!(
            state_with(vec![("openrgb", &ram), ("openrgb", &node)]),
            State::Active
        );
    }

    #[test]
    fn detected_serialises_as_the_contract_says() {
        let entry = Detected {
            id: "usb:046d:c539".into(),
            name: "Mouse".into(),
            kind: "mouse",
            bus: Bus::Usb,
            route: Route::None,
            state: State::Unsupported,
            note: String::new(),
        };
        let json = serde_json::to_value(&entry).unwrap();
        let expected = serde_json::json!({
            "id": "usb:046d:c539", "name": "Mouse", "kind": "mouse", "bus": "usb",
            "route": "none", "state": "unsupported", "note": "",
        });
        assert_eq!(json, expected);
        let buses = [Bus::Usb, Bus::Board, Bus::Memory, Bus::Gpu, Bus::Network];
        assert_eq!(
            serde_json::to_string(&buses).unwrap(),
            r#"["usb","board","memory","gpu","network"]"#
        );
        let routes = [Route::Native, Route::Openrgb, Route::None];
        assert_eq!(
            serde_json::to_string(&routes).unwrap(),
            r#"["native","openrgb","none"]"#
        );
        let states = [State::Active, State::Available, State::Unsupported];
        assert_eq!(
            serde_json::to_string(&states).unwrap(),
            r#"["active","available","unsupported"]"#
        );
    }
}
