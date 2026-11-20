//! SteelSeries Arctis 7 and Arctis Nova Pro Wireless headset lights.

use super::common::{Driver, Op, Proto, device, line};
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Arctis7,
    NovaPro,
}

const fn det(name: &'static str, pid: u16, interface: i32, page: Option<u16>) -> Detector {
    Detector {
        name,
        vid: 0x1038,
        pid,
        interface: Some(interface),
        usage_page: page,
        usage: if page.is_some() { Some(0x0202) } else { None },
        open,
    }
}

static DETECTORS: [Detector; 4] = [
    det("SteelSeries Arctis 7", 0x1260, 5, Some(0xFF43)),
    det("SteelSeries Arctis 7 2019", 0x12AD, 5, Some(0xFF43)),
    det("SteelSeries Arctis Nova Pro Wireless", 0x12E0, 4, None),
    det("SteelSeries Arctis Nova Pro Wireless X", 0x12E5, 4, None),
];

pub fn detectors() -> Vec<&'static Detector> {
    DETECTORS.iter().collect()
}

fn kind(pid: u16) -> Option<Kind> {
    Some(match pid {
        0x1260 | 0x12AD => Kind::Arctis7,
        0x12E0 | 0x12E5 => Kind::NovaPro,
        _ => return None,
    })
}

/// Arctis 7 lights are on or off; the Nova Pro base takes strength 1 (off) to 10.
fn level(kind: Kind, c: Color) -> u8 {
    let v = c.0.max(c.1).max(c.2) as u16;
    match kind {
        Kind::Arctis7 => u8::from(v > 0) * 2,
        Kind::NovaPro => 1 + (v * 9).div_ceil(255) as u8,
    }
}

fn packet(kind: Kind, level: u8) -> Vec<u8> {
    let (head, len): (&[u8], usize) = match kind {
        Kind::Arctis7 => (&[0x06, 0x55, 0x01, level], 8),
        Kind::NovaPro => (&[0x06, 0xBF, level], 31),
    };
    let mut b = vec![0u8; len];
    b[..head.len()].copy_from_slice(head);
    b
}

struct Headset {
    kind: Kind,
    last: Option<u8>,
}

impl Proto for Headset {
    fn frame(&mut self, colors: &[Color]) -> Vec<Op> {
        let l = level(self.kind, colors.first().copied().unwrap_or(Color::BLACK));
        if self.last == Some(l) {
            return Vec::new();
        }
        self.last = Some(l);
        vec![Op::Write(packet(self.kind, l))]
    }

    fn reset(&mut self) {
        self.last = None;
    }
}

fn open(api: &HidApi, info: &Info, det: &Detector) -> Result<Box<dyn Backend>, String> {
    let kind = kind(info.pid).ok_or("unknown SteelSeries headset")?;
    let hid = info.open(api)?;
    let dev = device(
        info.device_id("steelseries-headset"),
        det.name,
        DeviceKind::Other,
        line(1),
    );
    Ok(Driver::boxed(
        hid,
        "steelseries headset",
        dev,
        Headset { kind, last: None },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table() {
        assert_eq!(DETECTORS.len(), 4);
        assert!(DETECTORS.iter().all(|d| kind(d.pid).is_some()));
    }

    #[test]
    fn arctis7_on_off_once_per_change() {
        let mut h = Headset {
            kind: Kind::Arctis7,
            last: None,
        };
        assert_eq!(
            h.frame(&[Color(9, 0, 0)]),
            vec![Op::Write(vec![0x06, 0x55, 0x01, 0x02, 0, 0, 0, 0])]
        );
        assert!(h.frame(&[Color(0, 40, 0)]).is_empty());
        assert_eq!(
            h.frame(&[Color::BLACK]),
            vec![Op::Write(vec![0x06, 0x55, 0x01, 0x00, 0, 0, 0, 0])]
        );
    }

    #[test]
    fn nova_pro_strength() {
        let mut h = Headset {
            kind: Kind::NovaPro,
            last: None,
        };
        let ops = h.frame(&[Color(255, 255, 255)]);
        let mut want = vec![0u8; 31];
        want[..3].copy_from_slice(&[0x06, 0xBF, 10]);
        assert_eq!(ops, vec![Op::Write(want)]);
        assert_eq!(level(Kind::NovaPro, Color::BLACK), 1);
        assert_eq!(level(Kind::NovaPro, Color(1, 0, 0)), 2);
    }
}
