//! Ported from OpenRGB Controllers/JGINYUEInternalUSBController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, Device, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "jginyue";
/// OpenRGB leaves header sizes to the user; FoxRGB drives both headers at their 100-LED maximum.
const HEADER_LEDS: usize = 100;
const RG_DEFAULT: u8 = 0x01;

/// LED stripe settings: LED count, RG swap, direction and whether per-LED control is on.
pub fn stripe_set(area: u8, leds: u8, direct: bool) -> [u8; 16] {
    let mut b = [0u8; 16];
    b[..6].copy_from_slice(&[0x05, area, leds, RG_DEFAULT, 0x00, direct as u8]);
    b
}

/// DirectLEDControl's per-LED feature report.
pub fn per_led(area: u8, colors: &[Color]) -> [u8; 302] {
    let mut b = [0u8; 302];
    b[0] = 0x04;
    b[1] = area;
    for (i, c) in colors.iter().take(HEADER_LEDS).enumerate() {
        b[2 + i * 3..5 + i * 3].copy_from_slice(&rgb(*c));
    }
    b
}

struct Jginyue;

impl Proto for Jginyue {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        // Area_resize for both headers, then the first DirectLEDControl switches each to direct
        for area in [1, 2] {
            io.write(&stripe_set(area, HEADER_LEDS as u8, false))?;
        }
        for area in [1, 2] {
            io.write(&stripe_set(area, HEADER_LEDS as u8, true))?;
        }
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for (z, zone) in colors.chunks(HEADER_LEDS).enumerate() {
            io.feature(&per_led(z as u8 + 1, zone))?;
        }
        Ok(())
    }
}

fn device(info: &Info, d: &Detector) -> Device {
    let kind = DeviceKind::Motherboard;
    let mut dev = Device::grid(
        &info.device_id(FAMILY),
        d.name,
        kind,
        rect(kind),
        HEADER_LEDS as u32,
        2,
    );
    dev.leds = (0..2)
        .flat_map(|r| {
            (0..HEADER_LEDS)
                .map(move |i| [(i as f32 + 0.5) / HEADER_LEDS as f32, 0.25 + 0.5 * r as f32])
        })
        .collect();
    dev
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    boxed(api, info, FAMILY, device(info, d), Jginyue)
}

pub static DETECTORS: &[Detector] = &[det(
    "JGINYUE Internal USB Controller",
    0x0416,
    0xA125,
    ANY,
    ANY,
    ANY,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // Area_resize, DirectLEDControl's switch to direct, then its per-LED reports
    #[test]
    fn resize_direct_then_colours() {
        let mut a = vec![Color::BLACK; 200];
        a[0] = Color(1, 2, 3);
        a[199] = Color(4, 5, 6);
        let [first, _, _] = pushes(Jginyue, 200, vec![], &a, &[]);
        assert_eq!(first[0], w(pad(&[5, 1, 100, 1, 0, 0], 16)));
        assert_eq!(first[1], w(pad(&[5, 2, 100, 1, 0, 0], 16)));
        assert_eq!(first[2], w(pad(&[5, 1, 100, 1, 0, 1], 16)));
        assert_eq!(first[3], w(pad(&[5, 2, 100, 1, 0, 1], 16)));
        assert_eq!(first[4], Op::F(pad(&[4, 1, 1, 2, 3], 302)));
        let mut z2 = pad(&[4, 2], 302);
        z2[299..302].copy_from_slice(&[4, 5, 6]);
        assert_eq!(first[5], Op::F(z2));
    }
}
