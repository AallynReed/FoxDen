//! Ported from OpenRGB Controllers/CoolerMasterController/CMSmallARGBController (GPL-2.0-or-later).

use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::{HidApi, HidDevice};

use super::{Driver, Lights, det, device, exchange, line, report, write};
use crate::registry::{Detector, Info};

const TIMEOUT: i32 = 250;
const DIRECT: u8 = 0xFE;
/// OpenRGB leaves the header size to the user, up to 48; FoxRGB starts at 48.
const LEDS: usize = 48;
const FW0012: &str = "A202104052336";

pub static DETECTORS: &[Detector] = &[det!(
    "Cooler Master Small ARGB",
    0x2516,
    0x1000,
    Some(0),
    Some(0xFF00),
    Some(1),
    open
)];

pub type Report = [u8; 65];

/// SetLedCount
pub fn led_count(zone: u8, count: u8) -> Report {
    let mode = if 0x0F - (count as i32) > 0 {
        0x0F - count
    } else {
        0x01
    };
    report(&[0x80, 0x0D, 0x02, zone, mode, count])
}

/// SendUpdate after SetMode(Direct, speed 0, brightness, black, not random).
pub fn direct_mode(brightness: u8) -> [Report; 2] {
    let first = report(&[0x80, 0x10, 0x01, 0x00, 0x02]);
    let mut second = first;
    second[2..12].copy_from_slice(&[0x0B, 0x02, 0x01, DIRECT, 0x00, 0x10, brightness, 0, 0, 0]);
    [first, second]
}

/// SetLedsDirect: RGB bytes from index 6 of the first packet, then from index 2.
pub fn direct(colors: &[Color]) -> Vec<Report> {
    let bytes: Vec<u8> = colors.iter().flat_map(|c| [c.0, c.1, c.2]).collect();
    let mut out = Vec::new();
    let mut it = bytes.iter().peekable();
    let mut start = 6;
    let mut r = report(&[0x00, 0x10, 0x02, 0xFF, colors.len() as u8]);
    while it.peek().is_some() {
        r[1] = out.len() as u8;
        for slot in &mut r[start..] {
            match it.next() {
                Some(&b) => *slot = b,
                None => break,
            }
        }
        if it.peek().is_none() {
            r[1] += 0x80;
        }
        out.push(r);
        r = [0u8; 65];
        start = 2;
    }
    out
}

/// The header at the user's length.
fn sized(id: &str) -> usize {
    crate::lengths::length(id, 0, "Header 1", LEDS, 1, LEDS)
}

struct SmallArgb {
    hid: HidDevice,
    leds: usize,
}

impl Driver for SmallArgb {
    fn frame(&mut self, colors: &[Color], first: bool) -> Result<(), String> {
        if first {
            write(&self.hid, &led_count(1, self.leds as u8))?;
            let [a, b] = direct_mode(0xFF);
            write(&self.hid, &a)?;
            exchange(&self.hid, &b, TIMEOUT)?;
        }
        for r in direct(colors) {
            write(&self.hid, &r)?;
        }
        Ok(())
    }
}

fn open(api: &HidApi, info: &Info, det: &Detector) -> Result<Box<dyn Backend>, String> {
    if info.serial != FW0012 {
        return Err("unsupported firmware".into());
    }
    let hid = info.open(api)?;
    let id = info.device_id("coolermaster");
    let leds = sized(&id);
    let dev = device(
        id,
        det.name,
        DeviceKind::Strip,
        line(leds),
        [leds as u32, 1],
    );
    Ok(Lights::boxed(
        "cooler master small argb",
        dev,
        None,
        SmallArgb { hid, leds },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_set_length_sizes_the_header() {
        assert_eq!(sized("hid:cm-small-argb-test"), 48);
        crate::lengths::set_one("hid:cm-small-argb-test", 0, 12);
        assert_eq!(sized("hid:cm-small-argb-test"), 12);
    }

    // CMSmallARGBController::SetLedCount
    #[test]
    fn led_count_bytes() {
        assert_eq!(
            &led_count(1, 48)[..8],
            &[0x00, 0x80, 0x0D, 0x02, 0x01, 0x01, 48, 0]
        );
        assert_eq!(
            &led_count(1, 6)[..7],
            &[0x00, 0x80, 0x0D, 0x02, 0x01, 0x09, 6]
        );
    }

    // CMSmallARGBController::SendUpdate in Direct mode
    #[test]
    fn direct_mode_bytes() {
        let [a, b] = direct_mode(0xFF);
        assert_eq!(&a[..7], &[0x00, 0x80, 0x10, 0x01, 0x00, 0x02, 0x00]);
        assert_eq!(
            &b[..13],
            &[
                0x00, 0x80, 0x0B, 0x02, 0x01, 0xFE, 0x00, 0x10, 0xFF, 0, 0, 0, 0
            ]
        );
    }

    // CMSmallARGBController::SetLedsDirect
    #[test]
    fn direct_bytes() {
        let colors: Vec<Color> = (0..48u8).map(|i| Color(i, i, i)).collect();
        let p = direct(&colors);
        assert_eq!(p.len(), 3);
        assert_eq!(&p[0][..9], &[0x00, 0x00, 0x10, 0x02, 0xFF, 48, 0, 0, 0]);
        // 59 bytes in packet 0: LED 19 ends with its red and green
        assert_eq!(&p[0][63..65], &[19, 19]);
        assert_eq!(&p[1][..3], &[0x00, 0x01, 19]);
        assert_eq!(&p[2][..2], &[0x00, 0x82]);
        // 144 - 59 - 63 = 22 bytes left
        assert_eq!(p[2][2 + 21], 47);
        assert!(p[2][24..].iter().all(|&b| b == 0));
        let one = direct(&[Color(1, 2, 3)]);
        assert_eq!(&one[0][..9], &[0x00, 0x80, 0x10, 0x02, 0xFF, 1, 1, 2, 3]);
    }
}
