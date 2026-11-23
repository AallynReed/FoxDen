// SPDX-License-Identifier: GPL-2.0-or-later

use std::time::Duration;

use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

use super::{Driver, Io, NA, at, backend, device, matrix, on_i, on_ipu, open_io};
use crate::registry::{Detector, Info};

const LEDS: usize = 131;
const MODE_DIRECT: u8 = 0x01;
const PAUSE: Duration = Duration::from_micros(300);

#[rustfmt::skip]
const MAP_US: [u32; 7 * 21] = [
    NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA,  NA,  NA,  NA,  NA,  NA,
     5, NA, 17, 23, 29, 35, 41, 47, 53, 59, 65, 71, 77, 83, 89, 95, 101, 127, 128, 129, 130,
     4, 10, 16, 22, 28, 34, 40, 46, 52, 58, 64, 70, 76, 82, 88, 94, 100, 106, 112, 118, 124,
     3,  9, 15, 21, 27, 33, 39, 45, 51, 57, 63, 69, 75,  7, 87, 93,  99, 105, 111, 117, 123,
     2,  8, 14, 20, 26, 32, 38, 44, 50, 56, 62, 68, NA, 80, NA, NA,  NA, 104, 110, 116,  NA,
     1, NA, 13, 19, 25, 31, 37, 43, 49, 55, 61, 67, 79, NA, NA, 91,  NA, 103, 109, 115, 122,
     0,  6, 12, NA, NA, NA, 36, NA, NA, NA, 60, 66, 72, 78, 84, 90,  96, 102,  NA, 114,  NA,
];

#[rustfmt::skip]
const MAP_EU: [u32; 7 * 21] = [
    NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA,  NA, 126,  NA,  NA,  NA,
     5, NA, 17, 23, 29, 35, 41, 47, 53, 59, 65, 71, 77, 83, 89, 95, 101, 127, 128, 129, 130,
     4, 10, 16, 22, 28, 34, 40, 46, 52, 58, 64, 70, 76, 82, 88, 94, 100, 106, 112, 118, 124,
     3,  9, 15, 21, 27, 33, 39, 45, 51, 57, 63, 69, 75, NA, 87, 93,  99, 105, 111, 117, 123,
     2,  8, 14, 20, 26, 32, 38, 44, 50, 56, 62, 68, 81, 80, NA, NA,  NA, 104, 110, 116,  NA,
     1,  7, 13, 19, 25, 31, 37, 43, 49, 55, 61, 67, 79, NA, NA, 91,  NA, 103, 109, 115, 122,
     0,  6, 12, NA, NA, NA, 36, NA, NA, NA, 60, 66, 72, 78, 84, 90,  96, 102,  NA, 114,  NA,
];

/// `DasKeyboardController::SendDataModern`: one feature report, retried up to three more times.
fn send_modern(io: &mut dyn Io, data: &[u8]) -> Result<(), String> {
    let mut buf = vec![1u8];
    buf.extend_from_slice(data);
    buf.push(data.iter().fold(0, |x, b| x ^ b));
    let mut retries = 3;
    loop {
        let sent = io.feature(&buf);
        if let Err(e) = &sent {
            if retries == 0 {
                return Err(e.clone());
            }
            retries -= 1;
        }
        io.sleep(PAUSE);
        if sent.is_ok() {
            return Ok(());
        }
    }
}

/// `DasKeyboardController::SendDataTraditional`: 7 data bytes per 8-byte report, checksum last.
/// A failed report restarts at the second chunk with the checksum still running.
fn send_traditional(io: &mut dyn Io, data: &[u8]) -> Result<(), String> {
    let len = data.len();
    let mut retries = 3;
    let mut sum = 0u8;
    let mut idx = 0;
    while idx < len + 1 {
        let mut buf = [0u8; 8];
        buf[0] = 1;
        for (f, slot) in buf.iter_mut().enumerate().skip(1) {
            let t = idx + f - 1;
            if t < len {
                *slot = data[t];
                sum ^= data[t];
            } else if t == len {
                *slot = sum;
            }
        }
        if let Err(e) = io.feature(&buf) {
            idx = 0;
            if retries == 0 {
                return Err(e);
            }
            retries -= 1;
        }
        io.sleep(PAUSE);
        idx += 7;
    }
    Ok(())
}

fn send(io: &mut dyn Io, traditional: bool, data: &[u8]) -> Result<(), String> {
    if traditional {
        send_traditional(io, data)
    } else {
        send_modern(io, data)
    }
}

/// `DasKeyboardController::ReceiveData`: 8-byte feature reads until one starts with 0.
/// Returns the count (-1 on a bad checksum or length) and the payload.
fn receive(io: &mut dyn Io, max: usize) -> (i32, Vec<u8>) {
    let mut got = Vec::new();
    let mut sum = 0u8;
    for _ in 0..max / 7 + 2 {
        let mut buf = [0u8; 8];
        buf[0] = 1;
        if io.get_feature(&mut buf).is_err() {
            break;
        }
        if buf[0] == 0 {
            break;
        }
        for b in buf {
            got.push(b);
            sum ^= b;
        }
    }
    let mut data = vec![0u8; max];
    if sum != 0 {
        return (-1, data);
    }
    let mut size = 0usize;
    if got.len() > 1 {
        size = got[1] as usize;
        if size + 2 > got.len() {
            return (-1, data);
        }
        size = size.min(max);
        let n = size.saturating_sub(1);
        data[..n].copy_from_slice(&got[2..2 + n]);
    }
    (size as i32, data)
}

/// `DasKeyboardController::SendInitialize`: modern sends first, traditional when nothing answers.
/// Returns the version bytes and whether traditional sends stuck.
fn initialize(io: &mut dyn Io) -> Result<(Vec<u8>, bool), String> {
    let mut traditional = false;
    let data = loop {
        // SendData returns nothing; a dead modern send just reads back 0.
        let _ = send(io, traditional, &[0xEA, 0x02, 0xB0]);
        let (n, data) = receive(io, 65);
        if n != 0 || traditional {
            break data;
        }
        traditional = true;
    };
    let version = data[2..].iter().take_while(|&&b| b != 0).copied().collect();
    Ok((version, traditional))
}

#[derive(Debug, PartialEq, Eq)]
enum Layout {
    Us,
    Eu,
}

/// `DasKeyboardController::GetLayoutString`; `None` is "NONE", which drops the device.
fn layout(version: &[u8]) -> Option<Layout> {
    if version.len() < 17 {
        return None;
    }
    match &version[3..5] {
        b"16" | b"63" => Some(Layout::Us),
        _ => Some(Layout::Eu),
    }
}

/// `DasKeyboardController::SendColors`.
fn key_packet(key: usize, c: Color) -> Vec<u8> {
    if key < 130 {
        vec![
            0xEA,
            0x08,
            0x78,
            0x08,
            key as u8,
            MODE_DIRECT,
            c.0,
            c.1,
            c.2,
        ]
    } else {
        vec![0xEA, 0x06, 0x78, 0x06, c.0, c.1, c.2]
    }
}

struct Das {
    traditional: bool,
    shown: Vec<Option<Color>>,
}

impl Driver for Das {
    // RGBController_DasKeyboard::DeviceUpdateZoneLEDs: changed keys, then SendApply.
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for key in 0..LEDS {
            let c = at(colors, key);
            if self.shown[key] == Some(c) {
                continue;
            }
            send(io, self.traditional, &key_packet(key, c))?;
            self.shown[key] = Some(c);
        }
        send(io, self.traditional, &[0xEA, 0x03, 0x78, 0x0A])?;
        receive(io, 9);
        Ok(())
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let mut io = open_io(api, info)?;
    let (version, traditional) = initialize(io.as_mut())?;
    let map: &[u32] = match layout(&version).ok_or("no layout reported")? {
        Layout::Us => &MAP_US,
        Layout::Eu => &MAP_EU,
    };
    let leds = matrix(map, 21, LEDS, |_| [0.98, 0.04]);
    let dev = device(
        info.device_id("das"),
        d.name,
        DeviceKind::Keyboard,
        [21, 7],
        leds,
    );
    let driver = Das {
        traditional,
        shown: vec![None; LEDS],
    };
    Ok(backend(io, dev, driver, "das keyboard"))
}

const VID: u16 = 0x24F0;

pub(super) static DETECTORS: [Detector; 3] = [
    on_ipu("Das Keyboard 4Q RGB", VID, 0x2037, 1, 0x01, 0x80, open),
    on_i("Das Keyboard 5Q RGB", VID, 0x2020, 1, open),
    on_i("Das Keyboard 5QS RGB", VID, 0x209A, 1, open),
];

#[cfg(test)]
mod tests {
    use super::super::test_util::{Fake, Op};
    use super::*;

    fn pause() -> Op {
        Op::Sleep(PAUSE)
    }

    fn read_req() -> Op {
        Op::GetFeature(vec![1, 0, 0, 0, 0, 0, 0, 0])
    }

    fn modern(data: &[u8]) -> Op {
        let mut p = vec![1];
        p.extend_from_slice(data);
        p.push(data.iter().fold(0, |x, b| x ^ b));
        Op::Feature(p)
    }

    /// Three reports as ReceiveData collects them, the last byte balancing the XOR, then the end.
    fn version_reply(head: [u8; 8]) -> Vec<Vec<u8>> {
        let mut r = vec![
            head.to_vec(),
            vec![1, b'6', b'-', b'2', b'1', b'.', b'2', b'7'],
            vec![1, b'.', b'0', b'-', b'0', b'0', b'0', 0],
        ];
        r[2][7] = r.iter().flatten().fold(0, |x, b| x ^ b);
        r.push(vec![0; 8]);
        r
    }

    // DasKeyboardController::SendInitialize + ReceiveData; each report's leading byte stays in
    // the stream, so version[4] is that byte and the layout reads as EU
    #[test]
    fn initialize_reads_version_over_modern_sends() {
        let mut fake = Fake::with(version_reply([1, 20, 0xAA, 0xBB, b'U', b'S', b'-', b'1']));
        let (v, traditional) = initialize(&mut fake).unwrap();
        assert_eq!(v, b"US-1\x016-21.27\x01.0-0".to_vec());
        assert!(!traditional);
        let mut want = vec![modern(&[0xEA, 0x02, 0xB0]), pause()];
        want.extend(std::iter::repeat_n(read_req(), 4));
        assert_eq!(fake.ops(), want);
        assert_eq!(layout(&v), Some(Layout::Eu));
    }

    // DasKeyboardController::SendInitialize falling back to SendDataTraditional
    #[test]
    fn initialize_falls_back_to_traditional() {
        let mut replies = vec![vec![0u8; 8]];
        replies.extend(version_reply([1, 20, 0, 0, b'E', b'U', b'-', b'2']));
        let mut fake = Fake::with(replies);
        let (v, traditional) = initialize(&mut fake).unwrap();
        assert!(traditional);
        assert_eq!(v.len(), 17);
        let sum = 0xEA ^ 0x02 ^ 0xB0;
        assert_eq!(
            fake.ops()[..5],
            [
                modern(&[0xEA, 0x02, 0xB0]),
                pause(),
                read_req(),
                Op::Feature(vec![1, 0xEA, 0x02, 0xB0, sum, 0, 0, 0]),
                pause(),
            ]
        );
    }

    // DasKeyboardController::SendInitialize when SendDataModern gives up after four tries
    #[test]
    fn initialize_falls_back_when_modern_sends_fail() {
        let mut replies = vec![vec![0u8; 8]];
        replies.extend(version_reply([1, 20, 0, 0, b'E', b'U', b'-', b'2']));
        let mut fake = Fake::with(replies);
        *fake.failing_features.lock().unwrap() = 4;
        let (_, traditional) = initialize(&mut fake).unwrap();
        assert!(traditional);
        let m = modern(&[0xEA, 0x02, 0xB0]);
        let sum = 0xEA ^ 0x02 ^ 0xB0;
        let mut want = vec![
            m.clone(),
            pause(),
            m.clone(),
            pause(),
            m.clone(),
            pause(),
            m,
            read_req(),
            Op::Feature(vec![1, 0xEA, 0x02, 0xB0, sum, 0, 0, 0]),
            pause(),
        ];
        want.extend(std::iter::repeat_n(read_req(), 4));
        assert_eq!(fake.ops(), want);
    }

    // DasKeyboardController::ReceiveData: a bad checksum returns -1 and an empty buffer
    #[test]
    fn receive_rejects_bad_checksum() {
        let mut fake = Fake::with(vec![vec![1, 3, 9, 9, 0, 0, 0, 0], vec![0; 8]]);
        let (n, data) = receive(&mut fake, 9);
        assert_eq!(n, -1);
        assert_eq!(data, vec![0; 9]);
    }

    // DasKeyboardController::GetLayoutString
    #[test]
    fn layout_from_version() {
        assert_eq!(layout(b"V1.0"), None);
        assert_eq!(layout(b"EU-63-21.27.0-000"), Some(Layout::Us));
        assert_eq!(layout(b"EU-16-21.27.0-000"), Some(Layout::Us));
        assert_eq!(layout(b"EU-28-21.27.0-000"), Some(Layout::Eu));
    }

    // DasKeyboardController::SendDataModern: a failed report is retried after the pause
    #[test]
    fn modern_send_retries() {
        let mut fake = Fake::default();
        *fake.failing_features.lock().unwrap() = 1;
        send_modern(&mut fake, &[0xEA, 0x03, 0x78, 0x0A]).unwrap();
        let pkt = modern(&[0xEA, 0x03, 0x78, 0x0A]);
        assert_eq!(fake.ops(), vec![pkt.clone(), pause(), pkt, pause()]);
    }

    // DasKeyboardController::SendDataTraditional for SendColors(5, 1, 1, 2, 3)
    #[test]
    fn traditional_send_chunks() {
        let mut fake = Fake::default();
        let data = key_packet(5, Color(1, 2, 3));
        send_traditional(&mut fake, &data).unwrap();
        let sum = data.iter().fold(0, |x, b| x ^ b);
        assert_eq!(
            fake.ops(),
            vec![
                Op::Feature(vec![1, 0xEA, 0x08, 0x78, 0x08, 5, 1, 1]),
                pause(),
                Op::Feature(vec![1, 2, 3, sum, 0, 0, 0, 0]),
                pause(),
            ]
        );
    }

    // RGBController_DasKeyboard::DeviceUpdateZoneLEDs: every key first, then only changed ones
    #[test]
    fn frame_sends_changed_keys_then_apply() {
        let mut das = Das {
            traditional: false,
            shown: vec![None; LEDS],
        };
        let mut colors = vec![Color(0, 0, 0); LEDS];
        colors[130] = Color(7, 8, 9);
        let mut fake = Fake::default();
        das.frame(&mut fake, &colors).unwrap();
        let ops = fake.take();
        assert_eq!(ops.len(), 132 * 2 + 1);
        assert_eq!(ops[0], modern(&[0xEA, 0x08, 0x78, 0x08, 0, 1, 0, 0, 0]));
        assert_eq!(
            ops[2 * 129],
            modern(&[0xEA, 0x08, 0x78, 0x08, 129, 1, 0, 0, 0])
        );
        assert_eq!(ops[2 * 130], modern(&[0xEA, 0x06, 0x78, 0x06, 7, 8, 9]));

        colors[3] = Color(1, 1, 1);
        das.frame(&mut fake, &colors).unwrap();
        assert_eq!(
            fake.take(),
            vec![
                modern(&[0xEA, 0x08, 0x78, 0x08, 3, 1, 1, 1, 1]),
                pause(),
                modern(&[0xEA, 0x03, 0x78, 0x0A]),
                pause(),
                read_req(),
            ]
        );
    }
}
