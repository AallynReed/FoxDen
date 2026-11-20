//! Ported from OpenRGB Controllers/LogitechController/LogitechHIDPP20Controller (GPL-2.0-or-later).

use std::sync::{Arc, Mutex};

use super::super::fake::{Ev, Fake};
use super::*;

/// What the simulated device does with one request.
enum R {
    Ok(Vec<u8>),
    Err(u8),
    Silent,
}

/// A long request as `SendStandard` frames it.
fn req(index: u8, feat: u8, function: u8, params: &[u8]) -> Vec<u8> {
    let mut b = vec![0x11, index, feat, function | 0x07];
    b.extend(params);
    b.resize(20, 0);
    b
}

fn root(index: u8, page: u16) -> Vec<u8> {
    req(index, 0, 0, &page.to_be_bytes())
}

/// A device answering IRoot from `features` and everything else through `handle(feat, fn, params)`.
/// Answers longer than 16 bytes come back as a very-long (0x12) report.
fn sim(
    features: &'static [(u16, u8)],
    mut handle: impl FnMut(u8, u8, &[u8]) -> R + Send + 'static,
) -> Fake {
    Fake::new(move |w| {
        let (index, feat, func) = (w[1], w[2], w[3]);
        let answer = if feat == 0 && func & 0xF0 == 0 {
            let page = u16::from_be_bytes([w[4], w[5]]);
            let idx = features.iter().find(|f| f.0 == page).map_or(0, |f| f.1);
            R::Ok(vec![idx, 0, 1])
        } else {
            handle(feat, func & 0xF0, &w[4..20])
        };
        let mut frame = match answer {
            R::Ok(params) => {
                let id = if params.len() > 16 { 0x12 } else { 0x11 };
                let mut f = vec![id, index, feat, func];
                f.extend(params);
                f
            }
            R::Err(code) => vec![0x11, index, 0xFF, feat, func, code],
            R::Silent => return Vec::new(),
        };
        frame.resize(if frame[0] == 0x12 { 64 } else { 20 }, 0);
        vec![frame]
    })
}

const MOUSE: &[(u16, u8)] = &[
    (0x0001, 1),
    (0x0003, 3),
    (0x0005, 2),
    (0x8100, 4),
    (0x8071, 5),
];

fn mouse_handler(feat: u8, func: u8, p: &[u8]) -> R {
    match (feat, func, p[0], p[1]) {
        (2, 0x00, ..) => R::Ok(vec![4]),
        (2, 0x10, ..) => R::Ok(b"G203".to_vec()),
        (2, 0x20, ..) => R::Ok(vec![3]),
        (3, 0x00, ..) => R::Ok(vec![
            1, 0x1A, 0x2B, 0x3C, 0x4D, 0, 0, 0x40, 0x92, 0xC0, 0x92,
        ]),
        (3, 0x10, ..) => R::Ok(vec![0, b'M', b'P', b'M', 1, 2]),
        (3, 0x20, ..) => R::Ok(Vec::new()),
        (5, 0x00, 0xFF, 0xFF) => R::Ok(vec![0xFF, 0xFF, 2]),
        (5, 0x00, 0, 0xFF) => R::Ok(vec![0, 0xFF, 0, 1, 2]),
        (5, 0x00, 1, 0xFF) => R::Ok(vec![1, 0xFF, 0, 2, 1]),
        (5, 0x00, 0, 0) => R::Ok(vec![0, 0, 0, 0]),
        (5, 0x00, 0, 1) | (5, 0x00, 1, 0) => R::Ok(vec![p[0], p[1], 0, 1]),
        (5, 0x00, 0xFF, 0x00) => R::Err(0x02),
        (4 | 5, ..) => R::Ok(p[..3].to_vec()),
        _ => R::Err(0x02),
    }
}

fn mouse() -> Fake {
    sim(MOUSE, mouse_handler)
}

// SendAcked: BUSY (0x08) resends after the reliable backoff; any other error fails at once.
#[test]
fn acked_retries_busy_only() {
    let mut busy = 1;
    let f = sim(&[], move |_, _, _| {
        if busy > 0 {
            busy -= 1;
            R::Err(0x08)
        } else {
            R::Ok(vec![7])
        }
    });
    let mut h = Hidpp::new(f, 0xFF, false, false, String::new());
    assert_eq!(h.acked(5, 0x10, &[1], RELIABLE).map(|r| r[0]), Ok(7));
    let sent = req(0xFF, 5, 0x10, &[1]);
    assert_eq!(
        h.link.log,
        [Ev::Write(sent.clone()), Ev::Sleep(63), Ev::Write(sent)]
    );
    let f = sim(&[], |_, _, _| R::Err(0x05));
    let mut h = Hidpp::new(f, 0x02, false, false, String::new());
    assert_eq!(h.acked(5, 0x10, &[], RELIABLE), Err(Fail::Error(0x05)));
    assert_eq!(h.link.writes().len(), 1);
}

// FirstContactPolicy: probe on a cable, first-contact behind a receiver, Bluetooth on Bluetooth.
#[test]
fn first_contact_policies() {
    let attempts = |wireless, bluetooth| {
        let mut h = Hidpp::new(
            Fake::new(|_| Vec::new()),
            0xFF,
            wireless,
            bluetooth,
            String::new(),
        );
        assert_eq!(h.probe_identity(), "");
        let sleeps: Vec<u64> = h
            .link
            .log
            .iter()
            .filter_map(|e| match e {
                Ev::Sleep(ms) => Some(*ms),
                _ => None,
            })
            .collect();
        (h.link.writes().len(), sleeps)
    };
    assert_eq!(attempts(false, false), (2, vec![100]));
    assert_eq!(attempts(true, false), (4, vec![100, 250, 500]));
    assert_eq!(attempts(false, true), (3, vec![200, 500]));
}

// ProbeIdentity: IRoot FeatureSet, then DiscoverFirmwareInfo's three reads.
#[test]
fn probe_identity_reads_unit_id() {
    let mut h = Hidpp::new(mouse(), 0xFF, false, false, String::new());
    assert_eq!(h.probe_identity(), "1A2B3C4D");
    assert_eq!(
        h.link.writes(),
        [
            root(0xFF, 0x0001),
            root(0xFF, 0x0003),
            req(0xFF, 3, 0x00, &[]),
            req(0xFF, 3, 0x10, &[0]),
            req(0xFF, 3, 0x20, &[]),
        ]
    );
    assert_eq!(h.caps.serial, "1A2B3C4D");
}

// Probe on a zone-effect (0x8071) mouse: every lookup and read, in order.
#[test]
fn probe_zone_mouse() {
    let mut h = Hidpp::new(mouse(), 0xFF, false, false, String::new());
    assert_eq!(h.probe(), Probed::Lit);
    let w = |f, func, p: &[u8]| req(0xFF, f, func, p);
    assert_eq!(
        h.link.writes(),
        [
            root(0xFF, 0x0001),
            root(0xFF, 0x0005),
            w(2, 0x00, &[]),
            w(2, 0x10, &[0]),
            w(2, 0x20, &[]),
            root(0xFF, 0x0003),
            w(3, 0x00, &[]),
            w(3, 0x10, &[0]),
            w(3, 0x20, &[]),
            root(0xFF, 0x8101),
            root(0xFF, 0x8100),
            root(0xFF, 0x4522),
            root(0xFF, 0x8071),
            w(5, 0x00, &[0xFF, 0xFF, 0x00]),
            w(5, 0x00, &[0, 0xFF]),
            w(5, 0x00, &[0, 0, 0, 0]),
            w(5, 0x00, &[0, 1, 0, 0]),
            w(5, 0x00, &[1, 0xFF]),
            w(5, 0x00, &[1, 0, 0, 0]),
            w(5, 0x00, &[0xFF, 0x00, 0x01, 0x00, 0x01]),
            root(0xFF, 0x8081),
            root(0xFF, 0x8080),
            root(0xFF, 0x4540),
            root(0xFF, 0x1D4B),
            root(0xFF, 0x0001),
            root(0xFF, 0x8070),
            root(0xFF, 0x0600),
            root(0xFF, 0x0620),
        ]
    );
    assert_eq!(h.caps.name, "G203");
    assert_eq!(h.caps.device_type, 3);
    assert_eq!(
        h.caps.clusters,
        [
            Cluster {
                index: 0,
                effects: vec![Effect { index: 0, id: 0 }, Effect { index: 1, id: 1 }]
            },
            Cluster {
                index: 1,
                effects: vec![Effect { index: 0, id: 1 }]
            },
        ]
    );
    assert!(!h.caps.has_effect_cards && !h.caps.has_perkey);
}

fn static_req(cluster: u8, effect: u8, c: [u8; 3]) -> Vec<u8> {
    let mut p = vec![cluster, effect, c[0], c[1], c[2], 0x02];
    p.resize(16, 0);
    req(0xFF, 5, 0x10, &p)
}

// ClaimSWControlIfNeeded (0x8071 pipelined claim), DeviceUpdateLEDs zone path, UpgradeSwControlAfterFirstPaint.
#[test]
fn zone_direct_mode() {
    let mut h = Hidpp::new(mouse(), 0xFF, false, false, String::new());
    h.probe();
    let mut d = Unified::new(h, "1A2B3C4D");
    assert_eq!(d.device.leds.len(), 2);
    d.ctl.link.log.clear();
    let id = d.device.id.clone();
    d.push(&id, &[Color(1, 2, 3), Color(4, 5, 6)]).unwrap();
    let mut config = vec![0x01, 0, 0, 0x00, 0x3C, 0x01, 0x2C];
    config.resize(16, 0);
    assert_eq!(
        d.ctl.link.writes(),
        [
            req(0xFF, 5, 0x50, &[0x01, 0x03, 0x06]),
            req(0xFF, 5, 0x80, &[0x01, 0x01]),
            req(0xFF, 5, 0x70, &config),
            req(0xFF, 4, 0x10, &[0x02]),
            static_req(0, 1, [1, 2, 3]),
            static_req(1, 0, [4, 5, 6]),
            req(0xFF, 5, 0x50, &[0x01, 0x03, 0x05]),
        ]
    );
    d.ctl.link.log.clear();
    d.push(&id, &[Color(1, 2, 3), Color(4, 5, 6)]).unwrap();
    assert!(d.ctl.link.log.is_empty());
    d.push(&id, &[Color(1, 2, 3), Color::BLACK]).unwrap();
    let mut off = vec![1, 0, 0, 0, 0];
    off.resize(16, 0);
    assert_eq!(d.ctl.link.writes(), [req(0xFF, 5, 0x10, &off)]);
}

// VerifyClaimPipeline: an unanswered claim write is re-sent blocking.
#[test]
fn claim_resends_missed_ack() {
    let mut first_host = true;
    let f = sim(MOUSE, move |feat, func, p| {
        if feat == 4 && func == 0x10 && std::mem::take(&mut first_host) {
            return R::Silent;
        }
        mouse_handler(feat, func, p)
    });
    let mut h = Hidpp::new(f, 0xFF, false, false, String::new());
    h.probe();
    h.link.log.clear();
    assert!(h.claim());
    let writes = h.link.writes();
    assert_eq!(writes.len(), 5);
    assert_eq!(writes[3], writes[4]);
    assert_eq!(writes[4], req(0xFF, 4, 0x10, &[0x02]));
}

const KEYBOARD: &[(u16, u8)] = &[
    (0x0001, 1),
    (0x0005, 2),
    (0x8101, 4),
    (0x8071, 5),
    (0x8081, 6),
];

/// Per-key keyboard behind a receiver at slot 1: zones 4..7, 9 and 20, with effect cards.
fn keyboard(busy_frame_ends: usize) -> Fake {
    keyboard_with(KEYBOARD, busy_frame_ends, true)
}

fn keyboard_with(features: &'static [(u16, u8)], busy_frame_ends: usize, cards: bool) -> Fake {
    let mut busy = busy_frame_ends;
    sim(features, move |feat, func, p| match (feat, func) {
        (2, 0x00) => R::Ok(vec![4]),
        (2, 0x10) => R::Ok(b"G515".to_vec()),
        (2, 0x20) => R::Ok(vec![0]),
        (5, 0x00) if p[..2] == [0xFF, 0xFF] => R::Ok(vec![0, 0, 1]),
        (5, 0x00) if p[..2] == [0, 0xFF] => R::Ok(vec![0, 0xFF, 0, 1, 4]),
        (5, 0x00) if p[0] == 0 => R::Ok(vec![0, p[1], 0, 1]),
        (5, 0x00) if p[..2] == [0xFF, 0x00] && !cards => R::Err(0x02),
        (5, 0x00) if p[..2] == [0xFF, 0x00] => {
            let mut r = vec![0u8; 16];
            r[10..12].copy_from_slice(&[0x20, 0x64]);
            R::Ok(r)
        }
        (6, 0x00) if p[1] == 0 => {
            let mut r = vec![0u8; 16];
            r[2] = 0b1111_0000;
            r[3] = 0b0000_0010;
            r[4] = 0b0001_0000;
            R::Ok(r)
        }
        (6, 0x00) => R::Ok(vec![0; 16]),
        (6, 0x10) => R::Ok(p.chunks(4).map(|c| c[0]).filter(|&z| z != 0).collect()),
        (6, 0x50 | 0x60) => R::Ok(p.to_vec()),
        (6, 0x70) if busy > 0 => {
            busy -= 1;
            R::Err(0x08)
        }
        (6, 0x70) | (3, _) => R::Ok(Vec::new()),
        (4 | 5, _) => R::Ok(p[..3].to_vec()),
        _ => R::Err(0x02),
    })
}

fn probed_keyboard(busy_frame_ends: usize) -> Unified<Fake> {
    probed(keyboard(busy_frame_ends))
}

fn probed(f: Fake) -> Unified<Fake> {
    let target = Target {
        identity: "AABBCCDD".into(),
        index: 1,
        behind_receiver: true,
        pairing_name: "G515 LS TKL".into(),
    };
    let mut d = build(f, None, &target, false).unwrap();
    d.ctl.link.log.clear();
    d
}

const RED: Color = Color(0xFF, 0, 0);
const GREEN: Color = Color(0, 0xFF, 0);
const BLUE: Color = Color(0, 0, 0xFF);
const WHITE: Color = Color(0xFF, 0xFF, 0xFF);

// DiscoverPerKeyZones: zone ids from the page 0 bitmap, zone 0 skipped.
#[test]
fn perkey_zones_from_bitmap() {
    let d = probed_keyboard(0);
    assert_eq!(d.zones, [4, 5, 6, 7, 9, 20]);
    assert_eq!(d.device.kind, DeviceKind::Keyboard);
    assert!(d.ctl.caps.has_effect_cards);
    assert_eq!(d.ctl.caps.template, [0x20, 0x64]);
}

// SendPerKeyFrame: claim, DoObservedPerKeyPrep, fn5 ranges / fn6 single value / fn1 individual, FrameEnd, flags 5.
#[test]
fn perkey_first_frame() {
    let mut d = probed_keyboard(0);
    let id = d.device.id.clone();
    d.push(&id, &[RED, RED, RED, GREEN, RED, BLUE]).unwrap();
    let w = |f, func, p: &[u8]| req(1, f, func, p);
    let mut config = vec![0x01, 0, 0, 0x00, 0x3C, 0x01, 0x2C];
    config.resize(16, 0);
    let mut prep1 = vec![0xFF, 0x02, 0, 0, 0, 0, 0, 0, 0x20, 0x64, 0, 0, 0x01];
    prep1.resize(16, 0);
    let mut prep2 = vec![0xFF, 0x04, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x01];
    prep2.resize(16, 0);
    assert_eq!(
        d.ctl.link.writes(),
        [
            w(5, 0x50, &[0x01, 0x03, 0x06]),
            w(5, 0x80, &[0x01, 0x01]),
            w(5, 0x70, &config),
            w(4, 0x60, &[0x05]),
            w(5, 0x10, &prep1),
            w(5, 0x10, &prep2),
            w(6, 0x50, &[4, 6, 0xFF, 0, 0]),
            w(6, 0x60, &[0xFF, 0, 0, 9]),
            w(6, 0x10, &[7, 0, 0xFF, 0, 20, 0, 0, 0xFF]),
            w(6, 0x70, &[0; 16]),
            w(5, 0x50, &[0x01, 0x03, 0x05]),
        ]
    );
}

// SendPerKeyFrame delta: the changed key plus the rolling resync's four least recently sent keys.
#[test]
fn perkey_delta_with_resync() {
    let mut d = probed_keyboard(0);
    let id = d.device.id.clone();
    d.push(&id, &[RED, RED, RED, GREEN, RED, BLUE]).unwrap();
    d.ctl.link.log.clear();
    d.push(&id, &[RED, RED, RED, GREEN, RED, BLUE]).unwrap();
    assert!(d.ctl.link.log.is_empty());
    d.push(&id, &[RED, RED, RED, WHITE, RED, BLUE]).unwrap();
    let w = |f, func, p: &[u8]| req(1, f, func, p);
    assert_eq!(
        d.ctl.link.writes(),
        [
            w(6, 0x50, &[4, 6, 0xFF, 0, 0]),
            w(6, 0x60, &[0xFF, 0, 0, 9]),
            w(6, 0x10, &[7, 0xFF, 0xFF, 0xFF]),
            w(6, 0x70, &[0; 16]),
        ]
    );
}

// PerKeyFrameEnd: BUSY on FrameEnd is polled every 10 ms until the device accepts it.
#[test]
fn frame_end_polls_through_busy() {
    let mut d = probed_keyboard(2);
    let id = d.device.id.clone();
    d.push(&id, &[RED; 6]).unwrap();
    let fe = req(1, 6, 0x70, &[0; 16]);
    let tail: Vec<Ev> = d
        .ctl
        .link
        .log
        .iter()
        .skip_while(|e| **e != Ev::Write(fe.clone()))
        .cloned()
        .collect();
    assert_eq!(
        tail,
        [
            Ev::Write(fe.clone()),
            Ev::Sleep(10),
            Ev::Write(fe.clone()),
            Ev::Sleep(10),
            Ev::Write(fe),
            Ev::Write(req(1, 5, 0x50, &[0x01, 0x03, 0x05])),
        ]
    );
}

// MatchPerKeyAck: fn1 ACKs echo zone ids, 0xFF for a zone the firmware rejected.
#[test]
fn individual_ack_echo() {
    let mut d = probed_keyboard(0);
    let packet = || Packet {
        function: FN_SET_INDIVIDUAL,
        payload: [7, 1, 2, 3, 20, 4, 5, 6, 0, 0, 0, 0, 0, 0, 0, 0],
        len: 8,
        zones: vec![7, 20],
        acked: false,
    };
    let mut rd = [0u8; 60];
    d.ctl.packets.push(packet());
    rd[..2].copy_from_slice(&[7, 0xFF]);
    assert!(d.ctl.match_ack(FN_SET_INDIVIDUAL, &rd));
    assert_eq!(d.ctl.acked_zones, [7]);
    d.ctl.packets = vec![packet()];
    rd[..2].copy_from_slice(&[9, 9]);
    assert!(!d.ctl.match_ack(FN_SET_INDIVIDUAL, &rd));
    assert!(!d.ctl.match_ack(FN_SET_RANGE, &rd));
}

/// A very-long request as `SendVeryLongFrame` frames it.
fn vreq(index: u8, feat: u8, function: u8, params: &[u8]) -> Vec<u8> {
    let mut b = vec![0x12, index, feat, function | 0x07];
    b.extend(params);
    b.resize(64, 0);
    b
}

const V1: &[(u16, u8)] = &[
    (0x0001, 1),
    (0x0003, 3),
    (0x0005, 2),
    (0x8070, 5),
    (0x8080, 7),
];

/// keyType, fn1 keyCount (`None` fails), keyIds.
type Types = &'static [(u16, Option<u16>, &'static [u8])];

const A_TO_Q: [u8; 17] = [
    0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10, 0x11, 0x12, 0x13,
    0x14,
];

/// A wired 0x8070 + 0x8080 keyboard with model id `pid`, fn0 typeFlags `flags` and `types`.
fn v1_board(pid: u16, flags: u16, types: Types, busy_flushes: usize) -> Fake {
    let mut busy = busy_flushes;
    sim(V1, move |feat, func, p| {
        let key_type = u16::from_be_bytes([p[0], p[1]]);
        let entry = types.iter().find(|t| t.0 == key_type);
        match (feat, func) {
            (2, 0x00) => R::Ok(vec![4]),
            (2, 0x10) => R::Ok(b"G512".to_vec()),
            (2, 0x20) => R::Ok(vec![0]),
            (3, 0x00) => {
                let [h, l] = pid.to_be_bytes();
                R::Ok(vec![1, 1, 2, 3, 4, 0, 0, 0, 0, h, l])
            }
            (3, _) => R::Ok(Vec::new()),
            (5, 0x00) => R::Ok(vec![1]),
            (5, 0x10) => R::Ok(vec![0, 0, 0, 1]),
            (5, 0x20) => R::Ok(vec![p[0], p[1], 0, 1]),
            (7, 0x00) => {
                let [h, l] = flags.to_be_bytes();
                R::Ok(vec![h, l, 0, 0, types.len() as u8, 0, 0])
            }
            (7, 0x10) => match entry {
                Some((_, Some(n), _)) => R::Ok(n.to_be_bytes().to_vec()),
                _ => R::Err(0x02),
            },
            (7, 0x20) => {
                let start = u16::from_be_bytes([p[2], p[3]]) as usize;
                let mut r = p[..4].to_vec();
                for &id in entry.map_or(&[][..], |t| t.2).iter().skip(start).take(14) {
                    r.extend([id, 0, 0, 0]);
                }
                R::Ok(r)
            }
            (7, 0x30) => R::Ok(Vec::new()),
            (7, 0x50) if busy > 0 => {
                busy -= 1;
                R::Err(0x08)
            }
            (7, 0x50) => R::Ok(Vec::new()),
            _ => R::Err(0x02),
        }
    })
}

const G512_TYPES: Types = &[
    (0x0001, Some(17), &A_TO_Q),
    (0x0002, Some(0), &[]),
    (0x0040, None, &[1, 2, 3, 4, 5]),
];

fn wired<L: Link>(link: L, vl: Option<L>) -> Unified<L> {
    let target = Target {
        identity: "x".into(),
        index: 0xFF,
        behind_receiver: false,
        pairing_name: String::new(),
    };
    build(link, vl, &target, false).unwrap()
}

// DiscoverPerKey8080: fn0, fn1 per typeFlags bit, fn2 pages of 14; keyCount 0 skips, an fn1 failure pages fn2 until empty.
#[test]
fn perkey_8080_discovery() {
    let mut h = Hidpp::new(
        v1_board(0xC342, 0x0043, G512_TYPES, 0),
        0xFF,
        false,
        false,
        String::new(),
    );
    assert_eq!(h.probe(), Probed::Lit);
    let writes = h.link.writes();
    let at = writes
        .iter()
        .position(|w| *w == root(0xFF, 0x8080))
        .unwrap();
    let w = |func, p: &[u8]| req(0xFF, 7, func, p);
    assert_eq!(
        writes[at..at + 10],
        [
            root(0xFF, 0x8080),
            w(0x00, &[]),
            w(0x10, &[0, 1]),
            w(0x20, &[0, 1, 0, 0, 0]),
            w(0x20, &[0, 1, 0, 14, 0]),
            w(0x10, &[0, 2]),
            w(0x10, &[0, 0x40]),
            w(0x20, &[0, 0x40, 0, 0, 0]),
            w(0x20, &[0, 0x40, 0, 14, 0]),
            root(0xFF, 0x4540),
        ]
    );
    assert_eq!(
        h.caps.perkey_8080_types,
        [
            KeyType8080 {
                key_type: 1,
                key_ids: A_TO_Q.to_vec()
            },
            KeyType8080 {
                key_type: 0x40,
                key_ids: vec![1, 2, 3, 4, 5]
            },
        ]
    );
    assert!(h.caps.has_perkey_8080 && !h.caps.has_numpad);
    assert_eq!(h.caps.pids, [0, 0xC342, 0]);
}

/// One collection of a shared fake device: `wide` takes only 0x12 reports, the other refuses them.
struct Node(Arc<Mutex<Fake>>, bool);

impl Link for Node {
    fn write(&mut self, data: &[u8]) -> Result<usize, String> {
        if (data[0] == 0x12) != self.1 {
            return Err("wrong collection".into());
        }
        self.0.lock().unwrap().write(data)
    }

    fn read(&mut self, buf: &mut [u8], ms: i32) -> Result<usize, String> {
        self.0.lock().unwrap().read(buf, ms)
    }

    fn send_feature(&mut self, data: &[u8]) -> Result<(), String> {
        self.0.lock().unwrap().send_feature(data)
    }

    fn get_feature(&mut self, buf: &mut [u8]) -> Result<usize, String> {
        self.0.lock().unwrap().get_feature(buf)
    }

    fn descriptor(&mut self, buf: &mut [u8]) -> Result<usize, String> {
        self.0.lock().unwrap().descriptor(buf)
    }

    fn sleep(&mut self, ms: u64) {
        self.0.lock().unwrap().sleep(ms)
    }
}

// SetPerKey8080 on the 0x0604 handle: no claim on 0x8070, keyTypes in LED order, 14 keys a packet, FlushLEDs on the main handle.
#[test]
fn perkey_8080_frame_on_very_long_handle() {
    let dev = Arc::new(Mutex::new(v1_board(0xC342, 0x0043, G512_TYPES, 0)));
    let mut d = wired(Node(dev.clone(), false), Some(Node(dev.clone(), true)));
    assert_eq!(d.zones.len(), 89);
    assert_eq!(d.device.grid, [19, 6]);
    assert_eq!(d.zones.iter().filter(|&&z| z >> 8 == 0x40).count(), 2);
    dev.lock().unwrap().log.clear();
    let id = d.device.id.clone();
    let colors = vec![RED; d.device.leds.len()];
    d.push(&id, &colors).unwrap();
    let keys = |ids: &[u8]| {
        let mut p = vec![0, 1, 0, ids.len() as u8];
        for &k in ids {
            p.extend([k, 0xFF, 0, 0]);
        }
        p
    };
    let order = [
        0x14, 0x08, 0x0C, 0x12, 0x13, 0x04, 0x07, 0x09, 0x0A, 0x0B, 0x0D, 0x0E, 0x0F, 0x06, 0x05,
        0x11, 0x10,
    ];
    assert_eq!(
        dev.lock().unwrap().writes(),
        [
            vreq(
                0xFF,
                7,
                0x30,
                &[0, 0x40, 0, 2, 3, 0xFF, 0, 0, 2, 0xFF, 0, 0]
            ),
            vreq(0xFF, 7, 0x30, &keys(&order[..14])),
            vreq(0xFF, 7, 0x30, &keys(&order[14..])),
            req(0xFF, 7, 0x50, &[]),
        ]
    );
    assert!(d.ctl.retry_at.is_none());
}

// PerKeyCommit8080 without a very-long handle: 0x12 rides the main handle, BUSY on FlushLEDs is polled every 10 ms.
#[test]
fn perkey_8080_fallback_and_busy_flush() {
    let mut d = wired(v1_board(0xC342, 0x0043, G512_TYPES, 1), None);
    d.ctl.link.log.clear();
    let id = d.device.id.clone();
    d.push(&id, &vec![RED; d.device.leds.len()]).unwrap();
    let flush = req(0xFF, 7, 0x50, &[]);
    let log = &d.ctl.link.log;
    assert_eq!(
        log.iter()
            .filter(|e| matches!(e, Ev::Write(w) if w[0] == 0x12))
            .count(),
        3
    );
    assert_eq!(
        log[log.len() - 3..],
        [Ev::Write(flush.clone()), Ev::Sleep(10), Ev::Write(flush)]
    );
    assert!(d.ctl.retry_at.is_none());
}

// HIDPP20_QUIRK_8080_KEYTYPE40_ID12_ONLY (G410, 0xC330): keyType 0x40 sends keyIds 1 and 2 only; no strip, so all three stay LEDs.
#[test]
fn g410_keytype40_filter() {
    const TYPES: Types = &[(0x0001, Some(1), &[0x04]), (0x0040, Some(3), &[1, 2, 3])];
    let mut d = wired(v1_board(0xC330, 0x0041, TYPES, 0), None);
    assert_eq!(d.zones[d.zones.len() - 3..], [0x4001, 0x4002, 0x4003]);
    assert_eq!(d.zones.len(), 87 + 3);
    d.ctl.link.log.clear();
    let id = d.device.id.clone();
    d.push(&id, &vec![BLUE; d.device.leds.len()]).unwrap();
    assert_eq!(
        d.ctl.link.writes(),
        [
            vreq(0xFF, 7, 0x30, &[0, 1, 0, 1, 0x04, 0, 0, 0xFF]),
            vreq(
                0xFF,
                7,
                0x30,
                &[0, 0x40, 0, 2, 1, 0, 0, 0xFF, 2, 0, 0, 0xFF]
            ),
            req(0xFF, 7, 0x50, &[]),
        ]
    );
}

// G910 (0xC32B): the over-reported media keyType is still walked, its keys are phantoms; a numpad id makes the board full size.
#[test]
fn g910_media_walked_then_suppressed() {
    const TYPES: Types = &[
        (0x0001, Some(2), &[0x04, 0x53]),
        (0x0002, Some(2), &[0xB5, 0xB6]),
    ];
    let d = wired(v1_board(0xC32B, 0x0003, TYPES, 0), None);
    assert_eq!(d.ctl.caps.perkey_8080_types.len(), 2);
    assert!(d.ctl.caps.has_numpad);
    assert!(d.zones.iter().all(|&z| z >> 8 != 0x02));
    assert!(d.zones.contains(&0x0153) && d.zones.contains(&0x1001));
    assert_eq!(d.zones.len(), 104 + 11);
    assert_eq!(d.device.grid, [22, 8]);
}

// PerKey8080Capable on a known board with nothing enumerated: every main key on keyType 1, the off-frame backslashes as Extras.
#[test]
fn known_board_without_enumeration() {
    let d = wired(v1_board(0xC342, 0x0000, &[], 0), None);
    assert!(!d.ctl.caps.has_perkey_8080);
    assert_eq!(d.zones.len(), 104 + 2 + 2);
    assert!(d.zones.iter().all(|&z| z != 0));
    assert_eq!(d.zones[106..], [0x0132, 0x0164]);
    assert_eq!(d.device.grid, [21, 7]);
}

// Probe still gives up on the 0x0620 headset path.
#[test]
fn headset_hostmode_is_refused() {
    const HEADSET: &[(u16, u8)] = &[(0x0001, 1), (0x0620, 6)];
    let mut h = Hidpp::new(
        sim(HEADSET, |_, _, _| R::Err(0x02)),
        0xFF,
        false,
        false,
        String::new(),
    );
    assert_eq!(h.probe(), Probed::Unported(0x0620));
}

// 0x8070 zone device: DiscoverRGBEffects' 0x8070 layout, the simple [1, 1] claim, Static on fn 3, no upgrade.
#[test]
fn zone_8070_device() {
    const LED: &[(u16, u8)] = &[(0x0001, 1), (0x8100, 4), (0x8070, 5)];
    let f = sim(LED, |feat, func, p| match (feat, func) {
        (5, 0x00) => R::Ok(vec![2]),
        (5, 0x10) => R::Ok(vec![p[0], 0, 0, 1]),
        (5, 0x20) => R::Ok(vec![p[0], p[1], 0, 1]),
        (5, 0x30 | 0x80) => R::Ok(p[..2].to_vec()),
        _ => R::Err(0x02),
    });
    let mut h = Hidpp::new(f, 0xFF, false, false, String::new());
    assert_eq!(h.probe(), Probed::Lit);
    let writes = h.link.writes();
    let at = writes
        .iter()
        .position(|w| *w == root(0xFF, 0x8070))
        .unwrap();
    assert_eq!(
        writes[at + 1..at + 6],
        [
            req(0xFF, 5, 0x00, &[]),
            req(0xFF, 5, 0x10, &[0, 0]),
            req(0xFF, 5, 0x20, &[0, 0, 0, 0]),
            req(0xFF, 5, 0x10, &[1, 0]),
            req(0xFF, 5, 0x20, &[1, 0, 0, 0]),
        ]
    );
    let mut d = Unified::new(h, "x");
    d.ctl.link.log.clear();
    let id = d.device.id.clone();
    d.push(&id, &[Color(9, 8, 7), Color(1, 1, 1)]).unwrap();
    let mut a = vec![0, 0, 9, 8, 7, 0x02];
    a.resize(16, 0);
    let mut b = vec![1, 0, 1, 1, 1, 0x02];
    b.resize(16, 0);
    assert_eq!(
        d.ctl.link.writes(),
        [
            req(0xFF, 5, 0x80, &[1, 1]),
            req(0xFF, 5, 0x30, &a),
            req(0xFF, 5, 0x30, &b),
        ]
    );
}

// DoDisableKeysByUsageHandshake in the claim, then DoKeyboardFamilyPerKeyPrep before the first frame.
#[test]
fn keyboard_family_prep() {
    const G915: &[(u16, u8)] = &[
        (0x0001, 1),
        (0x0005, 2),
        (0x4522, 3),
        (0x8101, 4),
        (0x8071, 5),
        (0x8081, 6),
    ];
    let mut d = probed(keyboard_with(G915, 0, true));
    let id = d.device.id.clone();
    d.push(&id, &[RED; 6]).unwrap();
    let w = |f, func, p: &[u8]| req(1, f, func, p);
    let mut config = vec![0x01, 0, 0, 0x00, 0x3C, 0x01, 0x2C];
    config.resize(16, 0);
    let mut off = vec![0u8; 16];
    off[12] = 0x01;
    assert_eq!(
        d.ctl.link.writes(),
        [
            w(5, 0x50, &[0x01, 0x03, 0x06]),
            w(3, 0x30, &[]),
            w(3, 0x10, &[]),
            w(5, 0x80, &[0x01, 0x01]),
            w(5, 0x70, &config),
            w(4, 0x60, &[0x05]),
            w(5, 0x10, &off),
            w(6, 0x10, &[4, 0, 0, 0]),
            w(6, 0x70, &[0; 16]),
            w(6, 0x50, &[4, 7, 0xFF, 0, 0]),
            w(6, 0x60, &[0xFF, 0, 0, 9, 20]),
            w(6, 0x70, &[0; 16]),
            w(5, 0x50, &[0x01, 0x03, 0x05]),
        ]
    );
}

// SendPerKeyFrame without effect cards or 0x4522: per-cluster Static black, persist set, no 0x02 marker.
#[test]
fn static_black_primer() {
    let mut d = probed(keyboard_with(KEYBOARD, 0, false));
    let id = d.device.id.clone();
    d.push(&id, &[RED; 6]).unwrap();
    let mut primer = vec![0u8; 16];
    primer[12] = 0x01;
    let writes = d.ctl.link.writes();
    assert_eq!(writes[4], req(1, 5, 0x10, &primer));
    assert_eq!(writes[5], req(1, 6, 0x50, &[4, 7, 0xFF, 0, 0]));
}

// HIDPP20_QUIRK_KEEP_ONBOARD_MODE (G915 TKL, model 0x408E / 0xC343): the claim skips SetHostMode.
#[test]
fn keep_onboard_quirk_skips_host_mode() {
    let f = sim(MOUSE, |feat, func, p| match (feat, func) {
        (3, 0x00) => R::Ok(vec![1, 1, 2, 3, 4, 0, 0, 0x40, 0x8E, 0xC3, 0x43]),
        _ => mouse_handler(feat, func, p),
    });
    let mut h = Hidpp::new(f, 0xFF, false, false, String::new());
    h.probe();
    assert!(h.caps.keep_onboard);
    h.link.log.clear();
    assert!(h.claim());
    let writes = h.link.writes();
    assert_eq!(writes.len(), 3);
    assert!(writes.iter().all(|w| w[2] != 4));
}

// HIDPP20Enumerate on a receiver: legacy-owned slots skipped, a slot with no serial named by its node.
#[test]
fn receiver_slots_become_targets() {
    let mut f = Fake::new(|w| {
        let r = |b: &[u8]| {
            let mut v = b.to_vec();
            v.resize(20, 0);
            v
        };
        match (w[2], w[3], w[4]) {
            (0x81, 0x00, _) => vec![r(&[0x10, 0xFF, 0x81, 0x00, 0, 1, 0])],
            (0x81, 0x02, _) => vec![r(&[0x10, 0xFF, 0x81, 0x02, 0, 3, 0])],
            (0x80, 0x02, _) => vec![
                r(&[0x10, 0xFF, 0x80, 0x02]),
                r(&[0x10, 0x01, 0x41, 0x04, 0x00, 0x7F, 0x40]),
                r(&[0x10, 0x02, 0x41, 0x04, 0x00, 0xB4, 0x40]),
                r(&[0x10, 0x03, 0x41, 0x04, 0x00, 0xC0, 0x40]),
            ],
            (0x83, 0xB5, 0x31) => vec![r(&[0x11, 0xFF, 0x83, 0xB5, 0x31, 1, 2, 3, 4])],
            (0x83, 0xB5, 0x41) => vec![r(&[
                0x11, 0xFF, 0x83, 0xB5, 0x41, 4, b'G', b'5', b'1', b'5',
            ])],
            (0x83, 0xB5, sub) => vec![r(&[0x11, 0xFF, 0x83, 0xB5, sub, 0, 0, 0, 0])],
            _ => Vec::new(),
        }
    });
    let targets = receiver_targets(&mut f, "node");
    assert_eq!(
        targets,
        [
            Target {
                identity: "01020304".into(),
                index: 2,
                behind_receiver: true,
                pairing_name: "G515".into()
            },
            Target {
                identity: "node#3".into(),
                index: 3,
                behind_receiver: true,
                pairing_name: String::new()
            },
        ]
    );
}

// The REGISTER_HID_DETECTOR_B*/IP lines with HID_PID_ANY, Centurion left out.
#[test]
fn unified_matcher_lines() {
    let info = |page, usage, interface, bus| Info {
        vid: VID,
        pid: 0xC09D,
        interface,
        usage_page: page,
        usage,
        path: String::new(),
        serial: String::new(),
        product: String::new(),
        bus,
        release: 0,
    };
    let hits = |i: &Info| MATCHERS.iter().filter(|m| m.matches(i)).count();
    assert_eq!(hits(&info(0xFF00, 2, 1, Bus::Usb)), 2);
    assert_eq!(hits(&info(0xFF00, 1, 2, Bus::Usb)), 0);
    assert_eq!(hits(&info(0xFF43, 0x0602, 1, Bus::Usb)), 1);
    assert_eq!(hits(&info(0xFF43, 0x0602, -1, Bus::Bluetooth)), 1);
    assert_eq!(hits(&info(0xFF00, 2, -1, Bus::Bluetooth)), 1);
    assert_eq!(hits(&info(0xFFA0, 1, 3, Bus::Usb)), 0);
    assert_eq!(super::super::matchers().len(), 6);
}
