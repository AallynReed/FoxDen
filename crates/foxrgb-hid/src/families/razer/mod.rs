//! Ported from OpenRGB Controllers/RazerController (GPL-2.0-or-later).

mod chroma;
mod devices;
mod hanbo;
mod kraken;
mod kraken_v3;
mod kraken_v4;

use crate::registry::Detector;
use foxrgb_core::{Device, DeviceKind};

/// `RAZER_MATRIX_TYPE_*`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Matrix {
    None,
    Standard,
    Extended,
    Linear,
    ExtendedArgb,
    Custom,
    /// One colour per LED id, set as a static effect.
    Static,
}

/// One `razer_device` row; zones are (rows, cols).
pub struct Model {
    pub name: &'static str,
    pub pid: u16,
    pub kind: DeviceKind,
    pub matrix: Matrix,
    pub tid: u8,
    pub led_id: u8,
    pub rows: u8,
    pub cols: u8,
    pub zones: &'static [(u8, u8)],
}

impl Model {
    #[cfg(test)]
    pub fn led_count(&self) -> usize {
        self.zones
            .iter()
            .map(|&(r, c)| r as usize * c as usize)
            .sum()
    }
}

/// The table row the controllers pick for `pid`: the last match, or row 0 when none matches.
pub fn model(pid: u16) -> &'static Model {
    devices::MODELS
        .iter()
        .rev()
        .find(|m| m.pid == pid)
        .unwrap_or(&devices::MODELS[0])
}

/// Whether `pid` has its own row in the table.
pub fn listed(pid: u16) -> bool {
    devices::MODELS.iter().any(|m| m.pid == pid)
}

fn rect(kind: DeviceKind) -> [f32; 4] {
    match kind {
        DeviceKind::Keyboard => [0.06, 0.58, 0.54, 0.32],
        DeviceKind::Mouse => [0.70, 0.62, 0.06, 0.22],
        DeviceKind::Strip => [0.06, 0.12, 0.52, 0.18],
        _ => [0.62, 0.30, 0.30, 0.12],
    }
}

/// Zones stacked top to bottom, each row of a zone a line across the device, LEDs in OpenRGB's order.
pub fn stacked(id: String, name: String, kind: DeviceKind, zones: &[(u8, u8)]) -> Device {
    let total: usize = zones.iter().map(|&(r, _)| r as usize).sum::<usize>().max(1);
    let widest = zones
        .iter()
        .map(|&(_, c)| c as u32)
        .max()
        .unwrap_or(1)
        .max(1);
    let mut leds = Vec::new();
    let mut top = 0;
    for &(rows, cols) in zones {
        for r in 0..rows as usize {
            for c in 0..cols as usize {
                leds.push([
                    (c as f32 + 0.5) / cols as f32,
                    (top + r) as f32 / total as f32 + 0.5 / total as f32,
                ]);
            }
        }
        top += rows as usize;
    }
    Device {
        id,
        name,
        kind,
        rect: rect(kind),
        grid: [widest, total as u32],
        leds,
    }
}

pub fn detectors() -> Vec<&'static Detector> {
    devices::DETECTORS.iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(name: &str) -> Vec<&'static Detector> {
        detectors().into_iter().filter(|d| d.name == name).collect()
    }

    // REGISTER_HID_DETECTOR* lines in RazerControllerDetect.cpp, minus three commented out and the non-Windows Nommo pair,
    // plus 19 ids from SignalRGB's Razer plugins.
    #[test]
    fn one_detector_per_registered_line() {
        assert_eq!(detectors().len(), 253);
        assert!(detectors().iter().all(|d| d.vid == 0x1532));
    }

    #[test]
    fn detector_variants_match_the_macros() {
        let bw4 = find("Razer Blackwidow V4");
        assert_eq!(bw4.len(), 2);
        assert_eq!(
            (
                bw4[0].pid,
                bw4[0].interface,
                bw4[0].usage_page,
                bw4[0].usage
            ),
            (0x0287, Some(3), Some(0x01), Some(0x00))
        );
        assert_eq!(
            (bw4[1].interface, bw4[1].usage_page, bw4[1].usage),
            (Some(3), Some(0x0C), Some(0x01))
        );
        assert_eq!(find("Razer Blackwidow V4 Pro").len(), 2);
        let naga = &find("Razer Naga Pro V3 (Wired)")[0];
        assert_eq!(
            (naga.interface, naga.usage_page, naga.usage),
            (None, Some(1), Some(3))
        );
        let tiamat = &find("Razer Tiamat 7.1 V2")[0];
        assert_eq!(
            (tiamat.interface, tiamat.usage_page, tiamat.usage),
            (Some(0), None, None)
        );
        let v4 = &find("Razer Kraken V4 (Wired)")[0];
        assert_eq!(
            (v4.pid, v4.interface, v4.usage_page, v4.usage),
            (0x056B, Some(5), Some(0xFF14), Some(1))
        );
        for name in [
            "Razer Seiren Emote",
            "Razer Thunderbolt 4 Dock Chroma",
            "Razer Thunderbolt 5 Dock Chroma",
        ] {
            let d = &find(name)[0];
            assert_eq!(
                (d.interface, d.usage_page, d.usage),
                (None, Some(0x0C), Some(1))
            );
        }
        let nommo = &find("Razer Nommo Chroma")[0];
        assert_eq!(
            (nommo.interface, nommo.usage_page, nommo.usage),
            (Some(1), Some(1), Some(3))
        );
        let argb = &find("Razer Chroma Addressable RGB Controller")[0];
        assert_eq!(
            (argb.pid, argb.interface, argb.usage_page),
            (0x0F1F, Some(0), None)
        );
        let core = &find("Razer Core")[0];
        assert_eq!((core.usage_page, core.usage), (Some(0xFF00), Some(1)));
    }

    #[test]
    fn detectors_open_with_the_right_controller() {
        let opens =
            |name: &str, f: crate::registry::Open| std::ptr::fn_addr_eq(find(name)[0].open, f);
        assert!(opens("Razer Huntsman", chroma::open));
        assert!(opens(
            "Razer Chroma Addressable RGB Controller",
            chroma::open_argb
        ));
        assert!(opens("Razer Kraken 7.1 V2", kraken::open));
        assert!(opens("Razer Kraken V3", kraken_v3::open));
        assert!(opens(
            "Razer Kraken Kitty V3 Pro (Wireless)",
            kraken_v4::open
        ));
        assert!(opens("Razer Hanbo Chroma", hanbo::open));
        assert!(opens("Razer Kraken Kitty Black Edition", chroma::open));
    }

    // Controller constructors: a PID missing from device_list keeps device_index 0. The Kitty
    // Black Edition shares 0x0F21 with the Thunderbolt 4 Dock and gets that row.
    #[test]
    fn pid_lookup_matches_the_constructors() {
        assert!(!listed(0x0501));
        assert_eq!(model(0x0501).name, "Razer BlackWidow 2019");
        assert_eq!(model(0x0F21).name, "Razer Thunderbolt 4 Dock Chroma");
        assert_eq!(model(0x0287).name, "Razer Blackwidow V4");
    }

    #[test]
    fn table_rows_are_sane() {
        assert_eq!(devices::MODELS.len(), 251);
        for m in &devices::MODELS {
            assert!(m.led_count() > 0, "{}", m.name);
            assert!(
                m.led_count() <= m.rows as usize * m.cols as usize,
                "{}",
                m.name
            );
        }
        let bw4 = model(0x0287);
        assert_eq!((bw4.rows, bw4.cols, bw4.led_count()), (8, 23, 156));
    }

    #[test]
    fn added_rows() {
        let blade = model(0x02C7);
        assert_eq!(
            (blade.matrix, blade.tid, blade.rows, blade.cols),
            (Matrix::Standard, 0xFF, 6, 19)
        );
        assert_eq!((model(0x02E0).rows, model(0x02E0).cols), (6, 17));
        assert_eq!(model(0x00DA).matrix, Matrix::Static);
        assert_eq!(model(0x02CD).tid, 0x1F);
        assert_eq!(model(0x021F).tid, 0x3F);
        assert_eq!(model(0x0F17).led_count(), 20);
        assert_eq!((model(0x02A7).rows, model(0x02A7).cols), (6, 22));
        for pid in [
            0x0214, 0x0237, 0x024B, 0x02E1, 0x003F, 0x00A1, 0x00D4, 0x0520, 0x0F12,
        ] {
            assert!(listed(pid), "{pid:04x}");
            assert!(detectors().iter().any(|d| d.pid == pid), "{pid:04x}");
        }
    }

    #[test]
    fn every_led_is_placed_once() {
        for m in &devices::MODELS {
            let d = stacked("x".into(), m.name.into(), m.kind, m.zones);
            assert_eq!(d.leds.len(), m.led_count(), "{}", m.name);
            let mut cells: Vec<(u32, u32)> = d
                .leds
                .iter()
                .map(|p| ((p[0] * 10000.0) as u32, (p[1] * 10000.0) as u32))
                .collect();
            cells.sort();
            cells.dedup();
            assert_eq!(cells.len(), d.leds.len(), "{}", m.name);
        }
    }

    #[test]
    fn stacked_layout_keeps_led_order_and_bounds() {
        let d = stacked(
            "x".into(),
            "X".into(),
            DeviceKind::Keyboard,
            &[(6, 23), (1, 9), (1, 9)],
        );
        assert_eq!(d.leds.len(), 156);
        assert_eq!(d.grid, [23, 8]);
        assert!(d.leds.iter().flatten().all(|v| (0.0..=1.0).contains(v)));
        assert!(d.leds[138][1] > d.leds[137][1]);
        assert!(d.leds[147][1] > d.leds[146][1]);
    }
}
