//! Ported from OpenRGB Controllers/HIDLampArrayController (GPL-2.0-or-later).

use std::collections::{BTreeMap, BTreeSet};

use foxden_core::{Backend, Color, Device, DeviceKind};
use hidapi::HidApi;

use super::{Link, rect};
use crate::registry::{Info, Matcher};

pub const MATCHERS: [Matcher; 1] = [Matcher {
    name: "HID LampArray Device",
    vid: None,
    interface: None,
    usage_page: Some(0x59),
    usage: Some(0x01),
    bus: None,
    open,
}];

/// `HID_API_MAX_REPORT_DESCRIPTOR_SIZE`.
const MAX_DESCRIPTOR: usize = 4096;
const ATTRIBUTES_LEN: usize = 22;
const LAMP_LEN: usize = 28;
const MULTI_COUNT: usize = 8;
const MULTI_LEN: usize = 2 + MULTI_COUNT * 2 + MULTI_COUNT * 4;
const UPDATE_COMPLETE: u8 = 1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ids {
    pub attributes: u8,
    pub request: u8,
    pub response: u8,
    pub multi: u8,
    pub range: u8,
    pub control: u8,
}

/// `get_hid_item_size`: (data length, key size).
fn item_size(desc: &[u8], pos: usize) -> (usize, usize) {
    let key = desc[pos];
    if key & 0xF0 == 0xF0 && pos + 1 < desc.len() {
        return (desc[pos + 1] as usize, 3);
    }
    match key & 0x3 {
        3 => (4, 1),
        code => (code as usize, 1),
    }
}

/// `get_hid_report_bytes`.
fn report_bytes(desc: &[u8], num: usize, cur: usize) -> u32 {
    if cur + num >= desc.len() {
        return 0;
    }
    match num {
        1 => desc[cur + 1] as u32,
        2 => desc[cur + 2] as u32 * 256 + desc[cur + 1] as u32,
        4 => u32::from_le_bytes([desc[cur + 1], desc[cur + 2], desc[cur + 3], desc[cur + 4]]),
        _ => 0,
    }
}

/// Report ids of the LampArray reports, read from the report descriptor as the constructor does.
/// The usage page is kept in a byte there, so a 16-bit page is truncated; mirrored.
pub fn parse(desc: &[u8]) -> Ids {
    let mut curr_collection_usage = 0u32;
    let mut report_id = 0u8;
    let mut usage = 0u32;
    let mut usage_page = 0u8;
    let mut map: BTreeMap<u32, u8> = BTreeMap::new();
    let mut pos = 0usize;
    while pos < desc.len() {
        let (data_len, key_size) = item_size(desc, pos);
        let key = desc[pos];
        match key {
            0xA1 => {
                curr_collection_usage = usage;
                if usage_page == 0x59 && report_id != 0 {
                    map.insert(curr_collection_usage, report_id);
                }
            }
            0xC0 => {
                curr_collection_usage = 0;
                report_id = 0;
            }
            _ => {}
        }
        match key & 0xFC {
            0x04 => usage_page = report_bytes(desc, data_len, pos) as u8,
            0x08 => {
                usage = report_bytes(desc, data_len, pos);
                if data_len == 4 {
                    let local_page = usage >> 16;
                    usage &= 0xFFFF;
                    if local_page == 0x59 && report_id != 0 {
                        map.insert(usage, report_id);
                    }
                }
            }
            0x84 => {
                report_id = report_bytes(desc, data_len, pos) as u8;
                if usage_page == 0x59 && curr_collection_usage != 0 {
                    map.insert(curr_collection_usage, report_id);
                }
            }
            _ => {}
        }
        pos += data_len + key_size;
    }
    let mut ids = Ids::default();
    for (usage, id) in map {
        match usage {
            0x02 => ids.attributes = id,
            0x20 => ids.request = id,
            0x22 => ids.response = id,
            0x50 => ids.multi = id,
            0x60 => ids.range = id,
            0x70 => ids.control = id,
            _ => {}
        }
    }
    ids
}

impl Ids {
    fn complete(&self) -> bool {
        [
            self.attributes,
            self.request,
            self.response,
            self.multi,
            self.range,
            self.control,
        ]
        .iter()
        .all(|&id| id != 0)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Lamp {
    pub id: u16,
    pub x: u32,
    pub y: u32,
    pub key: u8,
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// What the constructor learns: report ids, LampArrayKind and every lamp.
#[derive(Debug, PartialEq, Eq)]
pub struct Found {
    pub ids: Ids,
    pub kind: u32,
    pub lamps: Vec<Lamp>,
}

/// `HIDLampArrayController::HIDLampArrayController`. `None` when the detector would drop it.
pub fn discover(link: &mut impl Link) -> Option<Found> {
    let mut desc = vec![0u8; MAX_DESCRIPTOR];
    let size = link.descriptor(&mut desc).ok()?;
    let ids = parse(&desc[..size]);
    if !ids.complete() {
        return None;
    }
    let mut buf = [0u8; ATTRIBUTES_LEN + 1];
    buf[0] = ids.attributes;
    let _ = link.get_feature(&mut buf);
    let count = u16_at(&buf, 1);
    let kind = u32_at(&buf, 15);
    let _ = link.send_feature(&[ids.request, 0, 0]);
    let mut lamps = Vec::new();
    for _ in 0..count {
        let mut buf = [0u8; LAMP_LEN + 1];
        buf[0] = ids.response;
        let _ = link.get_feature(&mut buf);
        lamps.push(Lamp {
            id: u16_at(&buf, 1),
            x: u32_at(&buf, 3),
            y: u32_at(&buf, 7),
            key: buf[28],
        });
    }
    (count > 0).then_some(Found { ids, kind, lamps })
}

/// `SetLampArrayControlReport`.
pub fn control(ids: &Ids, autonomous: bool) -> Vec<u8> {
    vec![ids.control, autonomous as u8]
}

/// `DeviceUpdateLEDs`: LampMultiUpdate reports of eight lamps, ids in LED order, full intensity.
pub fn frame(ids: &Ids, colors: &[Color]) -> Vec<Vec<u8>> {
    let total = colors.len();
    let mut out = Vec::new();
    let mut to_go = total;
    while to_go > 0 {
        let n = to_go.min(MULTI_COUNT);
        let start = total - to_go;
        to_go -= n;
        let mut buf = vec![0u8; MULTI_LEN + 1];
        buf[0] = ids.multi;
        buf[1] = n as u8;
        buf[2] = if to_go == 0 { UPDATE_COMPLETE } else { 0 };
        for i in 0..n {
            let led = (start + i) as u16;
            buf[3 + i * 2..5 + i * 2].copy_from_slice(&led.to_le_bytes());
            let c = colors[start + i];
            let at = 3 + MULTI_COUNT * 2 + i * 4;
            buf[at..at + 4].copy_from_slice(&[c.0, c.1, c.2, 255]);
        }
        out.push(buf);
    }
    out
}

/// `SetupZones`: each lamp at its cell of a matrix built from the distinct X and Y positions.
fn layout(lamps: &[Lamp]) -> (Vec<[f32; 2]>, [u32; 2]) {
    let xs: BTreeSet<u32> = lamps.iter().map(|l| l.x).collect();
    let ys: BTreeSet<u32> = lamps.iter().map(|l| l.y).collect();
    let (w, h) = (xs.len().max(1), ys.len().max(1));
    let leds = lamps
        .iter()
        .map(|l| {
            let col = xs.range(..l.x).count();
            let row = ys.range(..l.y).count();
            [(col as f32 + 0.5) / w as f32, (row as f32 + 0.5) / h as f32]
        })
        .collect();
    (leds, [w as u32, h as u32])
}

fn kind(lamp_array_kind: u32) -> DeviceKind {
    match lamp_array_kind {
        0x01 => DeviceKind::Keyboard,
        0x02 => DeviceKind::Mouse,
        _ => DeviceKind::Other,
    }
}

pub struct LampArray<L: Link> {
    link: L,
    device: Device,
    ids: Ids,
    sent: Option<Vec<Color>>,
}

impl<L: Link> LampArray<L> {
    pub fn new(mut link: L, id: String, name: &str) -> Option<Self> {
        let found = discover(&mut link)?;
        let (leds, grid) = layout(&found.lamps);
        let kind = kind(found.kind);
        let name = if name.is_empty() {
            "HID LampArray Device"
        } else {
            name
        };
        Some(LampArray {
            link,
            device: Device {
                id,
                name: name.into(),
                kind,
                rect: rect(kind),
                grid,
                leds,
            },
            ids: found.ids,
            sent: None,
        })
    }

    fn send(&mut self, colors: &[Color]) -> Result<(), String> {
        if self.sent.is_none() {
            self.link.send_feature(&control(&self.ids, false))?;
        }
        for report in frame(&self.ids, colors) {
            self.link.send_feature(&report)?;
        }
        Ok(())
    }
}

impl<L: Link + 'static> Backend for LampArray<L> {
    fn name(&self) -> &str {
        "HID LampArray"
    }

    fn devices(&self) -> Vec<Device> {
        vec![self.device.clone()]
    }

    fn push(&mut self, device_id: &str, colors: &[Color]) -> Result<(), String> {
        if device_id != self.device.id {
            return Err(format!("unknown device {device_id}"));
        }
        if self.sent.as_deref() == Some(colors) {
            return Ok(());
        }
        let colors: Vec<Color> = (0..self.device.leds.len())
            .map(|i| colors.get(i).copied().unwrap_or_default())
            .collect();
        match self.send(&colors) {
            Ok(()) => {
                self.sent = Some(colors);
                Ok(())
            }
            Err(e) => {
                self.sent = None;
                Err(e)
            }
        }
    }
}

impl<L: Link> LampArray<L> {
    /// Hands the lamps back to the device's own autonomous effect.
    fn release(&mut self) {
        if self.sent.take().is_some() {
            let _ = self.link.send_feature(&control(&self.ids, true));
        }
    }
}

impl<L: Link> Drop for LampArray<L> {
    fn drop(&mut self) {
        self.release();
    }
}

fn open(api: &HidApi, info: &Info, _claimed: &mut Vec<String>) -> Vec<Box<dyn Backend>> {
    let Ok(hid) = info.open(api) else {
        return Vec::new();
    };
    LampArray::new(hid, info.device_id("lamparray"), &info.product)
        .map(|d| Box::new(d) as Box<dyn Backend>)
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Ev, Fake};
    use super::*;

    /// A LampArray collection as the HID Usage Tables example lays it out, report ids 1..6.
    fn descriptor() -> Vec<u8> {
        let mut d = vec![0x05, 0x59, 0x09, 0x01, 0xA1, 0x01];
        for (id, usage) in [
            (1u8, 0x02u8),
            (2, 0x20),
            (3, 0x22),
            (4, 0x50),
            (5, 0x60),
            (6, 0x70),
        ] {
            d.extend([0x85, id, 0x09, usage, 0xA1, 0x02, 0x75, 0x08, 0xC0]);
        }
        d.push(0xC0);
        d
    }

    fn attributes(count: u16, kind: u32) -> Vec<u8> {
        let mut b = vec![1u8];
        b.extend(count.to_le_bytes());
        b.extend([0u8; 12]);
        b.extend(kind.to_le_bytes());
        b.extend(16u32.to_le_bytes());
        b
    }

    fn lamp(id: u16, x: u32, y: u32, key: u8) -> Vec<u8> {
        let mut b = vec![3u8];
        b.extend(id.to_le_bytes());
        b.extend(x.to_le_bytes());
        b.extend(y.to_le_bytes());
        b.extend([0u8; 12]);
        b.extend([255, 255, 255, 1, 1, key]);
        b
    }

    fn device(lamps: &[(u32, u32)]) -> Fake {
        let mut f = Fake::new(|_| Vec::new());
        f.desc = descriptor();
        f.features
            .insert(1, [attributes(lamps.len() as u16, 1)].into());
        f.features.insert(
            3,
            lamps
                .iter()
                .enumerate()
                .map(|(i, &(x, y))| lamp(i as u16, x, y, 4 + i as u8))
                .collect(),
        );
        f
    }

    // HIDLampArrayController::HIDLampArrayController report id parse.
    #[test]
    fn parses_report_ids() {
        let ids = parse(&descriptor());
        assert_eq!(
            ids,
            Ids {
                attributes: 1,
                request: 2,
                response: 3,
                multi: 4,
                range: 5,
                control: 6
            }
        );
    }

    // Same parse: a 32-bit usage carrying page 0x59 maps by itself; the byte-wide page truncates 0xFF59.
    #[test]
    fn parse_quirks_match_cpp() {
        let d = [0x85, 0x07, 0x0B, 0x50, 0x00, 0x59, 0x00];
        assert_eq!(parse(&d).multi, 7);
        let d = [0x06, 0x59, 0xFF, 0x09, 0x02, 0xA1, 0x02, 0x85, 0x09];
        assert_eq!(parse(&d).attributes, 9);
        let d = [0x05, 0x01, 0x09, 0x02, 0xA1, 0x02, 0x85, 0x09];
        assert_eq!(parse(&d).attributes, 0);
        assert_eq!(report_bytes(&[0x05, 0x59], 1, 0), 0x59);
        assert_eq!(report_bytes(&[0x05], 1, 0), 0);
    }

    // HIDLampArrayController constructor: attributes, request lamp 0, one response per lamp.
    #[test]
    fn detection_reads() {
        let mut f = device(&[(0, 0), (19_000, 0), (0, 19_000)]);
        let found = discover(&mut f).unwrap();
        assert_eq!(
            f.log,
            [
                Ev::GetFeature(1, 23),
                Ev::SendFeature(vec![2, 0, 0]),
                Ev::GetFeature(3, 29),
                Ev::GetFeature(3, 29),
                Ev::GetFeature(3, 29),
            ]
        );
        assert_eq!(found.kind, 1);
        assert_eq!(
            found.lamps[1],
            Lamp {
                id: 1,
                x: 19_000,
                y: 0,
                key: 5
            }
        );
    }

    // DetectHIDLampArrayControllers drops a controller with missing reports or no lamps.
    #[test]
    fn detection_drops_empty_arrays() {
        let mut f = device(&[]);
        assert_eq!(discover(&mut f), None);
        let mut f = device(&[(0, 0)]);
        f.desc.truncate(f.desc.len() - 10);
        assert_eq!(discover(&mut f), None);
        assert!(f.log.is_empty());
    }

    // RGBController_HIDLampArray::DeviceUpdateMode(Direct) then DeviceUpdateLEDs for 10 lamps.
    #[test]
    fn direct_setup_and_frame() {
        let lamps: Vec<(u32, u32)> = (0..10).map(|i| (i * 1000, 0)).collect();
        let mut d = LampArray::new(device(&lamps), "id".into(), "Board").unwrap();
        d.link.log.clear();
        let colors: Vec<Color> = (0..10).map(|i| Color(i, 0x10, 0x20)).collect();
        d.push("id", &colors).unwrap();
        let mut first = vec![0u8; 51];
        first[..3].copy_from_slice(&[4, 8, 0]);
        for i in 0..8u8 {
            first[3 + i as usize * 2] = i;
            first[19 + i as usize * 4..23 + i as usize * 4].copy_from_slice(&[i, 0x10, 0x20, 255]);
        }
        let mut last = vec![0u8; 51];
        last[..7].copy_from_slice(&[4, 2, 1, 8, 0, 9, 0]);
        last[19..27].copy_from_slice(&[8, 0x10, 0x20, 255, 9, 0x10, 0x20, 255]);
        assert_eq!(
            d.link.log,
            [
                Ev::SendFeature(vec![6, 0]),
                Ev::SendFeature(first),
                Ev::SendFeature(last)
            ]
        );
        d.link.log.clear();
        d.push("id", &colors).unwrap();
        assert!(d.link.log.is_empty());
        let mut changed = colors.clone();
        changed[0] = Color::BLACK;
        d.push("id", &changed).unwrap();
        assert_eq!(d.link.log.len(), 2);
    }

    // Autonomous mode (DeviceUpdateMode value 1) when the backend lets go.
    #[test]
    fn release_restores_autonomous_mode() {
        let mut d = LampArray::new(device(&[(0, 0)]), "id".into(), "").unwrap();
        d.release();
        assert!(d.link.log.iter().all(|e| *e != Ev::SendFeature(vec![6, 1])));
        d.push("id", &[Color(1, 2, 3)]).unwrap();
        d.link.log.clear();
        d.release();
        assert_eq!(d.link.log, [Ev::SendFeature(vec![6, 1])]);
    }

    // SetupZones matrix: columns and rows are the distinct positions.
    #[test]
    fn layout_uses_matrix_cells() {
        let lamps = [(0, 0), (5, 0), (0, 9)].map(|(x, y)| Lamp {
            x,
            y,
            ..Lamp::default()
        });
        let (leds, grid) = layout(&lamps);
        assert_eq!(grid, [2, 2]);
        assert_eq!(leds, [[0.25, 0.25], [0.75, 0.25], [0.25, 0.75]]);
    }
}
