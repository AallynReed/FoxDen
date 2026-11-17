//! SONIX 0C45:8006 keyboards that take lighting as feature reports on the keyboard interface
//! itself, such as the Newskill Suiko and Sandberg Firestorm. Live colours when the firmware
//! accepts them; otherwise settled colours saved to the keyboard, written sparingly.

use std::time::{Duration, Instant};

use super::common::{open_with, positioned};
use crate::hid::{Io, Proto};
use foxrgb_core::{Backend, Color, DeviceKind, key_spot};
use foxrgb_hid::registry::{Detector, Info};
use hidapi::HidApi;

const FAMILY: &str = "sonix-cfg";
const VID: u16 = 0x0C45;
const PID: u16 = 0x8006;
const LEN: usize = 65;
const BLOCK: usize = 64;
const SLOTS: usize = 144;
/// Not a key code: the number pad Enter shares its code with Enter.
const PAD_ENTER: u32 = 0x0E;

/// Colours must hold this long before they are saved to the keyboard.
const SETTLE: Duration = Duration::from_millis(1500);
/// Saved writes go to the keyboard's memory, so never more often than this.
const GAP: Duration = Duration::from_secs(5);

/// Light index of each key, by Windows key code.
static KEYS: &[(u8, u32)] = &[
    (1, 0x1B),
    (2, 0x70),
    (3, 0x71),
    (4, 0x72),
    (5, 0x73),
    (6, 0x74),
    (7, 0x75),
    (8, 0x76),
    (9, 0x77),
    (10, 0x78),
    (11, 0x79),
    (12, 0x7A),
    (13, 0x7B),
    (19, 0xC0),
    (20, 0x31),
    (21, 0x32),
    (22, 0x33),
    (23, 0x34),
    (24, 0x35),
    (25, 0x36),
    (26, 0x37),
    (27, 0x38),
    (28, 0x39),
    (29, 0x30),
    (30, 0xBD),
    (31, 0xBB),
    (32, 0x90),
    (33, 0x6F),
    (34, 0x6A),
    (37, 0x09),
    (38, 0x51),
    (39, 0x57),
    (40, 0x45),
    (41, 0x52),
    (42, 0x54),
    (43, 0x59),
    (44, 0x55),
    (45, 0x49),
    (46, 0x4F),
    (47, 0x50),
    (48, 0xDB),
    (49, 0xDD),
    (50, 0x67),
    (51, 0x68),
    (52, 0x69),
    (55, 0x14),
    (56, 0x41),
    (57, 0x53),
    (58, 0x44),
    (59, 0x46),
    (60, 0x47),
    (61, 0x48),
    (62, 0x4A),
    (63, 0x4B),
    (64, 0x4C),
    (65, 0xBA),
    (66, 0xDE),
    (67, 0xDC),
    (68, 0x64),
    (69, 0x65),
    (70, 0x66),
    (73, 0xA0),
    (74, 0x5A),
    (75, 0x58),
    (76, 0x43),
    (77, 0x56),
    (78, 0x42),
    (79, 0x4E),
    (80, 0x4D),
    (81, 0xBC),
    (82, 0xBE),
    (83, 0xBF),
    (84, 0xA1),
    (85, 0x0D),
    (86, 0x61),
    (87, 0x62),
    (88, 0x63),
    (91, 0xA2),
    (92, 0x5B),
    (93, 0xA4),
    (94, 0x20),
    (95, 0xA5),
    (96, 0x5C),
    (97, 0x5D),
    (98, 0xA3),
    (99, 0x25),
    (100, 0x28),
    (101, 0x26),
    (102, 0x27),
    (103, 0x08),
    (104, 0x60),
    (105, 0x6E),
    (106, PAD_ENTER),
    (112, 0x2C),
    (113, 0x91),
    (115, 0x13),
    (116, 0x2D),
    (117, 0x24),
    (118, 0x21),
    (119, 0x2E),
    (120, 0x23),
    (121, 0x22),
    (122, 0x6D),
    (123, 0x6B),
];

/// The built-in effects as the vendor software leaves them, then the per-key mode record.
const MODES: [[u8; 16]; 20] = {
    const fn rec(mode: u8, rgb: [u8; 3], multi: u8, speed: u8, dir: u8) -> [u8; 16] {
        [
            mode, rgb[0], rgb[1], rgb[2], 0, 0, 0, 0, multi, 0x10, speed, dir, 0, 0, 0xAA, 0x55,
        ]
    }
    const W: [u8; 3] = [0xFF, 0xFF, 0xFF];
    const B: [u8; 3] = [0x00, 0x00, 0xFF];
    [
        rec(0x01, W, 0, 0x0C, 0),
        rec(0x02, W, 0, 0x0C, 0),
        rec(0x03, W, 0, 0x0C, 0),
        rec(0x04, W, 0, 0x0C, 0),
        rec(0x05, W, 0, 0x0C, 0),
        rec(0x06, B, 1, 0x0C, 0),
        rec(0x07, W, 0, 0x0C, 0),
        rec(0x08, B, 1, 0x0C, 0),
        rec(0x09, W, 0, 0x0C, 0),
        rec(0x0A, W, 0, 0x0C, 2),
        rec(0x0B, W, 0, 0x0C, 1),
        rec(0x0C, W, 0, 0x0C, 0),
        rec(0x0D, W, 0, 0x01, 0),
        rec(0x0E, W, 0, 0x0C, 0),
        rec(0x0F, W, 0, 0x0C, 0),
        rec(0x10, W, 0, 0x0C, 1),
        rec(0x11, W, 0, 0x0C, 0),
        rec(0x12, W, 0, 0x0C, 1),
        rec(0x13, W, 0, 0x0C, 0),
        rec(0x80, [0, 0, 0], 0, 0x00, 0),
    ]
};

/// Sent after the 0xAB command, as the vendor software does.
const KEY_BLOCK: [u8; BLOCK] = [
    0x4B, 0x56, 0xFC, 0x68, 0xFC, 0xE9, 0x95, 0xE6, 0xE8, 0x38, 0xB5, 0xA3, 0x94, 0xC2, 0xAC, 0x43,
    0x6B, 0x58, 0xD7, 0x39, 0x61, 0x2E, 0x69, 0x80, 0xD7, 0x33, 0x6B, 0x8B, 0xBE, 0x9E, 0x02, 0x7E,
    0x00, 0xC9, 0xF1, 0x9E, 0x1B, 0x43, 0x5E, 0x06, 0x0F, 0x21, 0xD9, 0x9D, 0x7B, 0x94, 0x9A, 0x44,
    0x03, 0xC1, 0x0A, 0x54, 0xF4, 0xCC, 0x86, 0x47, 0x2B, 0x36, 0x5F, 0xB5, 0x33, 0x6A, 0x25, 0x83,
];

fn command(cmd: u8, blocks: u8) -> [u8; BLOCK] {
    let mut p = [0u8; BLOCK];
    p[0] = 0x04;
    p[1] = cmd;
    p[8] = blocks;
    p
}

fn send(io: &mut dyn Io, payload: &[u8]) -> Result<(), String> {
    let mut b = Vec::with_capacity(LEN);
    b.push(0x00);
    b.extend_from_slice(payload);
    io.feature(&b, LEN)
}

/// Waits out a busy keyboard, then reports whether it took `cmd`.
fn ack(io: &mut dyn Io, cmd: u8) -> Result<bool, String> {
    for _ in 0..20 {
        let r = io.get_feature(0x00, LEN)?;
        let p = r.get(1..).unwrap_or_default();
        if p.get(3) == Some(&0xFF) {
            io.sleep(5);
            continue;
        }
        return Ok(p.get(1) == Some(&cmd) && p.get(3) == Some(&0x01));
    }
    Ok(false)
}

fn order(cmd: u8, blocks: u8, io: &mut dyn Io) -> Result<(), String> {
    send(io, &command(cmd, blocks))?;
    if ack(io, cmd)? {
        Ok(())
    } else {
        Err(format!("keyboard refused command {cmd:#04x}"))
    }
}

/// Colours by light index.
fn slots(colors: &[Color]) -> [Color; SLOTS] {
    let mut s = [Color::BLACK; SLOTS];
    for (&(light, _), &c) in KEYS.iter().zip(colors) {
        s[light as usize] = c;
    }
    s
}

/// The whole saved configuration: effects, a gap, the per-key table, and per-key mode active.
fn config(colors: &[Color]) -> Vec<u8> {
    let mut d: Vec<u8> = MODES.iter().flatten().copied().collect();
    d.resize(d.len() + 3 * BLOCK, 0);
    for c in slots(colors) {
        d.extend_from_slice(&[0x80, c.0, c.1, c.2]);
    }
    let mut last = [0u8; BLOCK];
    last[..16].copy_from_slice(&MODES[19]);
    d.extend_from_slice(&last);
    d
}

#[derive(Default)]
struct SonixCfg {
    /// Whether the keyboard took live colours; unknown until the first frame.
    live: Option<bool>,
    pending: Option<(Vec<Color>, Instant)>,
    saved: Option<Vec<Color>>,
    saved_at: Option<Instant>,
}

impl SonixCfg {
    fn live_frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let mut d = vec![0u8; 8 * BLOCK];
        for (i, c) in slots(colors).iter().enumerate().take(128) {
            d[i * 4..i * 4 + 4].copy_from_slice(&[i as u8, c.0, c.1, c.2]);
        }
        for block in d.chunks(BLOCK) {
            send(io, block)?;
        }
        order(0x02, 0, io)
    }

    fn save(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        order(0x18, 0, io)?;
        order(0xAB, 0, io)?;
        send(io, &KEY_BLOCK)?;
        order(0x02, 0, io)?;
        let data = config(colors);
        order(0x13, (data.len() / BLOCK) as u8, io)?;
        for block in data.chunks(BLOCK) {
            send(io, block)?;
        }
        order(0x02, 0, io)?;
        send(io, &command(0xF0, 0))
    }
}

impl Proto for SonixCfg {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        if self.live.is_none() {
            send(io, &command(0x20, 8))?;
            let took = ack(io, 0x20)?;
            self.live = Some(took);
            if took {
                return self.live_frame(io, colors);
            }
        } else if self.live == Some(true) {
            send(io, &command(0x20, 8))?;
            if !ack(io, 0x20)? {
                return Err("keyboard refused live colours".into());
            }
            return self.live_frame(io, colors);
        }
        let now = Instant::now();
        match &self.pending {
            Some((p, _)) if p == colors => {}
            _ => self.pending = Some((colors.to_vec(), now)),
        }
        let Some((p, since)) = &self.pending else {
            return Ok(());
        };
        let settled = now.duration_since(*since) >= SETTLE;
        let due = self.saved_at.is_none_or(|at| now.duration_since(at) >= GAP);
        if settled && due && self.saved.as_deref() != Some(p.as_slice()) {
            let p = p.clone();
            self.saved_at = Some(now);
            self.save(io, &p)?;
            self.saved = Some(p);
        }
        Ok(())
    }

    /// Saved colours only go out once they hold, which needs a look at unchanged frames.
    fn keepalive_ms(&self) -> Option<u64> {
        (self.live != Some(true)).then_some(500)
    }
}

fn open(api: &HidApi, info: &Info, _: &Detector) -> Result<Box<dyn Backend>, String> {
    let mut dev = positioned(
        &info.device_id(FAMILY),
        "SONIX keyboard",
        DeviceKind::Keyboard,
        23,
        6,
        &[],
    );
    dev.leds = KEYS.iter().map(|&(_, vk)| spot(vk)).collect();
    open_with(api, info, FAMILY, dev, SonixCfg::default())
}

fn spot(vk: u32) -> [f32; 2] {
    if vk == PAD_ENTER {
        return [22.25 / 22.75, 5.0 / 6.0];
    }
    let (x, y) = key_spot(vk, true).unwrap_or((0.0, 0.0));
    [x, y]
}

pub static DETECTORS: &[Detector] = &[Detector {
    name: "SONIX keyboard",
    vid: VID,
    pid: PID,
    interface: Some(0),
    usage_page: Some(0x0001),
    usage: Some(0x0006),
    open,
}];

#[cfg(test)]
mod tests {
    use super::super::common::mock::{Op, Rec};
    use super::*;

    fn ok(cmd: u8) -> Vec<u8> {
        let mut r = vec![0u8; LEN];
        r[1..5].copy_from_slice(&[0x04, cmd, 0x00, 0x01]);
        r
    }

    fn sent(io: &Rec) -> Vec<Vec<u8>> {
        io.ops
            .iter()
            .filter_map(|op| match op {
                Op::Feature(b) => Some(b[1..].to_vec()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn every_key_has_a_place_and_a_slot_of_its_own() {
        let mut lights: Vec<u8> = KEYS.iter().map(|k| k.0).collect();
        lights.sort();
        lights.dedup();
        assert_eq!(lights.len(), KEYS.len());
        assert_eq!(KEYS.len(), 104);
        assert!(lights.iter().all(|&l| (l as usize) < 128));
        assert!(
            KEYS.iter()
                .all(|&(_, vk)| vk == PAD_ENTER || key_spot(vk, true).is_some())
        );
    }

    #[test]
    fn the_saved_config_matches_the_vendor_layout() {
        let mut c = vec![Color::BLACK; KEYS.len()];
        c[0] = Color(1, 2, 3);
        let d = config(&c);
        assert_eq!(d.len(), 18 * BLOCK);
        assert_eq!(
            &d[..16],
            &[
                1, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0, 0, 0x10, 0x0C, 0, 0, 0, 0xAA, 0x55
            ]
        );
        assert_eq!(&d[19 * 16..20 * 16], &MODES[19]);
        assert!(d[320..512].iter().all(|&b| b == 0));
        assert_eq!(&d[512..520], &[0x80, 0, 0, 0, 0x80, 1, 2, 3]);
        assert_eq!(&d[17 * BLOCK..17 * BLOCK + 2], &[0x80, 0]);
    }

    #[test]
    fn live_colours_when_the_keyboard_takes_them() {
        let mut io = Rec::default();
        io.replies.extend([ok(0x20), ok(0x02)]);
        let mut p = SonixCfg::default();
        let mut c = vec![Color::BLACK; KEYS.len()];
        c[1] = Color(9, 8, 7);
        p.frame(&mut io, &c).unwrap();
        assert_eq!(p.live, Some(true));
        let s = sent(&io);
        assert_eq!(s.len(), 10);
        assert_eq!(&s[0][..9], &[0x04, 0x20, 0, 0, 0, 0, 0, 0, 8]);
        assert_eq!(&s[1][8..12], &[2, 9, 8, 7]);
        assert_eq!(&s[9][..2], &[0x04, 0x02]);
        assert_eq!(p.keepalive_ms(), None);
    }

    #[test]
    fn settled_colours_are_saved_when_live_is_refused() {
        let mut io = Rec::default();
        let mut p = SonixCfg::default();
        let c = vec![Color(5, 5, 5); KEYS.len()];
        p.frame(&mut io, &c).unwrap();
        assert_eq!(p.live, Some(false));
        assert_eq!(sent(&io).len(), 1);
        p.pending = Some((c.clone(), Instant::now() - SETTLE));
        io.replies.extend([0x18, 0xAB, 0x02, 0x13, 0x02].map(ok));
        p.frame(&mut io, &c).unwrap();
        let s = sent(&io);
        assert_eq!(s.len(), 1 + 5 + 1 + 18 + 1);
        assert_eq!(&s[3], &KEY_BLOCK.to_vec());
        assert_eq!(&s[5][..9], &[0x04, 0x13, 0, 0, 0, 0, 0, 0, 0x12]);
        assert_eq!(&s.last().unwrap()[..2], &[0x04, 0xF0]);
        let before = sent(&io).len();
        p.frame(&mut io, &c).unwrap();
        assert_eq!(sent(&io).len(), before);
    }

    #[test]
    fn changing_colours_wait_to_settle() {
        let mut io = Rec::default();
        let mut p = SonixCfg {
            live: Some(false),
            ..SonixCfg::default()
        };
        for i in 0..5u8 {
            p.frame(&mut io, &vec![Color(i, 0, 0); KEYS.len()]).unwrap();
        }
        assert!(io.ops.is_empty());
    }
}
