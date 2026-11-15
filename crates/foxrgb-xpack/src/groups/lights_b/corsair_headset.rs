//! Corsair Virtuoso and HS80 RGB headsets (protocol learned from SignalRGB's
//! Corsair_Headset_Controller.js and Corsair_HS80_RGB_Wired_Gaming_Headset.js).

use super::{HEADSET, boxed, det, placed};
use crate::hid::{Io, Proto, drain, rgb};
use foxrgb_core::{Backend, Color, DeviceKind};
use foxrgb_hid::registry::{Detector, Info};
use hidapi::HidApi;

const FAMILY: &str = "corsair-headset";
const VID: u16 = 0x1B1C;
const REPORT: u8 = 0x02;
const LEN: usize = 64;
const WIRED: u8 = 0x08;
const WIRELESS: u8 = 0x09;
const HW_MODE: u8 = 0x01;
const SW_MODE: u8 = 0x02;
/// Hardware brightness 1000 (100.0%), little endian.
const FULL: [u8; 2] = [0xE8, 0x03];
/// Wireless headsets drop to hardware lighting after sleeping; software mode is asked for again.
const REASSERT_MS: u64 = 10_000;
/// Logo, power, mic on a 3x3 grid.
const LEDS: &[[u8; 2]] = &[[1, 0], [0, 2], [2, 2]];

/// Wireless receivers use link byte 0x09 instead of 0x08.
const WIRELESS_PIDS: &[u16] = &[
    0x0A3E, 0x0A40, 0x0A42, 0x0A44, 0x0A4B, 0x0A4C, 0x0A5C, 0x0A64, 0x0A6B, 0x0A73,
];

/// Page FF42 usage 1 on the interface the model's library lists (either, when it lists both).
const fn modern(pid: u16, name: &'static str, interface: Option<i32>) -> Detector {
    det(name, VID, pid, interface, Some(0xFF42), Some(0x01), open)
}

pub static DETECTORS: &[Detector] = &[
    modern(0x0A3D, "Corsair Virtuoso SE", Some(4)),
    modern(0x0A3E, "Corsair Virtuoso SE Wireless", None),
    modern(0x0A40, "Corsair Virtuoso Wireless", Some(4)),
    modern(0x0A41, "Corsair Virtuoso", None),
    modern(0x0A42, "Corsair Virtuoso Wireless", None),
    modern(0x0A43, "Corsair Virtuoso", Some(4)),
    modern(0x0A44, "Corsair Virtuoso Wireless", Some(3)),
    modern(0x0A4B, "Corsair Virtuoso Wireless", Some(4)),
    modern(0x0A4C, "Corsair Virtuoso Wireless", Some(4)),
    modern(0x0A5A, "Corsair Virtuoso", Some(4)),
    modern(0x0A5C, "Corsair Virtuoso Wireless", Some(4)),
    modern(0x0A62, "Corsair Virtuoso XT", Some(3)),
    modern(0x0A64, "Corsair Virtuoso XT Wireless", Some(3)),
    modern(0x0A69, "Corsair HS80 RGB", Some(3)),
    modern(0x0A6B, "Corsair HS80 RGB Wireless", Some(3)),
    modern(0x0A71, "Corsair HS80 RGB White", Some(3)),
    modern(0x0A73, "Corsair HS80 RGB White Wireless", Some(3)),
    det(
        "Corsair HS80 RGB Wired",
        VID,
        0x0A88,
        Some(3),
        Some(0xFFC5),
        Some(0x01),
        open_hs80_wired,
    ),
];

/// `modernDirectLightingMode` / `Shutdown`: lighting mode property 0x03.
pub fn mode(link: u8, mode: u8) -> Vec<u8> {
    vec![REPORT, link, 0x01, 0x03, 0x00, mode]
}

/// Opens lighting endpoint 1 on handle 0.
pub fn open_lighting(link: u8) -> Vec<u8> {
    vec![REPORT, link, 0x0D, 0x00, 0x01]
}

/// `writeRGB`: nine bytes on handle 0, as an R plane, a G plane and a B plane of three LEDs.
pub fn colors(link: u8, c: &[Color]) -> Vec<u8> {
    let mut b = vec![REPORT, link, 0x06, 0x00, 0x09, 0x00, 0x00, 0x00];
    for plane in 0..3 {
        b.extend((0..3).map(|i| rgb(c.get(i).copied().unwrap_or(Color(0, 0, 0)))[plane]));
    }
    b
}

struct Modern {
    link: u8,
    since: Option<std::time::Instant>,
}

impl Modern {
    fn take(&self, io: &mut dyn Io) -> Result<(), String> {
        io.write(&mode(self.link, SW_MODE), LEN)?;
        io.sleep(100);
        io.write(&open_lighting(self.link), LEN)
    }
}

impl Proto for Modern {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        self.take(io)?;
        io.sleep(100);
        drain(io, LEN);
        io.write(&[REPORT, self.link, 0x02, 0x02, 0x00], LEN)?;
        io.sleep(100);
        let reply = io.read(LEN, 100)?;
        if reply.get(4..6) != Some(&FULL[..]) {
            io.write(
                &[REPORT, self.link, 0x01, 0x02, 0x00, FULL[0], FULL[1]],
                LEN,
            )?;
        }
        self.since = Some(std::time::Instant::now());
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, c: &[Color]) -> Result<(), String> {
        if self.link == WIRELESS
            && self
                .since
                .is_some_and(|t| t.elapsed().as_millis() as u64 >= REASSERT_MS)
        {
            self.take(io)?;
            self.since = Some(std::time::Instant::now());
        }
        drain(io, LEN);
        io.write(&colors(self.link, c), LEN)
    }
    fn keepalive_ms(&self) -> Option<u64> {
        (self.link == WIRELESS).then_some(REASSERT_MS)
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = io.write(&mode(self.link, HW_MODE), LEN);
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let link = if WIRELESS_PIDS.contains(&info.pid) {
        WIRELESS
    } else {
        WIRED
    };
    let dev = placed(
        info,
        FAMILY,
        d.name,
        DeviceKind::Other,
        HEADSET,
        [3, 3],
        LEDS,
    );
    boxed(
        api,
        info,
        "corsair headset",
        dev,
        Modern { link, since: None },
    )
}

/// `sendColors` of the wired HS80: register writes 1C/16/17 for the logo's red, green, blue.
pub fn hs80_wired(c: Color) -> [u8; 8] {
    [0xCB, 0x03, 0x1C, c.0, 0x16, c.1, 0x17, c.2]
}

struct Hs80Wired;

impl Proto for Hs80Wired {
    fn frame(&mut self, io: &mut dyn Io, c: &[Color]) -> Result<(), String> {
        io.write(&hs80_wired(c[0]), 16)
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = io.write(&hs80_wired(Color(0, 0, 0)), 16);
    }
}

fn open_hs80_wired(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = placed(
        info,
        FAMILY,
        d.name,
        DeviceKind::Other,
        HEADSET,
        [1, 1],
        &[[0, 0]],
    );
    boxed(api, info, "corsair headset", dev, Hs80Wired)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::groups::lights_b::rec::*;

    #[test]
    fn planar_colors() {
        let p = colors(WIRELESS, &[Color(1, 2, 3), Color(4, 5, 6), Color(7, 8, 9)]);
        assert_eq!(p, [2, 9, 6, 0, 9, 0, 0, 0, 1, 4, 7, 2, 5, 8, 3, 6, 9]);
    }

    #[test]
    fn start_sets_brightness_when_not_full() {
        let mut r = Rec::default();
        Modern {
            link: WIRED,
            since: None,
        }
        .start(&mut r)
        .unwrap();
        let w = writes(&r);
        assert_eq!(w[0][..6], [2, 8, 1, 3, 0, 2]);
        assert_eq!(w[1][..5], [2, 8, 0x0D, 0, 1]);
        assert_eq!(w[3][..7], [2, 8, 1, 2, 0, 0xE8, 0x03]);
    }

    #[test]
    fn hs80_registers() {
        assert_eq!(
            hs80_wired(Color(9, 8, 7)),
            [0xCB, 3, 0x1C, 9, 0x16, 8, 0x17, 7]
        );
    }
}
