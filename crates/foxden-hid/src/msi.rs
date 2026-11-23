//! MSI Mystic Light, 185-byte generation, as used by the MS-7D99 boards.

use crate::families::{D99, Op};
use foxden_core::{Backend, Color, Device, DeviceKind, Hardware, HardwareSetting};
use hidapi::HidDevice;

pub const VID: u16 = 0x1462;
pub const PID: u16 = 0x7D99;

const ID: &str = "hid:msi-7d99";
/// On-board LEDs, which come first in a direct frame.
const ONBOARD_LEDS: usize = 6;
/// LEDs per addressable header by default, while all headers share one direct frame.
pub const HEADER_LEDS: usize = 40;
/// On-board, JRGB1, then JRAINBOW1 and JRAINBOW2.
#[cfg(test)]
pub const LED_COUNT: usize = ONBOARD_LEDS + 1 + 2 * HEADER_LEDS;

/// Settings report that hands every zone to per-LED control. Byte 184 is the save flag and stays 0.
pub fn enable_direct() -> [u8; 185] {
    let mut p = [0u8; 185];
    p[0] = 0x52;
    // (offset, speed and brightness flags) of JRGB1, JPIPE1, JPIPE2, JRAINBOW1, JRAINBOW2, outer fan ring, on-board
    let mut zones = vec![
        (1, 0x08),
        (11, 0x2A),
        (21, 0x2A),
        (31, 0x29),
        (42, 0x29),
        (64, 0x28),
        (74, 0xA9),
    ];
    zones.extend((84..=164).step_by(10).map(|at| (at, 0x28)));
    zones.push((174, 0x2A));
    for (at, flags) in zones {
        p[at] = 0x01;
        p[at + 4] = flags;
        p[at + 8] = 0x80;
    }
    p[41] = HEADER_LEDS as u8;
    p[52] = HEADER_LEDS as u8;
    // JCORSAIR has its own record layout
    p[53] = 0x01;
    p[57] = 0x29;
    p[61] = 0x82;
    p[63] = 120;
    // the on-board record carries the direct effect and syncs every header to it
    p[74] = 0x25;
    p[82] = 0xBF;
    p
}

/// One frame for all zones: 240 RGB slots.
#[cfg(test)]
pub fn direct_frame(colors: &[Color]) -> [u8; 725] {
    let mut p = [0u8; 725];
    p[..5].copy_from_slice(&[0x53, 0x25, 0x06, 0x00, 0x00]);
    for (i, c) in colors.iter().take(LED_COUNT).enumerate() {
        let at = 5 + i * 3;
        p[at..at + 3].copy_from_slice(&[c.0, c.1, c.2]);
    }
    p
}

/// `headers`: JRAINBOW1 and JRAINBOW2 lengths.
fn device(headers: [usize; 2]) -> Device {
    let mut leds: Vec<[f32; 2]> = (0..ONBOARD_LEDS)
        .map(|i| [(i as f32 + 0.5) / ONBOARD_LEDS as f32, 0.1])
        .collect();
    leds.push([0.5, 0.3]);
    for (n, row) in headers.into_iter().zip([0.6, 0.9]) {
        leds.extend((0..n).map(|i| [(i as f32 + 0.5) / n as f32, row]));
    }
    let wide = headers[0].max(headers[1]).max(ONBOARD_LEDS);
    Device {
        id: ID.into(),
        name: "MSI B760M Gaming Plus WiFi".into(),
        kind: DeviceKind::Motherboard,
        rect: [0.06, 0.10, 0.52, 0.15],
        grid: [wide as u32, 4],
        leds,
    }
}

fn headers(d99: &D99) -> [usize; 2] {
    let n = d99.counts();
    [n[2], n[3]]
}

pub struct Board {
    hid: HidDevice,
    headers: D99,
    /// Last frame on the board, `None` until direct mode is on.
    sent: Option<Vec<Color>>,
    /// The settings report as read at open.
    settings: [u8; 185],
    /// The last effect report, until a push takes direct mode back.
    last_effect: Option<[u8; 185]>,
}

/// Length of settings report 0x52 on this generation; some platforms count one byte more.
fn is_185(len: usize) -> bool {
    matches!(len, 185 | 186)
}

impl Board {
    /// Reads the settings report and refuses a board that is not the 185-byte generation.
    pub fn open(hid: HidDevice) -> Result<Self, String> {
        let mut settings = [0u8; 200];
        settings[0] = 0x52;
        let len = hid
            .get_feature_report(&mut settings)
            .map_err(|e| e.to_string())?;
        if !is_185(len) {
            return Err(format!("settings report is {len} bytes, expected 185"));
        }
        let mut stored = [0u8; 185];
        stored.copy_from_slice(&settings[..185]);
        stored[184] = 0;
        Ok(Board {
            hid,
            headers: D99::open(ID),
            sent: None,
            settings: stored,
            last_effect: None,
        })
    }
}

impl Backend for Board {
    fn name(&self) -> &str {
        "msi mystic light"
    }

    fn devices(&self) -> Vec<Device> {
        vec![device(headers(&self.headers))]
    }

    fn push(&mut self, device_id: &str, colors: &[Color]) -> Result<(), String> {
        if device_id != ID {
            return Err(format!("unknown device {device_id}"));
        }
        if self.sent.as_deref() == Some(colors) {
            return Ok(());
        }
        if self.sent.is_none() {
            self.hid
                .send_feature_report(&self.headers.enable())
                .map_err(|e| e.to_string())?;
            self.last_effect = None;
        }
        match send(&self.hid, self.headers.frames(colors)) {
            Ok(()) => {
                self.sent = Some(colors.to_vec());
                Ok(())
            }
            Err(e) => {
                self.sent = None;
                Err(e.to_string())
            }
        }
    }

    fn hardware(&self, device_id: &str) -> Option<Hardware> {
        (device_id == ID).then(crate::families::hardware_185)
    }

    /// The state on the board, then the effect; the next push turns direct mode on again.
    fn set_hardware(
        &mut self,
        device_id: &str,
        setting: Option<&HardwareSetting>,
    ) -> Result<(), String> {
        if device_id != ID {
            return Err(format!("unknown device {device_id}"));
        }
        let ops = match setting {
            Some(s) => {
                let now = match self.last_effect {
                    Some(last) => last,
                    None if self.sent.is_some() => self.headers.enable(),
                    None => self.settings,
                };
                let (ops, new) = effect_ops(&self.headers, now, &self.settings, s)?;
                self.last_effect = Some(new);
                ops
            }
            None => Vec::new(),
        };
        self.sent = None;
        send(&self.hid, ops)
    }
}

/// `now`, the state on the board, then the effect; also returns the effect report.
fn effect_ops(
    d99: &D99,
    now: [u8; 185],
    settings: &[u8; 185],
    s: &HardwareSetting,
) -> Result<(Vec<Op>, [u8; 185]), String> {
    let new = d99.effect(settings, s)?;
    Ok((
        vec![Op::Feature(now.to_vec()), Op::Feature(new.to_vec())],
        new,
    ))
}

fn send(hid: &HidDevice, ops: Vec<Op>) -> Result<(), String> {
    for op in ops {
        match op {
            Op::Feature(p) => hid.send_feature_report(&p).map_err(|e| e.to_string())?,
            Op::Sleep(ms) => std::thread::sleep(std::time::Duration::from_millis(ms)),
            Op::Write(p) => {
                hid.write(&p).map_err(|e| e.to_string())?;
            }
            Op::Read(..) => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enable_direct_packet() {
        let p = enable_direct();
        let expect = [
            (0, 0x52),
            (1, 0x01),
            (5, 0x08),
            (9, 0x80),
            (15, 0x2A),
            (19, 0x80),
            (25, 0x2A),
            (29, 0x80),
            (31, 0x01),
            (35, 0x29),
            (39, 0x80),
            (41, 0x28),
            (42, 0x01),
            (46, 0x29),
            (50, 0x80),
            (52, 0x28),
            (53, 0x01),
            (57, 0x29),
            (58, 0x00),
            (61, 0x82),
            (63, 0x78),
            (68, 0x28),
            (72, 0x80),
            (74, 0x25),
            (78, 0xA9),
            (82, 0xBF),
            (84, 0x01),
            (88, 0x28),
            (92, 0x80),
            (164, 0x01),
            (168, 0x28),
            (172, 0x80),
            (174, 0x01),
            (178, 0x2A),
            (182, 0x80),
        ];
        for (at, value) in expect {
            assert_eq!(p[at], value, "byte {at}");
        }
        assert_eq!(p[184], 0, "save flag must stay clear");
        // colours are zero everywhere
        assert_eq!(p[2..5], [0, 0, 0]);
        assert_eq!(p[75..78], [0, 0, 0]);
    }

    #[test]
    fn per_led_frame() {
        let mut colors = vec![Color::BLACK; LED_COUNT];
        colors[0] = Color(9, 9, 9);
        colors[6] = Color(1, 2, 3);
        colors[7] = Color(4, 5, 6);
        colors[46] = Color(7, 8, 9);
        colors[47] = Color(10, 11, 12);
        colors[86] = Color(13, 14, 15);
        let p = direct_frame(&colors);
        assert_eq!(p.len(), 5 + 240 * 3);
        assert_eq!(p[..5], [0x53, 0x25, 0x06, 0x00, 0x00]);
        let slot = |n: usize| [p[5 + n * 3], p[6 + n * 3], p[7 + n * 3]];
        assert_eq!(slot(0), [9, 9, 9]);
        assert_eq!(slot(6), [1, 2, 3]);
        assert_eq!(slot(7), [4, 5, 6]);
        assert_eq!(slot(46), [7, 8, 9]);
        assert_eq!(slot(47), [10, 11, 12]);
        assert_eq!(slot(86), [13, 14, 15]);
        assert!(p[5 + 87 * 3..].iter().all(|&b| b == 0));
    }

    // Detection picks the generation this way.
    #[test]
    fn only_the_185_byte_generation_is_accepted() {
        assert!(is_185(185));
        assert!(is_185(186));
        for other in [0, 112, 113, 162, 163, 184, 187, 200] {
            assert!(!is_185(other), "{other}");
        }
    }

    #[test]
    fn direct_frame_ignores_extra_colours() {
        let p = direct_frame(&[Color(255, 255, 255); 300]);
        assert!(p[5 + 87 * 3..].iter().all(|&b| b == 0));
    }

    #[test]
    fn device_has_one_led_per_slot() {
        let d = device([HEADER_LEDS; 2]);
        assert_eq!(d.leds.len(), LED_COUNT);
        assert_eq!(d.grid, [40, 4]);
        assert!(d.leds.iter().flatten().all(|v| (0.0..=1.0).contains(v)));
    }

    fn features(ops: &[Op]) -> Vec<&[u8]> {
        ops.iter()
            .filter_map(|op| match op {
                Op::Feature(p) => Some(&p[..]),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn default_headers_send_the_same_bytes() {
        let d99 = D99::open("hid:msi-7d99-default");
        assert_eq!(d99.counts(), [ONBOARD_LEDS, 1, HEADER_LEDS, HEADER_LEDS]);
        assert_eq!(d99.enable(), enable_direct());
        let colors: Vec<Color> = (0..LED_COUNT)
            .map(|i| Color(i as u8 + 1, (3 * i) as u8, 255 - i as u8))
            .collect();
        let ops = d99.frames(&colors);
        assert_eq!(features(&ops), [&direct_frame(&colors)[..]]);
    }

    #[test]
    fn a_long_header_uses_the_per_header_mode() {
        crate::lengths::set_one("hid:msi-7d99-long", 1, 100);
        let d99 = D99::open("hid:msi-7d99-long");
        assert_eq!(headers(&d99), [100, HEADER_LEDS]);
        let d = device(headers(&d99));
        assert_eq!((d.leds.len(), d.grid), (ONBOARD_LEDS + 1 + 140, [100, 4]));
        let p = d99.enable();
        assert_eq!(
            (p[31], p[41], p[42], p[52], p[82]),
            (0x25, 200, 0x25, 240, 0xB1)
        );
        let n = ONBOARD_LEDS + 1 + 140;
        let colors: Vec<Color> = (0..n).map(|i| Color(i as u8, 0, 1)).collect();
        let ops = d99.frames(&colors);
        let sent = features(&ops);
        assert_eq!(sent.len(), 3);
        assert_eq!(sent[0][..5], [0x53, 0x25, 0x04, 0x00, 0x00]);
        assert_eq!(sent[0][5..8], [7, 0, 1]);
        assert_eq!(sent[0][5 + 99 * 3..5 + 100 * 3], [106, 0, 1]);
        assert_eq!(sent[1][..5], [0x53, 0x25, 0x04, 0x01, 0x00]);
        assert_eq!(sent[1][5..8], [107, 0, 1]);
        assert_eq!(sent[2][..5], [0x53, 0x25, 0x06, 0x00, 0x00]);
        assert_eq!(sent[2][5..8], [0, 0, 1]);
        assert_eq!(sent[2][5 + 6 * 3..5 + 7 * 3], [6, 0, 1]);
        assert!(ops.contains(&Op::Sleep(13)));
        crate::lengths::set_one("hid:msi-7d99-max", 2, 999);
        assert_eq!(headers(&D99::open("hid:msi-7d99-max")), [HEADER_LEDS, 240]);
    }

    #[test]
    fn effect_on_the_7d99() {
        let d99 = D99::open("hid:msi-7d99-effect");
        let mut settings = enable_direct();
        settings[184] = 0;
        let mut s = HardwareSetting::new("lightning");
        s.speed = Some(2);
        s.brightness = Some(5);
        s.colors = vec![Color(7, 8, 9)];
        let (ops, new) = effect_ops(&d99, enable_direct(), &settings, &s).unwrap();
        let sent = features(&ops);
        assert_eq!(sent[0], &enable_direct()[..]);
        let p = sent[1];
        assert_eq!(p[1..11], [5, 7, 8, 9, 0x16, 7, 8, 9, 0x80, 0]);
        assert_eq!(p[31..41], [5, 7, 8, 9, 0x16, 7, 8, 9, 0x80, 0]);
        assert_eq!(p[42..52], [5, 7, 8, 9, 0x16, 7, 8, 9, 0x80, 0]);
        assert_eq!(p[74..84], [5, 7, 8, 9, 0x16, 7, 8, 9, 0x8F, 0]);
        assert_eq!(p[64..74], [5, 7, 8, 9, 0x16, 7, 8, 9, 0x80, 0]);
        assert_eq!(p[134..144], settings[134..144]);
        assert_eq!(p[184], 0);
        assert_eq!(p, &new[..]);
        let (again, _) = effect_ops(&d99, new, &settings, &s).unwrap();
        assert_eq!(features(&again)[0], &new[..]);
    }
}
