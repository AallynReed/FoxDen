// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Backend, Color, Device, DeviceKind};
use hidapi::{HidApi, HidDevice};

use crate::registry::{Detector, Info};

mod jpeg;

const COLS: u32 = 5;
const ROWS: u32 = 3;
const KEYS: usize = (COLS * ROWS) as usize;
const KEY_PX: u16 = 72;
const PACKET: usize = 1024;
const HEADER: usize = 8;

/// `ElgatoStreamDeckController::SendButtonImage`: one 1024-byte report, the JPEG cut at 1016 bytes.
fn image_report(key: u8, jpeg: &[u8]) -> Vec<u8> {
    let mut b = vec![0u8; PACKET];
    let len = jpeg.len().to_le_bytes();
    b[..HEADER].copy_from_slice(&[0x02, 0x07, key, 0x01, len[0], len[1], 0, 0]);
    let n = jpeg.len().min(PACKET - HEADER);
    b[HEADER..HEADER + n].copy_from_slice(&jpeg[..n]);
    b
}

/// `RGBController_ElgatoStreamDeck::CreateButtonImage`.
fn key_image(c: Color) -> Vec<u8> {
    jpeg::solid(KEY_PX, KEY_PX, c.0, c.1, c.2)
}

/// Keys whose colour differs from what was last sent.
fn changed(sent: &[Option<Color>], colors: &[Color]) -> Vec<(usize, Color)> {
    (0..KEYS)
        .map(|i| (i, colors.get(i).copied().unwrap_or_default()))
        .filter(|&(i, c)| sent[i] != Some(c))
        .collect()
}

struct Deck {
    hid: HidDevice,
    device: Device,
    sent: [Option<Color>; KEYS],
}

impl Backend for Deck {
    fn name(&self) -> &str {
        "stream deck"
    }

    fn devices(&self) -> Vec<Device> {
        vec![self.device.clone()]
    }

    fn push(&mut self, device_id: &str, colors: &[Color]) -> Result<(), String> {
        if device_id != self.device.id {
            return Err(format!("unknown device {device_id}"));
        }
        for (i, c) in changed(&self.sent, colors) {
            self.sent[i] = None;
            self.hid
                .write(&image_report(i as u8, &key_image(c)))
                .map_err(|e| e.to_string())?;
            self.sent[i] = Some(c);
        }
        Ok(())
    }
}

fn layout(info: &Info, name: &str) -> Device {
    Device::grid(
        &info.device_id("streamdeck"),
        name,
        DeviceKind::Other,
        [0.34, 0.12, 0.20, 0.12],
        COLS,
        ROWS,
    )
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let hid = info.open(api)?;
    Ok(Box::new(Deck {
        hid,
        device: layout(info, d.name),
        sent: [None; KEYS],
    }))
}

/// `REGISTER_HID_DETECTOR`, with the detect function's `interface_number == 0` check.
static DETECTORS: [Detector; 1] = [Detector {
    name: "Elgato Stream Deck MK.2",
    vid: 0x0FD9,
    pid: 0x0080,
    interface: Some(0),
    usage_page: None,
    usage: None,
    open,
}];

pub fn detectors() -> Vec<&'static Detector> {
    DETECTORS.iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_detector_for_the_mk2() {
        let all = detectors();
        assert_eq!(all.len(), 1);
        let d = all[0];
        assert_eq!(
            (d.name, d.vid, d.pid, d.interface),
            ("Elgato Stream Deck MK.2", 0x0FD9, 0x0080, Some(0))
        );
    }

    #[test]
    fn image_report_header_and_payload() {
        let jpeg: Vec<u8> = (0..300u16).map(|i| i as u8).collect();
        let b = image_report(14, &jpeg);
        assert_eq!(b.len(), 1024);
        assert_eq!(&b[..8], &[0x02, 0x07, 14, 0x01, 0x2C, 0x01, 0x00, 0x00]);
        assert_eq!(&b[8..308], &jpeg[..]);
        assert!(b[308..].iter().all(|&x| x == 0));
    }

    #[test]
    fn image_report_cuts_long_images() {
        let jpeg = vec![0xAB; 1100];
        let b = image_report(0, &jpeg);
        assert_eq!(b.len(), 1024);
        assert_eq!(&b[..8], &[0x02, 0x07, 0, 0x01, 0x4C, 0x04, 0x00, 0x00]);
        assert!(b[8..].iter().all(|&x| x == 0xAB));
    }

    #[test]
    fn key_images_fit_one_report() {
        for c in [
            Color(0, 0, 0),
            Color(255, 255, 255),
            Color(255, 0, 0),
            Color(0, 255, 0),
            Color(0, 0, 255),
            Color(12, 200, 99),
        ] {
            let img = key_image(c);
            assert!(img.len() <= PACKET - HEADER, "{c:?}: {}", img.len());
            let b = image_report(3, &img);
            assert_eq!(&b[8..8 + img.len()], &img[..]);
        }
    }

    #[test]
    fn only_changed_keys_are_resent() {
        let mut sent = [None; KEYS];
        let mut colors = vec![Color(1, 2, 3); KEYS];
        let first = changed(&sent, &colors);
        assert_eq!(first.len(), KEYS);
        for (i, c) in first {
            sent[i] = Some(c);
        }
        assert!(changed(&sent, &colors).is_empty());
        colors[7] = Color(9, 9, 9);
        assert_eq!(changed(&sent, &colors), vec![(7, Color(9, 9, 9))]);
        sent[2] = None;
        assert_eq!(
            changed(&sent, &colors),
            vec![(2, Color(1, 2, 3)), (7, Color(9, 9, 9))]
        );
    }

    #[test]
    fn missing_colours_go_out_black() {
        let sent = [Some(Color(5, 5, 5)); KEYS];
        let got = changed(&sent, &[Color(5, 5, 5); 13]);
        assert_eq!(got, vec![(13, Color::BLACK), (14, Color::BLACK)]);
    }

    #[test]
    fn layout_is_five_by_three() {
        let info = Info {
            vid: 0x0FD9,
            pid: 0x0080,
            interface: 0,
            usage_page: 0,
            usage: 0,
            path: "p".into(),
            serial: "S1".into(),
            product: String::new(),
            bus: crate::registry::Bus::Usb,
            release: 0,
        };
        let d = layout(&info, "Elgato Stream Deck MK.2");
        assert_eq!(d.id, "hid:streamdeck-0fd90080-S1");
        assert_eq!(d.kind, DeviceKind::Other);
        assert_eq!(d.grid, [5, 3]);
        assert_eq!(d.leds.len(), 15);
        assert_eq!(d.leds[0], [0.1, 1.0 / 6.0]);
        assert_eq!(d.leds[5], [0.1, 0.5]);
        assert_eq!(d.leds[14], [0.9, 5.0 / 6.0]);
    }
}
