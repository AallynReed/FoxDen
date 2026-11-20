//! Ported from OpenRGB Controllers/CoolerMasterController/CMARGBController (GPL-2.0-or-later).

use foxden_core::{Backend, Color, DeviceKind};
use hidapi::{HidApi, HidDevice};

use super::{Driver, Lights, det, device, report, rows, write};
use crate::registry::{Detector, Info};

const PORT_IDS: [u8; 5] = [0x01, 0x02, 0x04, 0x08, 0xFE];
const RGB_PORT: usize = 4;
const ARGB_DIRECT: u8 = 0xFE;
const RGB_STATIC: u8 = 0x05;
const RGB_OFF: u8 = 0x06;
/// OpenRGB leaves header sizes to the user, up to 48; FoxDen starts each at 48.
const LEDS: usize = 48;
const FIRMWARE: [&str; 2] = ["A202011171238", "A202105291658"];

pub static DETECTORS: &[Detector] = &[det!(
    "Cooler Master ARGB",
    0x2516,
    0x1011,
    Some(0),
    Some(0xFF00),
    Some(1),
    open
)];

pub type Report = [u8; 65];

/// SetPortLEDCount
pub fn led_count(port: usize, count: u8) -> Report {
    report(&[0x80, 0x0D, 0x02, PORT_IDS[port], count, 0x01])
}

/// SetPortMode: one packet for Direct on an ARGB header, two otherwise.
pub fn port_mode(
    port: usize,
    mode: u8,
    speed: u8,
    brightness: u8,
    random: bool,
    c: Color,
) -> Vec<Report> {
    let argb = PORT_IDS[port] != PORT_IDS[RGB_PORT];
    if mode == ARGB_DIRECT {
        return vec![report(&[0x80, 0x01, 0x01, 0x02])];
    }
    let passthru = mode == 0xFF;
    let function = match (passthru, argb) {
        (true, true) => 0x02,
        (true, false) => 0x04,
        (false, true) => 0x01,
        (false, false) => 0x03,
    };
    let first = report(&[0x80, 0x01, function]);
    let mut second = first;
    let colour_index = if random { 0x00 } else { 0x10 };
    if argb {
        second[2..12].copy_from_slice(&[
            0x0B,
            0x02,
            PORT_IDS[port],
            mode,
            colour_index,
            speed,
            brightness,
            c.0,
            c.1,
            c.2,
        ]);
    } else {
        second[2] = if passthru { 0x01 } else { 0x04 };
        second[3..10].copy_from_slice(&[mode, colour_index, speed, brightness, c.0, c.1, c.2]);
    }
    vec![first, second]
}

/// DeviceUpdateMode for Direct: every ARGB header to Direct, the RGB header off.
pub fn direct_mode() -> Vec<Report> {
    let mut out = Vec::new();
    for port in 0..PORT_IDS.len() {
        let mode = if port == RGB_PORT {
            RGB_OFF
        } else {
            ARGB_DIRECT
        };
        out.extend(port_mode(port, mode, 0, 0, false, Color::BLACK));
    }
    out
}

/// SetPortLEDsDirect: note byte 3 is the port index, not its id.
pub fn port_direct(port: usize, colors: &[Color]) -> Vec<Report> {
    let bytes: Vec<u8> = colors.iter().flat_map(|c| [c.0, c.1, c.2]).collect();
    let mut out = Vec::new();
    let mut it = bytes.iter().peekable();
    let mut start = 5;
    let mut r = report(&[0x00, 0x07, port as u8, colors.len() as u8]);
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

/// DeviceUpdateLEDs: each ARGB header, then the RGB header in static mode.
pub fn frame(colors: &[Color], counts: &[usize; RGB_PORT]) -> Vec<Report> {
    let mut out = Vec::new();
    let mut at = 0;
    for (port, &n) in counts.iter().enumerate() {
        out.extend(port_direct(port, &colors[at..at + n]));
        at += n;
    }
    out.extend(port_mode(RGB_PORT, RGB_STATIC, 0, 255, false, colors[at]));
    out
}

/// Each ARGB header at the user's length.
fn sized(id: &str) -> [usize; RGB_PORT] {
    std::array::from_fn(|i| {
        crate::lengths::length(id, i, &format!("Header {}", i + 1), LEDS, 0, LEDS)
    })
}

struct Argb {
    hid: HidDevice,
    counts: [usize; RGB_PORT],
}

impl Driver for Argb {
    fn frame(&mut self, colors: &[Color], first: bool) -> Result<(), String> {
        if first {
            for (port, &n) in self.counts.iter().enumerate() {
                write(&self.hid, &led_count(port, n as u8))?;
            }
            for r in direct_mode() {
                write(&self.hid, &r)?;
            }
        }
        for r in frame(colors, &self.counts) {
            write(&self.hid, &r)?;
        }
        Ok(())
    }
}

fn open(api: &HidApi, info: &Info, det: &Detector) -> Result<Box<dyn Backend>, String> {
    if !FIRMWARE.contains(&info.serial.as_str()) {
        return Err("unsupported firmware".into());
    }
    let hid = info.open(api)?;
    let id = info.device_id("coolermaster");
    let counts = sized(&id);
    let [a, b, c, d] = counts;
    let dev = device(
        id,
        det.name,
        DeviceKind::Strip,
        rows(&[a, b, c, d, 1]),
        [LEDS as u32, 5],
    );
    Ok(Lights::boxed(
        "cooler master argb",
        dev,
        None,
        Argb { hid, counts },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    // CMARGBController::SetPortLEDCount
    #[test]
    fn led_count_bytes() {
        assert_eq!(
            &led_count(2, 48)[..8],
            &[0x00, 0x80, 0x0D, 0x02, 0x04, 48, 0x01, 0]
        );
    }

    // CMARGBController::SetPortMode for Direct and for the RGB header
    #[test]
    fn port_mode_bytes() {
        let d = port_mode(3, ARGB_DIRECT, 0, 0, false, Color::BLACK);
        assert_eq!(d, vec![report(&[0x80, 0x01, 0x01, 0x02])]);
        let rgb = port_mode(RGB_PORT, RGB_STATIC, 0, 255, false, Color(1, 2, 3));
        assert_eq!(rgb[0], report(&[0x80, 0x01, 0x03]));
        assert_eq!(
            &rgb[1][..12],
            &[0x00, 0x80, 0x04, 0x05, 0x10, 0x00, 0xFF, 1, 2, 3, 0, 0]
        );
        let argb = port_mode(1, 0x04, 2, 0x80, true, Color(1, 2, 3));
        assert_eq!(argb[0], report(&[0x80, 0x01, 0x01]));
        assert_eq!(
            &argb[1][..13],
            &[
                0x00, 0x80, 0x0B, 0x02, 0x02, 0x04, 0x00, 0x02, 0x80, 1, 2, 3, 0
            ]
        );
    }

    // RGBController_CMARGBController::DeviceUpdateZoneMode with Direct active
    #[test]
    fn direct_mode_sequence() {
        let seq = direct_mode();
        assert_eq!(seq.len(), 6);
        assert!(
            seq[..4]
                .iter()
                .all(|r| *r == report(&[0x80, 0x01, 0x01, 0x02]))
        );
        assert_eq!(seq[4], report(&[0x80, 0x01, 0x03]));
        assert_eq!(
            &seq[5][..10],
            &[0x00, 0x80, 0x04, 0x06, 0x10, 0, 0, 0, 0, 0]
        );
    }

    // CMARGBController::SetPortLEDsDirect
    #[test]
    fn port_direct_bytes() {
        let colors: Vec<Color> = (0..48u8).map(|i| Color(i, i, i)).collect();
        let p = port_direct(2, &colors);
        assert_eq!(p.len(), 3);
        assert_eq!(&p[0][..8], &[0x00, 0x00, 0x07, 0x02, 48, 0, 0, 0]);
        // 60 bytes in packet 0 hold LEDs 0..19 exactly
        assert_eq!(&p[0][62..65], &[19, 19, 19]);
        assert_eq!(&p[1][..3], &[0x00, 0x01, 20]);
        assert_eq!(&p[2][..2], &[0x00, 0x82]);
        assert_eq!(p[2][2 + 20], 47);
        assert!(p[2][23..].iter().all(|&b| b == 0));
    }

    // RGBController_CMARGBController::DeviceUpdateZoneLEDs over all zones
    #[test]
    fn frame_covers_every_header() {
        let mut colors = vec![Color::BLACK; 4 * LEDS + 1];
        colors[4 * LEDS] = Color(9, 8, 7);
        let f = frame(&colors, &[LEDS; 4]);
        assert_eq!(f.len(), 4 * 3 + 2);
        assert_eq!(f[3][3], 1);
        assert_eq!(&f[13][3..10], &[0x05, 0x10, 0x00, 0xFF, 9, 8, 7]);
    }

    #[test]
    fn a_set_length_shortens_a_header() {
        crate::lengths::set_one("hid:cm-argb-test", 1, 10);
        crate::lengths::set_one("hid:cm-argb-test", 3, 0);
        let counts = sized("hid:cm-argb-test");
        assert_eq!(counts, [48, 10, 48, 0]);
        let mut colors = vec![Color::BLACK; 107];
        colors[106] = Color(9, 8, 7);
        let f = frame(&colors, &counts);
        assert_eq!(f.len(), 3 + 1 + 3 + 2);
        assert_eq!(f[3][4], 10);
        assert_eq!(&f[8][3..10], &[0x05, 0x10, 0x00, 0xFF, 9, 8, 7]);
    }
}
