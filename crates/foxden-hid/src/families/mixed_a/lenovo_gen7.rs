// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

use super::common::{Driver, Hid, Io, KEYBOARD, matrix};
use super::lenovo::led_values;
use super::lenovo_zones::*;
use crate::registry::{Detector, Info};

pub const S7GEN7: u16 = 0xC977;
pub const GEN7: u16 = 0xC978;
pub const GEN8: u16 = 0xC988;
pub const S7GEN8: u16 = 0xC987;
pub const GEN9: u16 = 0xC997;
pub const GEN9_H: u16 = 0xC998;
pub const GEN10: u16 = 0xC197;
pub const L5GEN10: u16 = 0xC195;

const LEN: usize = 960;
const MODE_STATIC: u8 = 0x0B;
const MODE_DIRECT: u8 = 0x0D;
/// MODE_DIRECTION_LEFT, the default direction, which GetLedGroups turns into 0x03.
const DIRECTION_LEFT: u8 = 0x03;

/// The 24-zone keyboard of the Legion 5 Gen 10.
#[rustfmt::skip]
const KBD_24ZONE: Zone = Zone {
    id: 0,
    height: 4,
    width: 6,
    map: Some(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23]),
    leds: &[
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C,
        0x0D, 0x0E, 0x0F, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18,
    ],
};

fn gen10(pid: u16) -> bool {
    pid == GEN10 || pid == L5GEN10
}

/// Keyboard-only parts have no Direct mode; they are driven per LED through Static.
fn keyboard_only(pid: u16) -> bool {
    pid == L5GEN10
}

fn header(pid: u16, cmd: u8, payload: u16) -> [u8; LEN] {
    let mut b = [0u8; LEN];
    b[..4].copy_from_slice(&[0x07, cmd, 0xC0, 0x03]);
    if gen10(pid) {
        b[2..4].copy_from_slice(&payload.to_le_bytes());
    }
    b
}

/// getCurrentProfileId's request.
pub fn profile_request(pid: u16) -> [u8; LEN] {
    header(pid, 0xCA, 1)
}

/// setLedsDirectOn.
pub fn direct_on(pid: u16, profile: u8) -> [u8; LEN] {
    let mut b = header(pid, 0xD0, 2);
    b[4] = 0x01;
    b[5] = profile;
    b
}

pub struct Group {
    pub mode: u8,
    pub direction: u8,
    pub colors: Vec<Color>,
    pub leds: Vec<u16>,
}

/// setLedsByGroup (speed and spin 0, colour mode 2).
pub fn by_group(pid: u16, profile: u8, groups: &[Group]) -> Vec<[u8; LEN]> {
    let mut out = Vec::new();
    let mut g = 0;
    while g < groups.len() {
        let mut b = [0u8; LEN];
        b[..7].copy_from_slice(&[0x07, 0xCB, 0xC0, 0x03, profile, 0x01, 0x01]);
        let mut i = 7;
        while g < groups.len() && i < LEN - 21 {
            let gr = &groups[g];
            let head = [
                g as u8 + 1,
                0x06,
                0x01,
                gr.mode,
                0x02,
                0x00,
                0x03,
                0x00,
                0x04,
                gr.direction,
                0x05,
                0x02,
                0x06,
                0x00,
                gr.colors.len() as u8,
            ];
            b[i..i + 15].copy_from_slice(&head);
            i += 15;
            for c in &gr.colors {
                b[i..i + 3].copy_from_slice(&[c.0, c.1, c.2]);
                i += 3;
            }
            let n = gr.leds.len().min((LEN - i - 1) / 2);
            b[i] = n as u8;
            i += 1;
            for l in &gr.leds[..n] {
                b[i..i + 2].copy_from_slice(&l.to_le_bytes());
                i += 2;
            }
            g += 1;
        }
        if gen10(pid) {
            b[2..4].copy_from_slice(&((i - 4) as u16).to_le_bytes());
        } else {
            b[2] = i as u8;
        }
        out.push(b);
    }
    out
}

/// setLedsDirect.
pub fn direct(pid: u16, values: &[u16], colors: &[Color]) -> [u8; LEN] {
    let mut b = header(pid, 0xA1, 0);
    let mut i = 4;
    let mut count = 0u16;
    for (v, c) in values.iter().zip(colors) {
        if i + 5 > LEN {
            break;
        }
        let [lo, hi] = v.to_le_bytes();
        b[i..i + 5].copy_from_slice(&[lo, hi, c.0, c.1, c.2]);
        i += 5;
        count += 1;
    }
    if gen10(pid) {
        b[2..4].copy_from_slice(&(count * 5).to_le_bytes());
    }
    b
}

/// GetLedGroups for Static with per-LED colours: one group per colour, first seen first, black dropped unless alone.
pub fn static_groups(values: &[u16], colors: &[Color]) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    for (v, c) in values.iter().zip(colors) {
        match groups.iter_mut().find(|g| g.colors[0] == *c) {
            Some(g) => g.leds.push(*v),
            None => groups.push(Group {
                mode: MODE_STATIC,
                direction: DIRECTION_LEFT,
                colors: vec![*c],
                leds: vec![*v],
            }),
        }
    }
    if groups.len() > 1 {
        groups.retain(|g| g.colors[0] != Color::BLACK);
    }
    groups
}

/// RGBController_Lenovo_Gen7_8::SetupZones.
pub fn zones(pid: u16) -> Vec<&'static Zone> {
    let mut z: Vec<&'static Zone> = if keyboard_only(pid) {
        vec![&KBD_24ZONE]
    } else {
        vec![&LEGION7_GEN7AND8_KBD_ANSI, &LEGION7_GEN7AND8_NEON]
    };
    match pid {
        GEN7 => z.extend([&LENOVO_LEGION_7GEN7_LOGO, &LENOVO_LEGION_7GEN7_VENTS]),
        GEN10 => z.extend([&LENOVO_LEGION_7GEN7_LOGO, &LENOVO_LEGION_7GEN10_VENTS]),
        _ => {}
    }
    z
}

struct Gen7 {
    pid: u16,
    values: Vec<u16>,
    profile: u8,
}

impl Driver for Gen7 {
    fn start(&mut self, io: &mut dyn Io, _: &[Color]) -> Result<(), String> {
        io.send_feature(&profile_request(self.pid))?;
        let mut r = [0u8; LEN];
        r[0] = 0x07;
        let n = io.get_feature(&mut r).unwrap_or(0);
        self.profile = if n > 4 { r[4] } else { 0x01 };
        if keyboard_only(self.pid) {
            return Ok(());
        }
        io.send_feature(&direct_on(self.pid, self.profile))?;
        if !gen10(self.pid) {
            let all = Group {
                mode: MODE_DIRECT,
                direction: DIRECTION_LEFT,
                colors: vec![Color::BLACK],
                leds: self.values.clone(),
            };
            for p in by_group(self.pid, self.profile, &[all]) {
                io.send_feature(&p)?;
            }
        }
        Ok(())
    }

    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        if keyboard_only(self.pid) {
            for p in by_group(self.pid, self.profile, &static_groups(&self.values, colors)) {
                io.send_feature(&p)?;
            }
            return Ok(());
        }
        if gen10(self.pid) {
            io.send_feature(&direct_on(self.pid, self.profile))?;
        }
        io.send_feature(&direct(self.pid, &self.values, colors))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let z = zones(info.pid);
    let values = led_values(&z);
    let kbd = z[0];
    let dev = matrix(
        info.device_id("lenovo"),
        d.name,
        DeviceKind::Keyboard,
        KEYBOARD,
        (kbd.height, kbd.width),
        kbd.map.unwrap_or(&[]),
        values.len(),
    );
    let drv = Gen7 {
        pid: info.pid,
        values,
        profile: 1,
    };
    Ok(Hid::boxed("lenovo legion gen 7", info.open(api)?, drv, dev))
}

macro_rules! pu {
    ($name:expr, $pid:expr) => {
        Detector {
            name: $name,
            vid: 0x048D,
            pid: $pid,
            interface: None,
            usage_page: Some(0xFF89),
            usage: Some(0x07),
            open,
        }
    };
}

pub static DETECTORS: [Detector; 8] = [
    pu!("Lenovo Legion 7S Gen 7", S7GEN7),
    pu!("Lenovo Legion 7 Gen 7", GEN7),
    pu!("Lenovo Legion 7 Gen 8", GEN8),
    pu!("Lenovo Legion 7S Gen 8", S7GEN8),
    pu!("Lenovo Legion 7 Gen 9", GEN9),
    pu!("Lenovo Legion 7 Gen 9", GEN9_H),
    pu!("Lenovo Legion 7 Gen 10", GEN10),
    pu!("Lenovo Legion 5 Gen 10", L5GEN10),
];

#[cfg(test)]
mod tests {
    use super::super::common::mock::{Mock, Op, pad};
    use super::*;

    const C1: Color = Color(0x11, 0x22, 0x33);

    // getCurrentProfileId, setLedsDirectOn and SetGen10PayloadLength.
    #[test]
    fn requests_and_gen10_lengths() {
        assert_eq!(profile_request(GEN8)[..5], [0x07, 0xCA, 0xC0, 0x03, 0x00]);
        assert_eq!(profile_request(GEN10)[..5], [0x07, 0xCA, 0x01, 0x00, 0x00]);
        assert_eq!(
            direct_on(GEN8, 2)[..7],
            [0x07, 0xD0, 0xC0, 0x03, 0x01, 0x02, 0x00]
        );
        assert_eq!(
            direct_on(GEN10, 2)[..7],
            [0x07, 0xD0, 0x02, 0x00, 0x01, 0x02, 0x00]
        );
    }

    // setLedsDirect, both packet formats.
    #[test]
    fn direct_frames() {
        let d = direct(GEN8, &[0x0001, 0x0105], &[C1, Color(4, 5, 6)]);
        assert_eq!(
            d[..14],
            [
                0x07, 0xA1, 0xC0, 0x03, 0x01, 0x00, 0x11, 0x22, 0x33, 0x05, 0x01, 4, 5, 6
            ]
        );
        assert!(d[14..].iter().all(|&b| b == 0));
        let g = direct(GEN10, &[0x0001, 0x0105], &[C1, Color(4, 5, 6)]);
        assert_eq!(g[..6], [0x07, 0xA1, 10, 0, 0x01, 0x00]);
    }

    // setLedsByGroup with the single black group the switch to Direct sends; length byte truncates.
    #[test]
    fn group_report() {
        let leds: Vec<u16> = (0..120).map(|i| i as u16).collect();
        let all = Group {
            mode: MODE_DIRECT,
            direction: 3,
            colors: vec![Color::BLACK],
            leds,
        };
        let p = by_group(GEN8, 1, &[all]);
        assert_eq!(p.len(), 1);
        let b = &p[0];
        assert_eq!(
            b[..26],
            [
                0x07, 0xCB, 0x0A, 0x03, 0x01, 0x01, 0x01, 0x01, 0x06, 0x01, 0x0D, 0x02, 0x00, 0x03,
                0x00, 0x04, 0x03, 0x05, 0x02, 0x06, 0x00, 0x01, 0x00, 0x00, 0x00, 120
            ]
        );
        // 7 + 15 + 3 + 1 + 240 = 266 bytes, written as 266 & 0xFF
        assert_eq!(b[26..30], [0x00, 0x00, 0x01, 0x00]);
        assert_eq!(b[264..266], [119, 0]);
        let g10 = by_group(
            GEN10,
            1,
            &[Group {
                mode: 0x0B,
                direction: 3,
                colors: vec![C1],
                leds: vec![0x0101],
            }],
        );
        assert_eq!(g10[0][2..4], [7 + 15 + 3 + 1 + 2 - 4, 0]);
    }

    // GetLedGroups with per-LED colours: black is dropped when other colours exist.
    #[test]
    fn static_groups_by_colour() {
        let g = static_groups(&[1, 2, 3, 4], &[C1, Color::BLACK, Color(1, 1, 1), C1]);
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].leds, vec![1, 4]);
        assert_eq!(g[1].colors, vec![Color(1, 1, 1)]);
        assert_eq!(static_groups(&[1, 2], &[Color::BLACK; 2]).len(), 1);
    }

    // RGBController_Lenovo_Gen7_8::DeviceUpdateMode into Direct, then DeviceUpdateLEDs.
    #[test]
    fn switch_to_direct_then_frames() {
        let m = Mock::default();
        m.reply(pad(&[0x07, 0xCA, 0xC0, 0x03, 0x02], 8));
        let z = zones(GEN8);
        let values = led_values(&z);
        assert_eq!(values.len(), 111);
        let dev = matrix(
            "g".into(),
            "G",
            DeviceKind::Keyboard,
            KEYBOARD,
            (7, 20),
            z[0].map.unwrap(),
            111,
        );
        let drv = Gen7 {
            pid: GEN8,
            values: values.clone(),
            profile: 1,
        };
        let mut b = Hid::new("t", Box::new(m.clone()), drv, dev);
        b.push("g", &[C1; 111]).unwrap();
        let ops = m.take();
        assert_eq!(ops[0], Op::Feature(profile_request(GEN8).to_vec()));
        assert_eq!(ops[1], Op::GetFeature(pad(&[0x07], LEN)));
        assert_eq!(ops[2], Op::Feature(direct_on(GEN8, 2).to_vec()));
        let Op::Feature(group) = &ops[3] else {
            panic!()
        };
        assert_eq!(
            group[..5],
            [0x07, 0xCB, (7 + 15 + 3 + 1 + 222) as u8, 0x03, 0x02]
        );
        assert_eq!(
            ops[4],
            Op::Feature(direct(GEN8, &values, &[C1; 111]).to_vec())
        );
        assert_eq!(values[101], 0x0100 | z[1].leds[0] as u16);
    }

    // Gen 10 re-sends setLedsDirectOn before every frame.
    #[test]
    fn gen10_frames_reenable_direct() {
        let m = Mock::default();
        let values = led_values(&zones(GEN10));
        let n = values.len();
        let dev = matrix(
            "g".into(),
            "G",
            DeviceKind::Keyboard,
            KEYBOARD,
            (7, 20),
            &[],
            n,
        );
        let drv = Gen7 {
            pid: GEN10,
            values,
            profile: 1,
        };
        let mut b = Hid::new("t", Box::new(m.clone()), drv, dev);
        b.push("g", &vec![C1; n]).unwrap();
        let ops = m.take();
        assert_eq!(ops.len(), 5);
        // an unanswered profile query reads profile 0
        assert_eq!(ops[2], Op::Feature(direct_on(GEN10, 0).to_vec()));
        assert_eq!(ops[3], Op::Feature(direct_on(GEN10, 0).to_vec()));
    }

    // getCurrentProfileId falls back to profile 1 when the read fails.
    #[test]
    fn failed_profile_read_uses_profile_1() {
        let m = Mock::default();
        m.reply(Vec::new());
        let values = led_values(&zones(GEN8));
        let n = values.len();
        let dev = matrix(
            "g".into(),
            "G",
            DeviceKind::Keyboard,
            KEYBOARD,
            (7, 20),
            &[],
            n,
        );
        let drv = Gen7 {
            pid: GEN8,
            values,
            profile: 0,
        };
        let mut b = Hid::new("t", Box::new(m.clone()), drv, dev);
        b.push("g", &vec![C1; n]).unwrap();
        assert_eq!(m.take()[2], Op::Feature(direct_on(GEN8, 1).to_vec()));
    }
}
