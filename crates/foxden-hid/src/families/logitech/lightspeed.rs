// SPDX-License-Identifier: GPL-2.0-or-later

use std::collections::BTreeMap;
use std::thread::sleep;
use std::time::Duration;

use foxden_core::{Backend, Color, Device, DeviceKind};
use hidapi::HidApi;

use super::{FAMILY, Hid, at, open_sibling};
use crate::registry::{Detector, Info};

const TIMEOUT: i32 = 300;
const RECEIVER: u8 = 0xFF;
const MAX_RETRY: usize = 10;
const GET_REGISTER: u8 = 0x81;
const SET_REGISTER: u8 = 0x80;
const PAGE_NAME: u16 = 0x0005;
const PAGE_8070: u16 = 0x8070;
const PAGE_8071: u16 = 0x8071;
const LED_ON: u16 = 0x0001;

/// A 20-byte HID++ request. The function bytes here carry their own software id, if any.
pub fn long(device: u8, feature: u8, command: u8, data: &[u8]) -> [u8; 20] {
    let mut r = [0u8; 20];
    r[..4].copy_from_slice(&[0x11, device, feature, command]);
    r[4..4 + data.len()].copy_from_slice(data);
    r
}

/// A 7-byte HID++ request, used for receiver registers.
pub fn short(device: u8, feature: u8, command: u8, data: &[u8]) -> [u8; 7] {
    let mut r = [0u8; 7];
    r[..4].copy_from_slice(&[0x10, device, feature, command]);
    r[4..4 + data.len()].copy_from_slice(data);
    r
}

/// getFeatureIndex: asks the root feature where `page` lives.
pub fn get_feature(device: u8, page: u16) -> [u8; 20] {
    long(device, 0x00, 0x01, &page.to_be_bytes())
}

/// setDirectMode(true): LOGITECH_FP8070_SET_SW_CTL, sent on either effects page.
pub fn sw_control(device: u8, rgb: u8) -> [u8; 20] {
    long(device, rgb, 0x80, &[0x01, 0x01])
}

/// set8071Effects(5): hands RGB and power control to software on 0x8071.
pub fn effects_8071(device: u8, rgb: u8) -> [u8; 20] {
    long(device, rgb, 0x50, &[0x01, 0x03, 0x05])
}

/// setMode for Direct: the zone's LED_ON effect in one colour.
pub fn set_zone(node: &Node, zone: u8, c: Color) -> [u8; 20] {
    let fp8070 = node.page == PAGE_8070;
    let mut data = [0u8; 13];
    data[..5].copy_from_slice(&[zone, node.direct, c.0, c.1, c.2]);
    data[12] = if fp8070 { 0x00 } else { 0x01 };
    long(
        node.index,
        node.rgb,
        if fp8070 { 0x30 } else { 0x10 },
        &data,
    )
}

/// One HID++ device: what initialiseDevice and getRGBconfig learn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub index: u8,
    pub name: String,
    pub kind: u8,
    pub rgb: u8,
    pub page: u16,
    /// Effect modes of each LED, keyed by LED number.
    pub leds: BTreeMap<u8, Vec<u16>>,
    /// Index of LED 0's LED_ON effect, which Direct uses.
    pub direct: u8,
}

fn read(hid: &dyn Hid) -> ([u8; 64], usize) {
    let mut buf = [0u8; 64];
    let n = hid.read(&mut buf, TIMEOUT).unwrap_or(0);
    (buf, n)
}

fn request(hid: &dyn Hid, report: &[u8]) -> ([u8; 64], usize) {
    if hid.write(report).is_err() {
        return ([0u8; 64], 0);
    }
    read(hid)
}

/// flushReadQueue.
fn flush(hid: &dyn Hid) {
    for _ in 0..64 {
        if read(hid).1 == 0 {
            break;
        }
    }
}

/// connected(): a paired device that is off answers the register read with ERR_RESOURCE_ERROR (9).
fn connected(short_hid: &dyn Hid, index: u8) -> bool {
    let req = short(index, GET_REGISTER, 0x02, &[]);
    let mut buf = [0u8; 64];
    buf[..7].copy_from_slice(&req);
    if short_hid.write(&req).is_err() {
        return false;
    }
    let _ = short_hid.read(&mut buf[..7], TIMEOUT);
    buf[5] != 0x09
}

fn feature_index(hid: &dyn Hid, index: u8, page: u16) -> u8 {
    request(hid, &get_feature(index, page)).0[4]
}

/// getDeviceName: the name in 16-byte pieces, then the device type.
fn name_and_type(hid: &dyn Hid, index: u8) -> (String, u8) {
    let f = feature_index(hid, index, PAGE_NAME);
    if f == 0 {
        return (String::new(), 0xFF);
    }
    let len = request(hid, &long(index, f, 0x01, &[])).0[4] as usize;
    let mut name = String::new();
    while name.len() < len {
        let (buf, _) = request(hid, &long(index, f, 0x11, &[name.len() as u8]));
        let piece: Vec<u8> = buf[4..].iter().copied().take_while(|&b| b != 0).collect();
        if piece.is_empty() {
            break;
        }
        name.push_str(&String::from_utf8_lossy(&piece));
    }
    let kind = request(hid, &long(index, f, 0x21, &[])).0[4];
    (name, kind)
}

/// getRGBconfig: the LEDs and the effects each one offers.
fn rgb_config(hid: &dyn Hid, index: u8, rgb: u8, page: u16) -> BTreeMap<u8, Vec<u16>> {
    let mut leds = BTreeMap::new();
    let mode = |buf: &[u8; 64]| (buf[6] as u16) << 8 | buf[7] as u16;
    if page == PAGE_8070 {
        let _ = hid.write(&long(index, rgb, 0x00, &[]));
        let mut resp = read(hid);
        for _ in 0..8 {
            let (b, n) = resp;
            if !(n == 20 && b[2] != rgb && b[3] != 0x00) {
                break;
            }
            resp = read(hid);
        }
        let count = resp.0[4];
        for i in 0..count {
            let (b, n) = request(hid, &long(index, rgb, 0x10, &[i]));
            if n == 20 && b[2] == rgb && b[3] == 0x10 && b[4] != 0x10 && b[5] != 0x02 {
                let fx = (0..b[7])
                    .map(|j| mode(&request(hid, &long(index, rgb, 0x20, &[i, j])).0))
                    .collect();
                leds.entry(b[4]).or_insert(fx);
            }
        }
    } else if page == PAGE_8071 {
        let (b, _) = request(hid, &long(index, rgb, 0x00, &[0xFF, 0xFF, 0, 0, 0]));
        // the LED count is kept in a byte
        let count = ((b[5] as u16) << 8 | b[6] as u16) as u8;
        for i in 0..count {
            let (b, _) = request(hid, &long(index, rgb, 0x00, &[i, 0xFF, 0, 0, 0]));
            let fx = (0..b[8])
                .map(|j| mode(&request(hid, &long(index, rgb, 0x00, &[i, j])).0))
                .collect();
            leds.entry(b[4]).or_insert(fx);
        }
    }
    leds
}

/// is_valid's checks on the name, type, LEDs and effects index.
fn valid(name: &str, kind: u8, leds: usize, rgb: u8) -> bool {
    let first = name.as_bytes().first().copied().unwrap_or(0);
    !name.is_empty()
        && kind <= 8
        && (32..122).contains(&first)
        && name.len() > 3
        && leds > 0
        && rgb > 0
}

/// One pass of logitech_device's constructor and is_valid. `short` is set behind a receiver.
pub fn discover(short_hid: Option<&dyn Hid>, long_hid: &dyn Hid, index: u8) -> Option<Node> {
    if let Some(s) = short_hid {
        if !connected(s, index) {
            return None;
        }
        flush(s);
    }
    flush(long_hid);
    let (name, kind) = name_and_type(long_hid, index);
    let (rgb, page) = [PAGE_8070, PAGE_8071]
        .into_iter()
        .map(|p| (feature_index(long_hid, index, p), p))
        .find(|&(f, _)| f > 0)?;
    let leds = rgb_config(long_hid, index, rgb, page);
    if let Some(s) = short_hid
        && !connected(s, index)
    {
        return None;
    }
    if !valid(&name, kind, leds.len(), rgb) {
        return None;
    }
    // RGBController_LogitechLightspeed maps Direct to LED 0's LED_ON effect
    let direct = leds.get(&0)?.iter().position(|&m| m == LED_ON)? as u8;
    Some(Node {
        index,
        name,
        kind,
        rgb,
        page,
        leds,
        direct,
    })
}

/// CreateLogitechLightspeedDevice's retry loop.
fn discover_retrying(short_hid: Option<&dyn Hid>, long_hid: &dyn Hid, index: u8) -> Option<Node> {
    for _ in 1..MAX_RETRY {
        sleep(Duration::from_millis(50));
        if let Some(node) = discover(short_hid, long_hid, index) {
            return Some(node);
        }
    }
    None
}

/// getWirelessDevice: the receiver fakes a reconnect of its paired devices; virtual PID to index.
pub fn paired(short_hid: &dyn Hid) -> BTreeMap<u16, u8> {
    let mut found = BTreeMap::new();
    let (b, _) = request(short_hid, &short(RECEIVER, GET_REGISTER, 0x00, &[]));
    if b[5] & 1 == 0 {
        request(
            short_hid,
            &short(RECEIVER, SET_REGISTER, 0x00, &[0x00, 0x01]),
        );
    }
    let count = request(short_hid, &short(RECEIVER, GET_REGISTER, 0x02, &[])).0[5] as usize;
    if count == 0
        || short_hid
            .write(&short(RECEIVER, SET_REGISTER, 0x02, &[0x02]))
            .is_err()
    {
        return found;
    }
    for _ in 0..count + 1 {
        let (b, n) = read(short_hid);
        if n > 0 && b[1] != RECEIVER {
            found
                .entry((b[6] as u16) << 8 | b[5] as u16)
                .or_insert(b[1]);
        }
    }
    found
}

struct Slot {
    node: Node,
    device: Device,
    sent: Option<Vec<Color>>,
}

fn device(id: String, node: &Node) -> Device {
    let n = node.leds.len() as u32;
    let (kind, rect, cols, rows) = match node.kind {
        0 => (DeviceKind::Keyboard, [0.06, 0.58, 0.54, 0.32], n, 1),
        3 => (DeviceKind::Mouse, [0.70, 0.62, 0.06, 0.22], 1, n),
        4 => (DeviceKind::Other, [0.62, 0.55, 0.34, 0.40], n, 1),
        _ => (DeviceKind::Other, [0.82, 0.12, 0.08, 0.16], 1, n),
    };
    Device::grid(&id, &node.name, kind, rect, cols, rows)
}

pub struct Lightspeed {
    /// Held so the usage-1 collection stays open while the receiver is in use.
    _short: Option<Box<dyn Hid>>,
    long: Box<dyn Hid>,
    slots: Vec<Slot>,
}

impl Lightspeed {
    fn new(short: Option<Box<dyn Hid>>, long: Box<dyn Hid>, nodes: Vec<(String, Node)>) -> Self {
        let slots = nodes
            .into_iter()
            .map(|(id, node)| Slot {
                device: device(id, &node),
                node,
                sent: None,
            })
            .collect();
        Lightspeed {
            _short: short,
            long,
            slots,
        }
    }
}

fn ack(hid: &dyn Hid, report: &[u8]) -> Result<(), String> {
    hid.write(report)?;
    read(hid);
    Ok(())
}

/// DetectLogitechWired: the device answers HID++ at index 0xFF on its own collection.
pub fn open_wired(api: &HidApi, info: &Info, _det: &Detector) -> Result<Box<dyn Backend>, String> {
    let hid = info.open(api)?;
    let node = discover_retrying(None, &hid, RECEIVER).ok_or("no direct-capable HID++ device")?;
    Ok(Box::new(Lightspeed::new(
        None,
        Box::new(hid),
        vec![(info.device_id(FAMILY), node)],
    )))
}

/// DetectLogitechLightspeedReceiver: every paired device, short requests on usage 1, long on usage 2.
pub fn open_receiver(
    api: &HidApi,
    info: &Info,
    _det: &Detector,
) -> Result<Box<dyn Backend>, String> {
    let short_hid = open_sibling(api, info, 0xFF00, 1)?;
    let long_hid = open_sibling(api, info, 0xFF00, 2)?;
    let base = info.device_id(FAMILY);
    let nodes: Vec<_> = paired(&short_hid)
        .into_values()
        .filter_map(|index| {
            discover_retrying(Some(&short_hid), &long_hid, index)
                .map(|node| (format!("{base}-{index:02x}"), node))
        })
        .collect();
    if nodes.is_empty() {
        return Err("no paired device answered".into());
    }
    Ok(Box::new(Lightspeed::new(
        Some(Box::new(short_hid)),
        Box::new(long_hid),
        nodes,
    )))
}

impl Backend for Lightspeed {
    fn name(&self) -> &str {
        "logitech lightspeed"
    }

    fn devices(&self) -> Vec<Device> {
        self.slots.iter().map(|s| s.device.clone()).collect()
    }

    fn push(&mut self, device_id: &str, colors: &[Color]) -> Result<(), String> {
        let hid = self.long.as_ref();
        let slot = self
            .slots
            .iter_mut()
            .find(|s| s.device.id == device_id)
            .ok_or_else(|| format!("unknown device {device_id}"))?;
        if slot.sent.as_deref() == Some(colors) {
            return Ok(());
        }
        let node = &slot.node;
        let first = slot.sent.take().is_none();
        if first {
            if node.page == PAGE_8071 {
                ack(hid, &effects_8071(node.index, node.rgb))?;
            }
            ack(hid, &sw_control(node.index, node.rgb))?;
        }
        for zone in 0..node.leds.len() {
            ack(hid, &set_zone(node, zone as u8, at(colors, zone)))?;
        }
        slot.sent = Some(colors.to_vec());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Fake, Log, take};
    use super::*;

    fn reply(bytes: &[u8]) -> Vec<u8> {
        let mut r = vec![0u8; 20];
        r[..bytes.len()].copy_from_slice(bytes);
        r
    }

    /// A wired 8070 mouse named "G502 HERO Gaming Mouse" with two LEDs offering Off and On.
    fn mouse_8070(req: &[u8]) -> Vec<Vec<u8>> {
        let name = b"G502 HERO Gaming Mouse";
        let r = match req[..4] {
            [0x11, 0xFF, 0x00, 0x01] => match (req[4], req[5]) {
                (0x00, 0x05) => reply(&[0x11, 0xFF, 0x00, 0x01, 0x04]),
                (0x80, 0x70) => reply(&[0x11, 0xFF, 0x00, 0x01, 0x0B]),
                _ => reply(&[0x11, 0xFF, 0x00, 0x01, 0x00]),
            },
            [0x11, 0xFF, 0x04, 0x01] => reply(&[0x11, 0xFF, 0x04, 0x01, name.len() as u8]),
            [0x11, 0xFF, 0x04, 0x11] => {
                let from = req[4] as usize;
                let to = (from + 16).min(name.len());
                let mut r = reply(&[0x11, 0xFF, 0x04, 0x11]);
                r[4..4 + to - from].copy_from_slice(&name[from..to]);
                r
            }
            [0x11, 0xFF, 0x04, 0x21] => reply(&[0x11, 0xFF, 0x04, 0x21, 0x03]),
            [0x11, 0xFF, 0x0B, 0x00] => reply(&[0x11, 0xFF, 0x0B, 0x00, 0x02]),
            [0x11, 0xFF, 0x0B, 0x10] => reply(&[0x11, 0xFF, 0x0B, 0x10, req[4], 0x00, 0x01, 0x02]),
            [0x11, 0xFF, 0x0B, 0x20] => {
                reply(&[0x11, 0xFF, 0x0B, 0x20, req[4], req[5], 0x00, req[5]])
            }
            _ => return Vec::new(),
        };
        vec![r]
    }

    fn hex(s: &str) -> Vec<u8> {
        let mut r = vec![0u8; 20];
        for (i, b) in s.split_whitespace().enumerate() {
            r[i] = u8::from_str_radix(b, 16).unwrap();
        }
        r
    }

    // getFeatureIndex, setDirectMode and set8071Effects requests.
    #[test]
    fn request_bytes() {
        assert_eq!(get_feature(0xFF, 0x8070).to_vec(), hex("11 FF 00 01 80 70"));
        assert_eq!(sw_control(0x01, 0x0B).to_vec(), hex("11 01 0B 80 01 01"));
        assert_eq!(
            effects_8071(0xFF, 0x0A).to_vec(),
            hex("11 FF 0A 50 01 03 05")
        );
        assert_eq!(
            short(0xFF, 0x81, 0x02, &[]),
            [0x10, 0xFF, 0x81, 0x02, 0, 0, 0]
        );
    }

    // logitech_device::setMode for Direct (LED_ON): no speed or brightness, data[12] is the 8071 persistence.
    #[test]
    fn set_zone_bytes() {
        let mut node = Node {
            index: 0xFF,
            name: "Mouse".into(),
            kind: 3,
            rgb: 0x0B,
            page: PAGE_8070,
            leds: BTreeMap::new(),
            direct: 1,
        };
        assert_eq!(
            set_zone(&node, 1, Color(1, 2, 3)).to_vec(),
            hex("11 FF 0B 30 01 01 01 02 03 00 00 00 00 00 00 00 00")
        );
        node.page = PAGE_8071;
        node.rgb = 0x0A;
        assert_eq!(
            set_zone(&node, 0, Color(4, 5, 6)).to_vec(),
            hex("11 FF 0A 10 00 01 04 05 06 00 00 00 00 00 00 00 01")
        );
    }

    // logitech_device::initialiseDevice (getDeviceName, getFeatureIndex, getRGBconfig on 8070).
    #[test]
    fn wired_8070_discovery() {
        let log = Log::default();
        let hid = Fake::new(log.clone(), mouse_8070);
        let node = discover(None, &hid, 0xFF).unwrap();
        assert_eq!(node.name, "G502 HERO Gaming Mouse");
        assert_eq!(node.kind, 3);
        assert_eq!((node.rgb, node.page, node.direct), (0x0B, PAGE_8070, 1));
        assert_eq!(node.leds.len(), 2);
        assert_eq!(node.leds[&0], vec![0x0000, 0x0001]);
        assert_eq!(
            take(&log),
            vec![
                hex("11 FF 00 01 00 05"),
                hex("11 FF 04 01"),
                hex("11 FF 04 11 00"),
                hex("11 FF 04 11 10"),
                hex("11 FF 04 21"),
                hex("11 FF 00 01 80 70"),
                hex("11 FF 0B 00"),
                hex("11 FF 0B 10 00"),
                hex("11 FF 0B 20 00 00"),
                hex("11 FF 0B 20 00 01"),
                hex("11 FF 0B 10 01"),
                hex("11 FF 0B 20 01 00"),
                hex("11 FF 0B 20 01 01"),
            ]
        );
    }

    // getRGBconfig on 8071: count with FF FF, LED info with i FF, effects through GET_COUNT (0x00).
    #[test]
    fn wired_8071_discovery() {
        let log = Log::default();
        let hid = Fake::new(log.clone(), |req: &[u8]| {
            let r = match req[..4] {
                [0x11, 0xFF, 0x00, 0x01] => match (req[4], req[5]) {
                    (0x00, 0x05) => reply(&[0x11, 0xFF, 0x00, 0x01, 0x04]),
                    (0x80, 0x71) => reply(&[0x11, 0xFF, 0x00, 0x01, 0x0A]),
                    _ => reply(&[0x11, 0xFF, 0x00, 0x01, 0x00]),
                },
                [0x11, 0xFF, 0x04, 0x01] => reply(&[0x11, 0xFF, 0x04, 0x01, 4]),
                [0x11, 0xFF, 0x04, 0x11] => {
                    reply(&[0x11, 0xFF, 0x04, 0x11, b'G', b'7', b'3', b'3'])
                }
                [0x11, 0xFF, 0x04, 0x21] => reply(&[0x11, 0xFF, 0x04, 0x21, 0x08]),
                [0x11, 0xFF, 0x0A, 0x00] => match (req[4], req[5]) {
                    (0xFF, 0xFF) => reply(&[0x11, 0xFF, 0x0A, 0x00, 0x00, 0x00, 0x01]),
                    (i, 0xFF) => reply(&[0x11, 0xFF, 0x0A, 0x00, i, 0x00, 0x00, 0x01, 0x02]),
                    (_, j) => reply(&[
                        0x11,
                        0xFF,
                        0x0A,
                        0x00,
                        0,
                        0,
                        0x00,
                        if j == 1 { 0 } else { 1 },
                    ]),
                },
                _ => return Vec::new(),
            };
            vec![r]
        });
        let node = discover(None, &hid, 0xFF).unwrap();
        assert_eq!(
            (node.rgb, node.page, node.kind, node.direct),
            (0x0A, PAGE_8071, 8, 0)
        );
        let sent = take(&log);
        assert_eq!(
            sent[4..],
            [
                hex("11 FF 00 01 80 70"),
                hex("11 FF 00 01 80 71"),
                hex("11 FF 0A 00 FF FF"),
                hex("11 FF 0A 00 00 FF"),
                hex("11 FF 0A 00 00 00"),
                hex("11 FF 0A 00 00 01"),
            ]
        );

        let mut ls = Lightspeed::new(None, Box::new(hid), vec![("h".into(), node)]);
        ls.push("h", &[Color(1, 2, 3)]).unwrap();
        assert_eq!(
            take(&log),
            vec![
                hex("11 FF 0A 50 01 03 05"),
                hex("11 FF 0A 80 01 01"),
                hex("11 FF 0A 10 00 00 01 02 03 00 00 00 00 00 00 00 01"),
            ]
        );
        ls.push("h", &[Color(1, 2, 3)]).unwrap();
        assert!(take(&log).is_empty());
        ls.push("h", &[Color(0, 0, 0)]).unwrap();
        assert_eq!(take(&log).len(), 1);
    }

    // RGBController_LogitechLightspeed only offers Direct when LED 0 has an LED_ON effect.
    #[test]
    fn no_led_on_effect_means_no_direct_mode() {
        let hid = Fake::new(Log::default(), |req: &[u8]| {
            let mut r = mouse_8070(req);
            if req[..4] == [0x11, 0xFF, 0x0B, 0x20] {
                r[0][7] = 0x03;
            }
            r
        });
        assert_eq!(discover(None, &hid, 0xFF), None);
    }

    #[test]
    fn wired_push_takes_control_once() {
        let log = Log::default();
        let hid = Fake::new(log.clone(), mouse_8070);
        let node = discover(None, &hid, 0xFF).unwrap();
        take(&log);
        let mut ls = Lightspeed::new(None, Box::new(hid), vec![("m".into(), node)]);
        assert_eq!(ls.devices()[0].leds.len(), 2);
        ls.push("m", &[Color(9, 9, 9), Color(8, 8, 8)]).unwrap();
        assert_eq!(
            take(&log),
            vec![
                hex("11 FF 0B 80 01 01"),
                hex("11 FF 0B 30 00 01 09 09 09"),
                hex("11 FF 0B 30 01 01 08 08 08"),
            ]
        );
    }

    // getWirelessDevice: enable notifications, read the count, fake a reconnect and collect slots.
    #[test]
    fn paired_discovery() {
        let log = Log::default();
        let hid = Fake::new(log.clone(), |req: &[u8]| match req {
            [0x10, 0xFF, 0x81, 0x00, ..] => vec![vec![0x10, 0xFF, 0x81, 0x00, 0x00, 0x00, 0x00]],
            [0x10, 0xFF, 0x80, 0x00, ..] => vec![vec![0x10, 0xFF, 0x80, 0x00, 0x00, 0x00, 0x00]],
            [0x10, 0xFF, 0x81, 0x02, ..] => vec![vec![0x10, 0xFF, 0x81, 0x02, 0x00, 0x01, 0x00]],
            [0x10, 0xFF, 0x80, 0x02, ..] => vec![
                vec![0x10, 0x01, 0x41, 0x04, 0x00, 0x5F, 0x40],
                vec![0x10, 0xFF, 0x80, 0x02, 0x00, 0x00, 0x00],
            ],
            _ => Vec::new(),
        });
        assert_eq!(paired(&hid), BTreeMap::from([(0x405F, 0x01)]));
        assert_eq!(
            take(&log),
            vec![
                vec![0x10, 0xFF, 0x81, 0x00, 0x00, 0x00, 0x00],
                vec![0x10, 0xFF, 0x80, 0x00, 0x00, 0x01, 0x00],
                vec![0x10, 0xFF, 0x81, 0x02, 0x00, 0x00, 0x00],
                vec![0x10, 0xFF, 0x80, 0x02, 0x02, 0x00, 0x00],
            ]
        );
    }

    // logitech_device::connected for a wireless index: error 9 means paired but switched off.
    #[test]
    fn connected_reads_resource_error() {
        let log = Log::default();
        let off = Fake::new(log.clone(), |_: &[u8]| {
            vec![vec![0x10, 0x01, 0x8F, 0x81, 0x02, 0x09, 0x00]]
        });
        assert!(!connected(&off, 0x01));
        assert_eq!(
            take(&log),
            vec![vec![0x10, 0x01, 0x81, 0x02, 0x00, 0x00, 0x00]]
        );
        let on = Fake::new(log.clone(), |_: &[u8]| {
            vec![vec![0x10, 0x01, 0x81, 0x02, 0x00, 0x00, 0x00]]
        });
        assert!(connected(&on, 0x01));
    }

    #[test]
    fn validity_matches_is_valid() {
        assert!(valid("G502", 3, 1, 0x0B));
        assert!(!valid("G50", 3, 1, 0x0B));
        assert!(!valid("G502", 9, 1, 0x0B));
        assert!(!valid("G502", 3, 0, 0x0B));
        assert!(!valid("{502", 3, 1, 0x0B));
    }
}
