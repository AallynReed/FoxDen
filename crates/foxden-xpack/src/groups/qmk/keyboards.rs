//! QMK keyboards on SignalRGB firmware (protocol learned from SignalRGB's SRGBmods QMK plugins, e.g. Keychron_Q1v1_QMK_ANSI_Keyboard.js).

use super::boards::{BOARDS, LAYOUTS};
use crate::hid::{Driver, Io, Proto, drain, open_io, open_sibling, read_until, rgb};
use foxden_core::{Backend, Color, Device, DeviceKind};
use foxden_hid::registry::{Detector, Info};
use hidapi::HidApi;

const FAMILY: &str = "qmk";
const REPORT: usize = 33;
const RAW_PAGE: u16 = 0xFF60;
const GAP_MS: u64 = 30;
const REPLY_MS: u64 = 120;

const QMK_VERSION: u8 = 0x21;
const PROTOCOL_VERSION: u8 = 0x22;
const UNIQUE_ID: u8 = 0x23;
const STREAM: u8 = 0x24;
const EFFECT_ON: u8 = 0x25;
const EFFECT_OFF: u8 = 0x26;
const TOTAL_LEDS: u8 = 0x27;
const FIRMWARE_TYPE: u8 = 0x28;
const FIRMWARE_MAINLINE: u8 = 1;
const FIRMWARE_VIA: u8 = 2;

/// LED order and cells of one key layout: LED `i` sits at `pos[i]` and fills colour slot `keys[i]`.
pub struct Layout {
    pub w: u16,
    pub h: u16,
    pub keys: &'static [u8],
    pub pos: &'static [(u8, u8)],
}

/// One keyboard id: its default layout and the layouts picked by the unique-id reply's third byte.
pub struct Board {
    pub vid: u16,
    pub pid: u16,
    pub chunk: u8,
    pub layout: Option<u16>,
    pub by_id: &'static [(u8, u16)],
}

/// The bytes after the command echo; hidapi drops report id 0, SignalRGB keeps it.
fn reply(r: &[u8], cmd: u8) -> Option<&[u8]> {
    match r {
        [c, rest @ ..] if *c == cmd => Some(rest),
        [0, c, rest @ ..] if *c == cmd => Some(rest),
        _ => None,
    }
}

fn ask(io: &mut dyn Io, cmd: u8) -> Option<Vec<u8>> {
    io.write(&[0x00, cmd], REPORT).ok()?;
    io.sleep(GAP_MS);
    let r = read_until(io, REPORT, REPLY_MS, |r| reply(r, cmd).is_some()).ok()?;
    drain(io, REPORT);
    reply(&r, cmd).map(<[u8]>::to_vec)
}

/// Initialize's queries; refuses firmware without the SignalRGB commands. Returns the layout id byte.
fn handshake(io: &mut dyn Io) -> Result<u8, String> {
    drain(io, REPORT);
    let fw = ask(io, FIRMWARE_TYPE).ok_or("no SignalRGB QMK firmware reply")?;
    if !matches!(fw.first(), Some(&FIRMWARE_MAINLINE | &FIRMWARE_VIA)) {
        return Err("unsupported QMK firmware type".into());
    }
    let _ = ask(io, QMK_VERSION);
    ask(io, PROTOCOL_VERSION).ok_or("no SignalRGB protocol version reply")?;
    let id = ask(io, UNIQUE_ID)
        .and_then(|p| p.get(2).copied())
        .unwrap_or(0);
    let _ = ask(io, TOTAL_LEDS);
    Ok(id)
}

fn pick(board: &Board, id: u8) -> Option<&'static Layout> {
    board
        .by_id
        .iter()
        .find(|(u, _)| *u == id)
        .map(|&(_, l)| l)
        .or(board.layout)
        .and_then(|l| LAYOUTS.get(l as usize))
}

/// Colour slots in firmware order; slots no key fills stay black.
fn slots(layout: &Layout, colors: &[Color]) -> Vec<u8> {
    let n = layout
        .keys
        .iter()
        .map(|&k| k as usize + 1)
        .max()
        .unwrap_or(0)
        .max(layout.keys.len());
    let mut b = vec![0u8; n * 3];
    for (&k, &c) in layout.keys.iter().zip(colors) {
        let o = k as usize * 3;
        b[o..o + 3].copy_from_slice(&rgb(c));
    }
    b
}

/// StreamLightingData: `[0x00, 0x24, first LED, LED count, RGB...]`, `chunk` LEDs per report.
fn stream(layout: &Layout, chunk: usize, colors: &[Color]) -> Vec<Vec<u8>> {
    slots(layout, colors)
        .chunks(chunk.max(1) * 3)
        .enumerate()
        .map(|(i, rgb)| {
            let mut p = vec![0x00, STREAM, (i * chunk) as u8, (rgb.len() / 3) as u8];
            p.extend_from_slice(rgb);
            p
        })
        .collect()
}

struct Qmk {
    layout: &'static Layout,
    chunk: usize,
}

impl Proto for Qmk {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.write(&[0x00, EFFECT_ON], REPORT)?;
        io.sleep(GAP_MS);
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        drain(io, REPORT);
        for p in stream(self.layout, self.chunk, colors) {
            io.write(&p, REPORT)?;
        }
        Ok(())
    }
    fn keepalive_ms(&self) -> Option<u64> {
        Some(1000)
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = io.write(&[0x00, EFFECT_OFF], REPORT);
    }
}

fn device(info: &Info, name: &str, l: &Layout) -> Device {
    let (w, h) = (l.w.max(1) as u32, l.h.max(1) as u32);
    let rect = if w <= 8 {
        [0.64, 0.58, 0.12, 0.24]
    } else {
        [0.06, 0.58, 0.54, 0.32]
    };
    let mut d = Device::grid(
        &info.device_id(FAMILY),
        name,
        DeviceKind::Keyboard,
        rect,
        w,
        h,
    );
    d.leds = l
        .pos
        .iter()
        .map(|&(x, y)| [(x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32])
        .collect();
    d
}

pub fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let board = BOARDS
        .iter()
        .find(|b| b.vid == info.vid && b.pid == info.pid)
        .ok_or("not a known QMK board")?;
    let mut io: Box<dyn Io> = match open_sibling(api, info, Some(1), Some(RAW_PAGE), None) {
        Ok(h) => Box::new(h),
        Err(_) => open_io(api, info)?,
    };
    let id = handshake(io.as_mut())?;
    let layout = pick(board, id).ok_or("unknown QMK layout id")?;
    let dev = device(info, d.name, layout);
    let proto = Qmk {
        layout,
        chunk: board.chunk as usize,
    };
    Ok(Driver::new(io, dev, FAMILY, proto).boxed())
}

#[cfg(test)]
mod tests {
    use super::super::boards::DETECTORS;
    use super::*;
    use std::collections::VecDeque;

    #[derive(Default)]
    struct Rec {
        writes: Vec<Vec<u8>>,
        replies: VecDeque<Vec<u8>>,
    }

    impl Io for Rec {
        fn write(&mut self, data: &[u8], len: usize) -> Result<(), String> {
            self.writes.push(crate::hid::padded(data, len));
            Ok(())
        }
        fn feature(&mut self, _: &[u8], _: usize) -> Result<(), String> {
            Ok(())
        }
        fn get_feature(&mut self, _: u8, _: usize) -> Result<Vec<u8>, String> {
            Ok(Vec::new())
        }
        fn read(&mut self, _: usize, _: i32) -> Result<Vec<u8>, String> {
            Ok(self.replies.pop_front().unwrap_or_default())
        }
        fn sleep(&mut self, _: u64) {}
    }

    fn answered(prefix: bool) -> Rec {
        let mut r = Rec::default();
        let p = |v: &[u8]| {
            let mut b = if prefix { vec![0] } else { vec![] };
            b.extend_from_slice(v);
            b.resize(32 + prefix as usize, 0);
            b
        };
        // drain() before the first query, then one reply and one empty drain per query
        r.replies.push_back(Vec::new());
        for v in [
            &[FIRMWARE_TYPE, FIRMWARE_VIA][..],
            &[QMK_VERSION, 0, 24, 0],
            &[PROTOCOL_VERSION, 1, 0, 6],
            &[UNIQUE_ID, 0, 0, 6],
            &[TOTAL_LEDS, 98],
        ] {
            r.replies.push_back(p(v));
            r.replies.push_back(Vec::new());
        }
        r
    }

    #[test]
    fn handshake_reads_replies_with_or_without_report_id() {
        for prefix in [false, true] {
            let mut io = answered(prefix);
            assert_eq!(handshake(&mut io), Ok(6));
            let cmds: Vec<u8> = io.writes.iter().map(|w| w[1]).collect();
            assert_eq!(cmds, [0x28, 0x21, 0x22, 0x23, 0x27]);
            assert!(io.writes.iter().all(|w| w.len() == REPORT && w[0] == 0));
        }
    }

    #[test]
    fn stock_firmware_is_refused() {
        let mut io = Rec::default();
        io.replies.push_back(vec![0xFF, FIRMWARE_TYPE]);
        assert!(handshake(&mut io).is_err());
    }

    #[test]
    fn layout_by_unique_id() {
        let pro = BOARDS
            .iter()
            .find(|b| b.pid == 0x5044 && b.vid == 0x320F)
            .unwrap();
        assert_eq!(pro.by_id.len(), 1);
        assert!(std::ptr::eq(
            pick(pro, 0).unwrap(),
            &LAYOUTS[pro.layout.unwrap() as usize]
        ));
        assert!(std::ptr::eq(
            pick(pro, 6).unwrap(),
            &LAYOUTS[pro.by_id[0].1 as usize]
        ));
        let gmmk3 = BOARDS.iter().find(|b| b.vid == 0x504B).unwrap();
        assert!(pick(gmmk3, 0).is_none());
        assert!(pick(gmmk3, 13).is_some());
    }

    #[test]
    fn stream_packets() {
        static L: Layout = Layout {
            w: 3,
            h: 1,
            keys: &[2, 0, 11],
            pos: &[(0, 0), (1, 0), (2, 0)],
        };
        let c = [Color(1, 2, 3), Color(4, 5, 6), Color(7, 8, 9)];
        let p = stream(&L, 9, &c);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0][..4], [0x00, 0x24, 0, 9]);
        assert_eq!(p[0][4..13], [4, 5, 6, 0, 0, 0, 1, 2, 3]);
        assert_eq!(p[1][..4], [0x00, 0x24, 9, 3]);
        assert_eq!(p[1][10..13], [7, 8, 9]);
        assert!(p.iter().all(|x| x.len() <= REPORT));
    }

    #[test]
    fn table_is_consistent() {
        assert_eq!(BOARDS.len(), DETECTORS.len());
        for (b, d) in BOARDS.iter().zip(DETECTORS) {
            assert_eq!((b.vid, b.pid), (d.vid, d.pid));
            assert_eq!(d.interface, Some(1));
            assert!(b.layout.is_some() || !b.by_id.is_empty());
            assert!(b.chunk as usize * 3 + 4 <= REPORT);
        }
        for l in LAYOUTS {
            assert_eq!(l.keys.len(), l.pos.len());
            let slots = l
                .keys
                .iter()
                .map(|&k| k as usize + 1)
                .max()
                .unwrap()
                .max(l.keys.len());
            assert!(slots <= 256);
            assert!(
                l.pos
                    .iter()
                    .all(|&(x, y)| (x as u16) < l.w && (y as u16) < l.h)
            );
        }
        let mut ids: Vec<_> = BOARDS.iter().map(|b| (b.vid, b.pid)).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), BOARDS.len());
    }
}
