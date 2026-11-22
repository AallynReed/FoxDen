//! NZXT Function keyboards: ten lighting zones.

use super::layouts::NZXT_FULL_POS;
use super::{KEYBOARD, boxed, det, placed};
use crate::hid::{Io, Proto, drain, rgb};
use foxden_core::{Backend, Color, DeviceKind};
use foxden_hid::registry::{Detector, Info};
use hidapi::HidApi;

const FAMILY: &str = "nzxt-function";
const VID: u16 = 0x1E71;
const LEN: usize = 64;
const ZONES: usize = 10;
/// Firmware from this version takes all ten zones in one report.
const NEW_PROTOCOL: (u8, u8, u8) = (1, 3, 71);
const ISO: &[u16] = &[0x2107, 0x2108, 0x2131];
const MINI_TKL: &[u16] = &[0x2105, 0x2108];

const fn function(pid: u16, name: &'static str) -> Detector {
    det(name, VID, pid, Some(1), Some(0xFFCA), Some(0x01), open)
}

pub static DETECTORS: &[Detector] = &[
    function(0x2103, "NZXT Function"),
    function(0x2106, "NZXT Function"),
    function(0x2104, "NZXT Function TKL"),
    function(0x2107, "NZXT Function TKL"),
    function(0x2105, "NZXT Function MiniTKL"),
    function(0x2108, "NZXT Function MiniTKL"),
    function(0x2130, "NZXT Function 2"),
    function(0x2131, "NZXT Function 2"),
    function(0x2136, "NZXT Function 2"),
];

const SOFTWARE: &[&[u8]] = &[
    &[0x43, 0x81, 0x00, 0x84],
    &[0x43, 0x81, 0x00, 0x86],
    &[0x43, 0x82, 0x00, 0x41, 0x64],
    &[0x43, 0x97, 0x00, 0x10, 0x01],
];

/// `sendNewColors`: every key of every bank on, ten zones of R, G, B, 0.
pub fn new_report(colors: &[Color]) -> Vec<u8> {
    let mut b = vec![0x43, 0xBD, 0x01, 0x10, 0x02];
    b.extend([0xFF; 18]);
    b.extend([0x00, 0x0A]);
    for i in 0..ZONES {
        b.extend(rgb(colors.get(i).copied().unwrap_or(Color(0, 0, 0))));
        b.push(0);
    }
    b.truncate(LEN);
    b
}

fn packet(fixed: &[(usize, u8)], zones: &[(usize, usize)], colors: &[Color]) -> Vec<u8> {
    let mut b = vec![0u8; LEN];
    for &(i, v) in fixed {
        b[i] = v;
    }
    for &(at, zone) in zones {
        let c = colors.get(zone).copied().unwrap_or(Color(0, 0, 0));
        b[at..at + 3].copy_from_slice(&rgb(c));
    }
    b
}

/// `sendOldColors`: four reports that map key groups to zones, each zone colour followed by 0x01.
pub fn old_reports(colors: &[Color], iso: bool, mini: bool) -> [Vec<u8>; 4] {
    [
        packet(
            &[
                (0, 0x43),
                (1, 0xBD),
                (2, 0x03),
                (3, 0x10),
                (4, 0x01),
                (5, 0x01),
                (7, 0x03),
                (9, 0x03),
                (11, 0x03),
                (13, if iso { 0x03 } else { 0x01 }),
                (15, 0x03),
                (26, 0x01),
                (27, 0x06),
                (29, 0x0C),
                (31, 0x0C),
                (33, 0x0C),
                (35, 0x0C),
                (37, 0x04),
                (48, 0x01),
                (49, 0x60),
                (51, 0xC0),
                (53, 0xC0),
                (55, 0xC0),
                (57, 0xC0),
                (59, 0x20),
            ],
            &[(23, 0), (45, 1)],
            colors,
        ),
        packet(
            &[
                (0, 0x43),
                (1, 0x3D),
                (2, 0x02),
                (9, 0x01),
                (10, 0x80),
                (11, 0x01),
                (13, 0x03),
                (15, 0x03),
                (17, 0x03),
                (19, 0x03),
                (31, 0x01),
                (44, 0xC0),
                (46, 0x46),
                (47, 0x0E),
                (53, 0x01),
            ],
            &[(6, 2), (28, 3), (50, 4)],
            colors,
        ),
        packet(
            &[
                (0, 0x43),
                (1, 0x3D),
                (2, 0x01),
                (5, 0x32),
                (6, 0x04),
                (7, 0xB0),
                (8, 0x11),
                (14, 0x01),
                (16, 0x40),
                (24, 0x40),
                (26, 0x40),
                (27, 0x08),
                (28, 0x43),
                (29, 0x08),
                (30, 0x40),
                (36, 0x01),
                (38, if mini { 0xF0 } else { 0x30 }),
                (40, 0x40),
                (42, if mini { 0xE0 } else { 0x60 }),
                (44, if iso { 0x70 } else { 0x60 }),
                (46, 0x20),
                (48, 0x34),
                (58, 0x01),
                (60, 0x0E),
                (62, 0x1C),
            ],
            &[(11, 5), (33, 6), (55, 7)],
            colors,
        ),
        packet(
            &[
                (0, 0x43),
                (1, 0x26),
                (3, 0x1C),
                (5, 0x0C),
                (7, 0x0C),
                (9, 0x0A),
                (19, 0x01),
                (20, 0x18),
                (22, 0x30),
                (24, 0x30),
                (26, 0x30),
                (28, 0x30),
            ],
            &[(16, 8), (38, 9)],
            colors,
        ),
    ]
}

/// The firmware version from the second reply to 43 81 00 01 (bytes 3..6).
pub fn version(reply: &[u8]) -> Option<(u8, u8, u8)> {
    Some((*reply.get(3)?, *reply.get(4)?, *reply.get(5)?))
}

struct Function {
    iso: bool,
    mini: bool,
    new: bool,
}

impl Proto for Function {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        drain(io, LEN);
        io.write(&[0x43, 0x81, 0x00, 0x01], LEN)?;
        io.read(LEN, 500)?;
        self.new = version(&io.read(LEN, 500)?).is_some_and(|v| v >= NEW_PROTOCOL);
        for p in SOFTWARE {
            io.write(p, LEN)?;
        }
        drain(io, LEN);
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, c: &[Color]) -> Result<(), String> {
        if self.new {
            io.write(&new_report(c), LEN)?;
            io.write(&[0x43, 0x01], LEN)?;
        } else {
            for p in old_reports(c, self.iso, self.mini) {
                io.write(&p, LEN)?;
            }
        }
        drain(io, LEN);
        Ok(())
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = self.frame(io, &[Color(0, 0, 0); ZONES]);
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = placed(
        info,
        FAMILY,
        d.name,
        DeviceKind::Keyboard,
        KEYBOARD,
        [10, 5],
        NZXT_FULL_POS,
    );
    let proto = Function {
        iso: ISO.contains(&info.pid),
        mini: MINI_TKL.contains(&info.pid),
        new: false,
    };
    boxed(api, info, "nzxt function", dev, proto)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_report_layout() {
        let b = new_report(&[Color(1, 2, 3); ZONES]);
        assert_eq!(b.len(), LEN);
        assert_eq!(b[23..29], [0x00, 0x0A, 1, 2, 3, 0]);
    }

    #[test]
    fn old_reports_place_zones() {
        let c: Vec<Color> = (0..10).map(|i| Color(i, 0, 0)).collect();
        let r = old_reports(&c, true, true);
        assert_eq!(r[0][13], 0x03);
        assert_eq!(r[0][23..27], [0, 0, 0, 0x01]);
        assert_eq!(r[1][50], 4);
        assert_eq!(r[2][38], 0xF0);
        assert_eq!(r[2][44], 0x70);
        assert_eq!(r[3][38], 9);
        assert!(version(&[0, 0, 0, 1, 3, 71]).unwrap() >= NEW_PROTOCOL);
    }
}
