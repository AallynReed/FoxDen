//! Ported from OpenRGB Controllers/RazerController/RazerKrakenController (GPL-2.0-or-later).

use super::{listed, model, stacked};
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, Device, DeviceKind};
use hidapi::{HidApi, HidDevice};

pub const REPORT_LEN: usize = 37;

const KRAKEN: u16 = 0x0504;
const KRAKEN_V2: u16 = 0x0510;
const KRAKEN_TE: u16 = 0x0520;
const KRAKEN_KITTY_BLACK_EDITION_V2: u16 = 0x0560;
const KRAKEN_ULTIMATE: u16 = 0x0527;

/// `on_off_static` bit of `razer_kraken_effect_byte`.
const EFFECT_ON: u8 = 0x01;

/// The register addresses the constructor picks per PID: (led mode, custom colour).
pub fn addresses(pid: u16) -> (u16, u16) {
    match pid {
        KRAKEN_V2 | KRAKEN_TE | KRAKEN_KITTY_BLACK_EDITION_V2 | KRAKEN_ULTIMATE => (0x172D, 0x1189),
        _ => (0x1008, 0x1189),
    }
}

/// `razer_kraken_create_report`.
pub fn report(id: u8, destination: u8, length: u8, address: u16, args: &[u8]) -> [u8; REPORT_LEN] {
    let mut r = [0u8; REPORT_LEN];
    r[0] = id;
    r[1] = destination;
    r[2] = length;
    r[3] = (address >> 8) as u8;
    r[4] = address as u8;
    r[5..5 + args.len()].copy_from_slice(args);
    r
}

/// `razer_set_mode_custom`: the colour write is skipped on the Kraken Classic.
pub fn set_mode_custom(pid: u16, c: Color) -> Vec<[u8; REPORT_LEN]> {
    let (led_mode, custom) = addresses(pid);
    let mut out = Vec::new();
    if matches!(
        pid,
        KRAKEN | KRAKEN_V2 | KRAKEN_TE | KRAKEN_KITTY_BLACK_EDITION_V2 | KRAKEN_ULTIMATE
    ) {
        out.push(report(0x04, 0x40, 3, custom, &[c.0, c.1, c.2]));
    }
    out.push(report(0x04, 0x40, 0x01, led_mode, &[EFFECT_ON]));
    out
}

pub struct Kraken {
    hid: HidDevice,
    pid: u16,
    device: Device,
    sent: Option<Vec<Color>>,
}

fn device(id: String, name: String, pid: u16) -> Device {
    let zones: &[(u8, u8)] = if listed(pid) {
        model(pid).zones
    } else {
        &[(1, 1)]
    };
    stacked(id, name, DeviceKind::Other, zones)
}

pub fn open(api: &HidApi, info: &Info, detector: &Detector) -> Result<Box<dyn Backend>, String> {
    let hid = info.open(api)?;
    Ok(Box::new(Kraken {
        hid,
        pid: info.pid,
        device: device(info.device_id("razer"), detector.name.into(), info.pid),
        sent: None,
    }))
}

impl Backend for Kraken {
    fn name(&self) -> &str {
        "razer kraken"
    }

    fn devices(&self) -> Vec<Device> {
        vec![self.device.clone()]
    }

    /// `DeviceUpdateLEDs`: the whole headset takes the first colour.
    fn push(&mut self, device_id: &str, colors: &[Color]) -> Result<(), String> {
        if device_id != self.device.id {
            return Err(format!("unknown device {device_id}"));
        }
        if self.sent.as_deref() == Some(colors) {
            return Ok(());
        }
        let first = colors.first().copied().unwrap_or(Color::BLACK);
        for r in set_mode_custom(self.pid, first) {
            if let Err(e) = self.hid.write(&r) {
                self.sent = None;
                return Err(e.to_string());
            }
        }
        self.sent = Some(colors.to_vec());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KRAKEN_CLASSIC: u16 = 0x0501;
    const KRAKEN_CLASSIC_ALT: u16 = 0x0506;

    // RazerKrakenController::razer_set_mode_custom on the Kraken V2.
    #[test]
    fn v2_custom_writes_colour_then_effect() {
        let r = set_mode_custom(KRAKEN_V2, Color(0x11, 0x22, 0x33));
        assert_eq!(r.len(), 2);
        let mut rgb = [0u8; 37];
        rgb[..8].copy_from_slice(&[0x04, 0x40, 0x03, 0x11, 0x89, 0x11, 0x22, 0x33]);
        assert_eq!(r[0], rgb);
        let mut fx = [0u8; 37];
        fx[..6].copy_from_slice(&[0x04, 0x40, 0x01, 0x17, 0x2D, 0x01]);
        assert_eq!(r[1], fx);
    }

    // Tournament Edition uses the V2 addresses and writes the colour.
    #[test]
    fn tournament_edition_matches_v2() {
        let c = Color(0x11, 0x22, 0x33);
        assert_eq!(set_mode_custom(KRAKEN_TE, c), set_mode_custom(KRAKEN_V2, c));
        assert_eq!(addresses(KRAKEN_TE), (0x172D, 0x1189));
    }

    // razer_set_mode_custom on the Kraken 7.1 Chroma.
    #[test]
    fn chroma_uses_the_old_mode_address() {
        let r = set_mode_custom(KRAKEN, Color(1, 2, 3));
        assert_eq!(r.len(), 2);
        assert_eq!(r[0][..8], [0x04, 0x40, 0x03, 0x11, 0x89, 1, 2, 3]);
        assert_eq!(r[1][..6], [0x04, 0x40, 0x01, 0x10, 0x08, 0x01]);
    }

    // razer_set_mode_custom on the Kraken Classic: no colour register.
    #[test]
    fn classic_only_writes_the_effect() {
        for pid in [KRAKEN_CLASSIC, KRAKEN_CLASSIC_ALT] {
            let r = set_mode_custom(pid, Color(1, 2, 3));
            assert_eq!(r.len(), 1);
            assert_eq!(r[0][..6], [0x04, 0x40, 0x01, 0x10, 0x08, 0x01]);
        }
    }

    #[test]
    fn devices() {
        assert_eq!(device("x".into(), "X".into(), KRAKEN_CLASSIC).leds.len(), 1);
        assert_eq!(
            device("x".into(), "X".into(), KRAKEN_ULTIMATE).leds.len(),
            1
        );
        assert_eq!(
            device("x".into(), "X".into(), KRAKEN_KITTY_BLACK_EDITION_V2)
                .leds
                .len(),
            4
        );
    }
}
