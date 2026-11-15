//! Ported from OpenRGB Controllers/SkydimoSerialController (GPL-2.0-or-later).

use super::common::*;
use super::serial::{Port, Serial};
use foxrgb_core::{Backend, Color, Device, DeviceKind};

const FAMILY: &str = "skydimo-serial";
pub const IDS: (u16, u16) = (0x1A86, 0x7523);
const BAUD: u32 = 115_200;
const QUERY: &[u8] = b"Moni-A";
const MODEL_LEN: usize = 6;
const MAX_RESPONSE: usize = 63;
const ATTEMPTS: usize = 10;
const DEFAULT_MAX: usize = 150;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Layout {
    Free,
    Strip,
    Perimeter3,
    Perimeter4,
    Sides2,
}
use Layout::*;

#[rustfmt::skip]
const MODELS: &[(&str, Layout, [usize; 4], usize, usize)] = &[
    ("SK0121", Perimeter3, [13, 25, 13, 0], 51, 0),
    ("SK0124", Perimeter3, [14, 26, 14, 0], 54, 0),
    ("SK0127", Perimeter3, [17, 31, 17, 0], 65, 0),
    ("SK0132", Perimeter3, [20, 37, 20, 0], 77, 0),
    ("SK0134", Perimeter3, [15, 41, 15, 0], 71, 0),
    ("SK0149", Perimeter3, [19, 69, 19, 0], 107, 0),
    ("SK0201", Sides2, [20, 20, 0, 0], 40, 0),
    ("SK0202", Sides2, [30, 30, 0, 0], 60, 0),
    ("SK0204", Sides2, [25, 25, 0, 0], 50, 0),
    ("SK0402", Strip, [72, 0, 0, 0], 72, 0),
    ("SK0403", Strip, [96, 0, 0, 0], 96, 0),
    ("SK0404", Strip, [144, 0, 0, 0], 144, 0),
    ("SK0410", Free, [0, 0, 0, 0], 0, 300),
    ("SK0801", Strip, [2, 0, 0, 0], 2, 0),
    ("SK0802", Strip, [18, 0, 0, 0], 18, 0),
    ("SK0803", Strip, [10, 0, 0, 0], 10, 0),
    ("SK0901", Strip, [14, 0, 0, 0], 14, 0),
    ("SK0E01", Strip, [16, 0, 0, 0], 16, 0),
    ("SK0F01", Sides2, [29, 29, 0, 0], 58, 0),
    ("SK0F02", Sides2, [25, 25, 0, 0], 50, 0),
    ("SK0H01", Strip, [2, 0, 0, 0], 2, 0),
    ("SK0H02", Strip, [4, 0, 0, 0], 4, 0),
    ("SK0K01", Strip, [120, 0, 0, 0], 120, 0),
    ("SK0K02", Strip, [15, 0, 0, 0], 15, 0),
    ("SK0L21", Perimeter4, [13, 25, 13, 25], 76, 0),
    ("SK0L24", Perimeter4, [14, 26, 14, 26], 80, 0),
    ("SK0L27", Perimeter4, [17, 31, 17, 31], 96, 0),
    ("SK0L32", Perimeter4, [20, 37, 20, 37], 114, 0),
    ("SK0L34", Perimeter4, [15, 41, 15, 41], 112, 0),
    ("SK0M01", Strip, [24, 0, 0, 0], 24, 0),
    ("SK0N01", Strip, [256, 0, 0, 0], 256, 0),
    ("SK0N02", Strip, [1024, 0, 0, 0], 1024, 0),
    ("SK0N03", Strip, [253, 0, 0, 0], 253, 0),
    ("SK0S01", Strip, [32, 0, 0, 0], 32, 0),
    ("SKA124", Perimeter3, [18, 34, 18, 0], 70, 0),
    ("SKA127", Perimeter3, [20, 41, 20, 0], 81, 0),
    ("SKA132", Perimeter3, [25, 45, 25, 0], 95, 0),
    ("SKA134", Perimeter3, [21, 51, 21, 0], 93, 0),
    ("SKB124", Perimeter4, [18, 34, 18, 34], 104, 0),
    ("SKB127", Perimeter4, [20, 41, 20, 41], 122, 0),
    ("SKB132", Perimeter4, [25, 44, 25, 44], 138, 0),
    ("SKB134", Perimeter4, [21, 50, 21, 50], 142, 0),
];

/// QueryInfo: the model id the device answers `Moni-A` with.
pub fn query_model(io: &mut dyn Io) -> Option<String> {
    io.flush(true, false);
    if io.write(QUERY).ok()? != QUERY.len() {
        return None;
    }
    let mut r: Vec<u8> = Vec::new();
    for _ in 0..ATTEMPTS {
        let mut buf = [0u8; 64];
        let n = io.read(&mut buf, 0).ok()?;
        r.extend(&buf[..n]);
        if r.len() >= MAX_RESPONSE {
            break;
        }
        let last = r.last().copied();
        let prev = if r.len() >= 2 { r[r.len() - 2] } else { 0 };
        if matches!(last, Some(b'\r' | b'\n')) || prev == b'\r' {
            break;
        }
    }
    while matches!(r.last(), Some(b'\r' | b'\n')) {
        r.pop();
    }
    if r.len() < MODEL_LEN || !r[0].eq_ignore_ascii_case(&b'S') || !r[1].eq_ignore_ascii_case(&b'K')
    {
        return None;
    }
    if r.iter()
        .position(|&b| b == b',')
        .is_some_and(|i| i < MODEL_LEN)
    {
        return None;
    }
    Some(String::from_utf8_lossy(&r[..MODEL_LEN]).to_ascii_uppercase())
}

/// ConfigureOutput: LED count and matrix map (`width`, cells) of a model.
pub fn output(model: &str) -> (usize, Option<(usize, Vec<u32>)>) {
    let Some(&(_, layout, z, total, max)) = MODELS.iter().find(|m| m.0 == model) else {
        return (DEFAULT_MAX, None);
    };
    if layout == Free {
        return (if max != 0 { max } else { DEFAULT_MAX }, None);
    }
    if layout == Strip {
        return (total, None);
    }
    let (h, w) = match layout {
        Perimeter4 => (z[0].max(z[2]) + 2, z[1].max(z[3]) + 2),
        Perimeter3 => (z[0].max(z[2]) + 1, z[1] + 2),
        _ => {
            let h = z[0].max(z[1]) + 2;
            (h, 3.max((16 * h + 4) / 9))
        }
    };
    let mut map = vec![NA; w * h];
    let mut led = 0u32;
    let mut set = |y: usize, x: usize, map: &mut Vec<u32>| {
        if y < h && x < w {
            map[y * w + x] = led;
            led += 1;
        }
    };
    let (col, top) = if layout == Sides2 {
        (0, h - 2)
    } else if layout == Perimeter3 {
        (w - 1, h - 1)
    } else {
        (w - 1, h - 2)
    };
    for y in (1..=top).rev().take(z[0]) {
        set(y, col, &mut map);
    }
    if matches!(layout, Perimeter3 | Perimeter4) {
        for x in (1..=w - 2).rev().take(z[1]) {
            set(0, x, &mut map);
        }
    } else {
        for y in (1..=h - 2).take(z[1]) {
            set(y, w - 1, &mut map);
        }
    }
    let end = if layout == Perimeter3 { h - 1 } else { h - 2 };
    for y in (1..=end).take(z[2]) {
        set(y, 0, &mut map);
    }
    if layout == Perimeter4 {
        for x in (1..=w - 2).take(z[3]) {
            set(h - 1, x, &mut map);
        }
    }
    (total, Some((w, map)))
}

/// WriteFrame: Adalight-style header with the LED count less one, then RGB.
pub fn frame_packet(colors: &[Color]) -> Vec<u8> {
    let len = colors.len().saturating_sub(1);
    let mut b = vec![0x41, 0x64, 0x61, 0x00, (len >> 8) as u8, len as u8];
    for c in colors {
        b.extend([c.0, c.1, c.2]);
    }
    b
}

pub struct Skydimo;

impl Proto for Skydimo {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let p = frame_packet(colors);
        let mut done = 0;
        while done < p.len() {
            match io.write(&p[done..])? {
                0 => return Err("serial write stalled".into()),
                n => done += n,
            }
        }
        Ok(())
    }
    fn keepalive_ms(&self) -> Option<u64> {
        Some(1500)
    }
}

fn device(port: &Port, model: &str) -> Device {
    let id = named_id(FAMILY, &port.name);
    let name = format!("Skydimo {model}");
    match output(model) {
        (n, Some((w, map))) => mapped(id, &name, DeviceKind::Strip, &map, w, n),
        (n, None) if free(model) => {
            let n = crate::lengths::length(&id, 0, "Strip", n, 1, n);
            rows(id, &name, DeviceKind::Strip, &[n])
        }
        (n, None) => rows(id, &name, DeviceKind::Strip, &[n]),
    }
}

/// Models whose length is the user's, up to the model's maximum.
fn free(model: &str) -> bool {
    MODELS
        .iter()
        .find(|m| m.0 == model)
        .is_none_or(|m| m.1 == Free)
}

/// Opens the port and keeps it only when a Skydimo answers.
pub fn open(port: &Port) -> Option<Box<dyn Backend>> {
    let mut io = Serial::open(&port.name, BAUD, false).ok()?;
    let model = query_model(&mut io)?;
    Some(boxed(Box::new(io), device(port, &model), FAMILY, Skydimo))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    #[test]
    fn model_query() {
        let mut model = None;
        let ops = record(vec![b"sk0l24,".to_vec(), vec![1, 2, b'\r', b'\n']], |io| {
            model = query_model(io)
        });
        assert_eq!(model.as_deref(), Some("SK0L24"));
        assert_eq!(
            ops,
            vec![
                Op::Flush(true, false),
                w(b"Moni-A".to_vec()),
                Op::R(64),
                Op::R(64)
            ]
        );
        record(vec![b"SK01,23".to_vec()], |io| model = query_model(io));
        assert_eq!(model, None);
    }

    #[test]
    fn layouts() {
        let (n, map) = output("SK0121");
        let (w, map) = map.unwrap();
        assert_eq!((n, w, map.len()), (51, 27, 27 * 14));
        assert_eq!(map[13 * w + 26], 0);
        assert_eq!(map[w + 26], 12);
        assert_eq!(map[25], 13);
        assert_eq!(map[1], 37);
        assert_eq!(map[w], 38);
        assert_eq!(map[13 * w], 50);
        assert_eq!(map.iter().filter(|&&m| m != NA).count(), 51);
        let (n, map) = output("SK0204");
        let (w, map) = map.unwrap();
        assert_eq!((n, w, map.len() / w), (50, 48, 27));
        assert_eq!(map[25 * w], 0);
        assert_eq!(map[w + w - 1], 25);
        assert_eq!(output("SK0L24").0, 80);
        assert_eq!(output("SK0402"), (72, None));
        assert_eq!(output("SK0410"), (300, None));
        assert_eq!(output("SK9999"), (150, None));
    }

    #[test]
    fn free_models_take_a_set_length() {
        let port = |name: &str| Port {
            name: name.into(),
            vid: IDS.0,
            pid: IDS.1,
        };
        crate::lengths::set_one("hid:skydimo-serial-com93", 0, 90);
        assert_eq!(device(&port("COM93"), "SK0410").leds.len(), 90);
        crate::lengths::set_one("hid:skydimo-serial-com94", 0, 90);
        assert_eq!(device(&port("COM94"), "SK0402").leds.len(), 72);
    }

    #[test]
    fn frame() {
        let [first, again, _] = pushes(Skydimo, 2, vec![], &[Color(1, 2, 3), Color(4, 5, 6)], &[]);
        assert_eq!(
            first,
            vec![w(vec![0x41, 0x64, 0x61, 0, 0, 1, 1, 2, 3, 4, 5, 6])]
        );
        assert!(again.is_empty());
    }
}
