// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "asus-aura-mouse";
const DOCK_FAMILY: &str = "asus-aura-mouse-dock";
const VID: u16 = 0x0B05;
const PNK_LTD: u16 = 0x18CD;
const SPATHA_X_2_4: u16 = 0x1979;
const DOCK: u16 = 0xFFFF;
/// Static is the first entry of every mouse_modes list, so its mode value is 0.
const MODE_STATIC: u8 = 0;

const LOGO: u8 = 0;
const SCROLL: u8 = 1;
const UNDER: u8 = 2;
const ZONE_DOCK: u8 = 4;

/// One aura_mouse_devices entry; `map` is its aura_mouse_led_maps entry when it has native Direct.
pub struct Mouse {
    pub pid: u16,
    pub zones: &'static [u8],
    pub brightness: u8,
    pub map: Option<(u8, &'static [&'static [u8]])>,
}

const fn m(pid: u16, zones: &'static [u8], brightness: u8) -> Mouse {
    Mouse {
        pid,
        zones,
        brightness,
        map: None,
    }
}

const fn dm(pid: u16, zones: &'static [u8], amount: u8, map: &'static [&'static [u8]]) -> Mouse {
    Mouse {
        pid,
        zones,
        brightness: 100,
        map: Some((amount, map)),
    }
}

const LS: &[u8] = &[LOGO, SCROLL];
const LSU: &[u8] = &[LOGO, SCROLL, UNDER];
const L: &[u8] = &[LOGO];
const ONE: &[&[u8]] = &[&[0]];
const CHAKRAM_X: &[&[u8]] = &[&[7], &[8], &[0, 1, 2, 3, 4, 5, 6]];
const SPATHA_X: &[&[u8]] = &[&[0], &[1], &[2, 3, 4]];

/// aura_mouse_devices; 0x189E keeps the first of its two entries, as std::map does.
pub static MICE: &[Mouse] = &[
    m(0x18DD, LS, 4),
    m(0x1845, LSU, 4),
    m(0x1877, LSU, 4),
    m(0x18B1, LSU, 4),
    m(PNK_LTD, &[SCROLL, UNDER], 4),
    m(0x189E, LS, 4),
    m(0x18A0, LS, 4),
    m(0x197B, LSU, 100),
    m(0x1C8D, LS, 100),
    m(0x197D, LSU, 100),
    m(0x197F, LSU, 100),
    m(0x1981, LSU, 100),
    dm(0x1A70, L, 1, ONE),
    dm(0x1A72, L, 1, ONE),
    m(0x18E5, LSU, 4),
    m(0x18E3, LSU, 4),
    m(0x1958, LS, 4),
    dm(0x1A18, LSU, 9, CHAKRAM_X),
    dm(0x1A1A, LSU, 9, CHAKRAM_X),
    dm(0x1977, LSU, 5, SPATHA_X),
    dm(SPATHA_X_2_4, LSU, 5, SPATHA_X),
    m(0x1846, LSU, 4),
    m(0x1906, LSU, 4),
    m(0x1908, LSU, 4),
    m(0x1847, L, 4),
    m(0x18E1, LSU, 4),
    m(0x1956, LSU, 4),
    m(0x19D2, LSU, 4),
    m(0x1947, LS, 4),
    m(0x1949, LS, 4),
    dm(0x1A88, LS, 2, &[&[0], &[1]]),
    m(0x195C, LS, 4),
    m(0x195E, LS, 4),
    m(0x1960, LS, 4),
    m(0x1962, LS, 4),
    dm(0x1A66, L, 1, ONE),
    dm(0x1A68, L, 1, ONE),
    m(0x1910, L, 4),
    m(0x1A9B, L, 100),
    m(0x1898, L, 4),
    m(0x1C56, L, 100),
    m(0x1C57, L, 100),
    m(DOCK, &[ZONE_DOCK], 100),
];

fn mouse(pid: u16) -> Option<&'static Mouse> {
    MICE.iter().find(|m| m.pid == pid)
}

/// SendUpdate's report for a static colour.
pub fn update(pid: u16, zone: u8, c: Color, brightness: u8) -> [u8; 65] {
    if pid == PNK_LTD {
        report(&[
            0x00,
            0x51,
            0x28,
            zone,
            0x00,
            MODE_STATIC,
            brightness,
            0x00,
            c.0,
            c.1,
            c.2,
        ])
    } else {
        report(&[
            0x00,
            0x51,
            0x28,
            zone,
            0x00,
            MODE_STATIC,
            brightness,
            c.0,
            c.1,
            c.2,
        ])
    }
}

/// SendDirect: zone colours spread over the LED map, five LEDs per packet.
pub fn direct(amount: u8, map: &[&[u8]], colors: &[Color]) -> Vec<[u8; 65]> {
    let mut leds = vec![Color::BLACK; amount as usize];
    for (zone, c) in colors.iter().enumerate() {
        for &led in map.get(zone).copied().unwrap_or(&[]) {
            leds[led as usize] = *c;
        }
    }
    leds.chunks(5)
        .enumerate()
        .map(|(i, chunk)| {
            let mut b = report(&[0x00, 0x51, 0x29, chunk.len() as u8, 0x00, (i * 5) as u8]);
            for (j, c) in chunk.iter().enumerate() {
                b[6 + j * 3..9 + j * 3].copy_from_slice(&rgb(*c));
            }
            b
        })
        .collect()
}

struct AuraMouse(&'static Mouse);

impl Proto for AuraMouse {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let m = self.0;
        if let Some((amount, map)) = m.map {
            for p in direct(amount, map, colors) {
                io.write(&p)?;
            }
            return Ok(());
        }
        // Static per zone, each with a flush before and a 10 ms read after
        for (zone, c) in m.zones.iter().zip(colors) {
            flush(io)?;
            io.write(&update(m.pid, *zone, *c, m.brightness))?;
            let mut r = [0u8; 65];
            io.read(&mut r, 10)?;
        }
        Ok(())
    }
}

fn build(
    api: &HidApi,
    info: &Info,
    d: &Detector,
    pid: u16,
    family: &'static str,
) -> Result<Box<dyn Backend>, String> {
    let m = mouse(pid).ok_or("unknown ASUS mouse")?;
    let kind = if pid == DOCK {
        DeviceKind::Other
    } else {
        DeviceKind::Mouse
    };
    let dev = line(info, family, d, kind, m.zones.len());
    boxed(api, info, FAMILY, dev, AuraMouse(m))
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    build(api, info, d, info.pid, FAMILY)
}

/// OpenRGB registers the Spatha X dock on the same collection as the 2.4GHz mouse.
fn spatha_x(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let mouse = build(api, info, d, info.pid, FAMILY)?;
    let dock = build(api, info, &DOCK_DETECTOR, DOCK, DOCK_FAMILY)?;
    Ok(Box::new(Both(vec![mouse, dock])))
}

static DOCK_DETECTOR: Detector = det(
    "Asus ROG Spatha X Dock",
    VID,
    SPATHA_X_2_4,
    0,
    0xFF01,
    ANY,
    spatha_x,
);

pub static DETECTORS: &[Detector] = &[
    det(
        "ASUS ROG Gladius II Core",
        VID,
        0x18DD,
        0,
        0xFF01,
        ANY,
        open,
    ),
    det("ASUS ROG Gladius II", VID, 0x1845, 2, 0xFF01, ANY, open),
    det(
        "ASUS ROG Gladius II Origin",
        VID,
        0x1877,
        2,
        0xFF01,
        ANY,
        open,
    ),
    det(
        "ASUS ROG Gladius II Origin PNK LTD",
        VID,
        PNK_LTD,
        2,
        0xFF01,
        ANY,
        open,
    ),
    det(
        "ASUS ROG Gladius II Origin COD",
        VID,
        0x18B1,
        2,
        0xFF01,
        ANY,
        open,
    ),
    det(
        "ASUS ROG Gladius II Wireless",
        VID,
        0x189E,
        1,
        0xFF13,
        ANY,
        open,
    ),
    det(
        "ASUS ROG Gladius II Wireless",
        VID,
        0x18A0,
        2,
        0xFF01,
        ANY,
        open,
    ),
    det("ASUS ROG Gladius III", VID, 0x197B, 0, 0xFF01, ANY, open),
    det(
        "ASUS ROG Gladius III Core",
        VID,
        0x1C8D,
        0,
        0xFF01,
        ANY,
        open,
    ),
    det(
        "ASUS ROG Gladius III Wireless USB",
        VID,
        0x197D,
        0,
        0xFF01,
        ANY,
        open,
    ),
    det(
        "ASUS ROG Gladius III Wireless 2.4Ghz",
        VID,
        0x197F,
        0,
        0xFF01,
        ANY,
        open,
    ),
    det(
        "ASUS ROG Gladius III Wireless Bluetooth",
        VID,
        0x1981,
        0,
        0xFF01,
        ANY,
        open,
    ),
    det(
        "ASUS ROG Gladius III Wireless AimPoint USB",
        VID,
        0x1A70,
        0,
        0xFF01,
        ANY,
        open,
    ),
    det(
        "ASUS ROG Gladius III Wireless AimPoint 2.4Ghz",
        VID,
        0x1A72,
        0,
        0xFF01,
        ANY,
        open,
    ),
    det(
        "ASUS ROG Chakram (Wireless)",
        VID,
        0x18E5,
        0,
        0xFF01,
        ANY,
        open,
    ),
    det(
        "ASUS ROG Chakram (Wired)",
        VID,
        0x18E3,
        0,
        0xFF01,
        ANY,
        open,
    ),
    det("ASUS ROG Chakram Core", VID, 0x1958, 0, 0xFF01, ANY, open),
    det("ASUS ROG Chakram X USB", VID, 0x1A18, 0, 0xFF01, ANY, open),
    det(
        "ASUS ROG Chakram X 2.4GHz",
        VID,
        0x1A1A,
        0,
        0xFF01,
        ANY,
        open,
    ),
    det("Asus ROG Spatha X USB", VID, 0x1977, 0, 0xFF01, ANY, open),
    det(
        "Asus ROG Spatha X 2.4GHz",
        VID,
        SPATHA_X_2_4,
        0,
        0xFF01,
        ANY,
        spatha_x,
    ),
    det("ASUS ROG Pugio", VID, 0x1846, 2, 0xFF01, ANY, open),
    det(
        "ASUS ROG Pugio II (Wired)",
        VID,
        0x1906,
        0,
        0xFF01,
        ANY,
        open,
    ),
    det(
        "ASUS ROG Pugio II (Wireless)",
        VID,
        0x1908,
        0,
        0xFF01,
        ANY,
        open,
    ),
    det("ASUS ROG Strix Impact", VID, 0x1847, 2, 0xFF01, ANY, open),
    det(
        "ASUS ROG Strix Impact II",
        VID,
        0x18E1,
        0,
        0xFF01,
        ANY,
        open,
    ),
    det(
        "ASUS ROG Strix Impact II Gundam",
        VID,
        0x189E,
        0,
        0xFF01,
        ANY,
        open,
    ),
    det(
        "ASUS ROG Strix Impact II Electro Punk",
        VID,
        0x1956,
        0,
        0xFF01,
        ANY,
        open,
    ),
    det(
        "ASUS ROG Strix Impact II Moonlight White",
        VID,
        0x19D2,
        0,
        0xFF01,
        ANY,
        open,
    ),
    det(
        "ASUS ROG Strix Impact II Wireless USB",
        VID,
        0x1947,
        0,
        0xFF01,
        ANY,
        open,
    ),
    det(
        "ASUS ROG Strix Impact II Wireless 2.4 Ghz",
        VID,
        0x1949,
        0,
        0xFF01,
        ANY,
        open,
    ),
    det(
        "ASUS ROG Strix Impact III",
        VID,
        0x1A88,
        0,
        0xFF01,
        ANY,
        open,
    ),
    det("ASUS ROG Keris", VID, 0x195C, 0, 0xFF01, ANY, open),
    det(
        "ASUS ROG Keris Wireless USB",
        VID,
        0x195E,
        0,
        0xFF01,
        ANY,
        open,
    ),
    det(
        "ASUS ROG Keris Wireless 2.4Ghz",
        VID,
        0x1960,
        0,
        0xFF01,
        ANY,
        open,
    ),
    det(
        "ASUS ROG Keris Wireless Bluetooth",
        VID,
        0x1962,
        0,
        0xFF01,
        ANY,
        open,
    ),
    det(
        "ASUS ROG Keris Wireless AimPoint USB",
        VID,
        0x1A66,
        0,
        0xFF01,
        ANY,
        open,
    ),
    det(
        "ASUS ROG Keris Wireless AimPoint 2.4Ghz",
        VID,
        0x1A68,
        0,
        0xFF01,
        ANY,
        open,
    ),
    det("ASUS TUF Gaming M3", VID, 0x1910, 1, 0xFF01, ANY, open),
    det(
        "ASUS TUF Gaming M3 Gen II",
        VID,
        0x1A9B,
        2,
        0xFF01,
        ANY,
        open,
    ),
    det("ASUS TUF Gaming M5", VID, 0x1898, 2, 0xFF01, ANY, open),
    det(
        "ASUS TUF Gaming Mini WL Miku USB",
        VID,
        0x1C56,
        ANY,
        0xFF01,
        ANY,
        open,
    ),
    det(
        "ASUS TUF Gaming Mini WL Miku 2.4Ghz",
        VID,
        0x1C57,
        ANY,
        0xFF01,
        ANY,
        open,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // RGBController_AuraMouse::DeviceUpdateLEDs in Static: SendUpdate per zone
    #[test]
    fn static_zones() {
        let a = [Color(1, 2, 3), Color(4, 5, 6)];
        let [first, _, _] = pushes(AuraMouse(mouse(0x18DD).unwrap()), 2, vec![], &a, &[]);
        assert_eq!(
            first,
            vec![
                Op::Read,
                w(pad(&[0, 0x51, 0x28, 0, 0, 0, 4, 1, 2, 3], 65)),
                Op::Read,
                Op::Read,
                w(pad(&[0, 0x51, 0x28, 1, 0, 0, 4, 4, 5, 6], 65)),
                Op::Read,
            ]
        );
        let [first, _, _] = pushes(AuraMouse(mouse(PNK_LTD).unwrap()), 2, vec![], &a, &[]);
        assert_eq!(
            first[1],
            w(pad(&[0, 0x51, 0x28, 1, 0, 0, 4, 0, 1, 2, 3], 65))
        );
    }

    // AuraMouseController::SendDirect through the LED map
    #[test]
    fn direct_map() {
        let a = [Color(1, 1, 1), Color(2, 2, 2), Color(3, 3, 3)];
        let [first, _, _] = pushes(AuraMouse(mouse(0x1A18).unwrap()), 3, vec![], &a, &[]);
        let mut p0 = pad(&[0, 0x51, 0x29, 5, 0, 0], 65);
        p0[6..21].fill(3);
        let p1 = pad(
            &[0, 0x51, 0x29, 4, 0, 5, 3, 3, 3, 3, 3, 3, 1, 1, 1, 2, 2, 2],
            65,
        );
        assert_eq!(first, vec![w(p0), w(p1)]);
    }

    #[test]
    fn miku_logo_at_full_brightness() {
        let m = mouse(0x1C57).unwrap();
        assert_eq!(
            update(m.pid, m.zones[0], Color(7, 8, 9), m.brightness),
            pad(&[0, 0x51, 0x28, 0, 0, 0, 100, 7, 8, 9], 65)[..]
        );
    }

    #[test]
    fn every_detector_has_a_device() {
        assert!(DETECTORS.iter().all(|d| mouse(d.pid).is_some()));
        let direct = MICE.iter().filter(|m| m.map.is_some()).count();
        assert_eq!(direct, 9);
        assert_eq!((MICE.len(), DETECTORS.len()), (43, 43));
    }
}
