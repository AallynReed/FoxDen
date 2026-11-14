//! MSI Mystic Light, 185-byte generation, as used by the MS-7D99 boards.

use foxrgb_core::{Backend, Color, Device, DeviceKind};
use hidapi::HidDevice;

pub const VID: u16 = 0x1462;
pub const PID: u16 = 0x7D99;

const ID: &str = "hid:msi-7d99";
/// On-board LEDs, which come first in a direct frame (OpenRGB's 7D99 row).
const ONBOARD_LEDS: usize = 6;
/// LEDs per addressable header while all headers share one direct frame.
pub const HEADER_LEDS: usize = 40;
/// On-board, JRGB1, then JRAINBOW1 and JRAINBOW2.
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
pub fn direct_frame(colors: &[Color]) -> [u8; 725] {
    let mut p = [0u8; 725];
    p[..5].copy_from_slice(&[0x53, 0x25, 0x06, 0x00, 0x00]);
    for (i, c) in colors.iter().take(LED_COUNT).enumerate() {
        let at = 5 + i * 3;
        p[at..at + 3].copy_from_slice(&[c.0, c.1, c.2]);
    }
    p
}

fn device() -> Device {
    let mut leds: Vec<[f32; 2]> = (0..ONBOARD_LEDS)
        .map(|i| [(i as f32 + 0.5) / ONBOARD_LEDS as f32, 0.1])
        .collect();
    leds.push([0.5, 0.3]);
    for row in [0.6, 0.9] {
        leds.extend((0..HEADER_LEDS).map(|i| [(i as f32 + 0.5) / HEADER_LEDS as f32, row]));
    }
    Device {
        id: ID.into(),
        name: "MSI B760M Gaming Plus WiFi".into(),
        kind: DeviceKind::Motherboard,
        rect: [0.06, 0.10, 0.52, 0.15],
        grid: [HEADER_LEDS as u32, 4],
        leds,
    }
}

pub struct Board {
    hid: HidDevice,
    /// Last frame on the board, `None` until direct mode is on.
    sent: Option<Vec<Color>>,
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
        Ok(Board { hid, sent: None })
    }
}

impl Backend for Board {
    fn name(&self) -> &str {
        "msi mystic light"
    }

    fn devices(&self) -> Vec<Device> {
        vec![device()]
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
                .send_feature_report(&enable_direct())
                .map_err(|e| e.to_string())?;
        }
        match self.hid.send_feature_report(&direct_frame(colors)) {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    // Offsets from FeaturePacket_185 in OpenRGB's MSIMotherboardCommon.h, values from
    // enable_per_led_msg in MSIMotherboard185Controller.cpp.
    #[test]
    fn enable_direct_matches_openrgb_185_controller() {
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

    // FeaturePacket_PerLED_185 and the slot order set up in MSIMotherboard185Controller's constructor.
    #[test]
    fn direct_frame_matches_openrgb_per_led_packet() {
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

    // DetectMSIMotherboardControllers in MSIMotherboardControllerDetect.cpp picks the generation this way.
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
        let d = device();
        assert_eq!(d.leds.len(), LED_COUNT);
        assert!(d.leds.iter().flatten().all(|v| (0.0..=1.0).contains(v)));
    }
}
