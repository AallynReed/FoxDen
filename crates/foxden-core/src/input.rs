use crate::Color;
use serde::Serialize;
use std::collections::BTreeMap;

pub const AUDIO_BANDS: usize = 16;

/// Latest audio analysis, low to high frequency. All values 0..=1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct AudioFrame {
    pub level: f32,
    pub bands: [f32; AUDIO_BANDS],
}

/// Downscaled desktop image, row-major. Empty while capture is off.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScreenFrame {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<Color>,
}

impl ScreenFrame {
    /// Nearest pixel to canvas position `x`, `y` (0..=1). Black when empty.
    pub fn sample(&self, x: f32, y: f32) -> Color {
        if self.width == 0 || self.height == 0 || self.pixels.len() < self.width * self.height {
            return Color::BLACK;
        }
        let px = ((x.clamp(0.0, 1.0) * self.width as f32) as usize).min(self.width - 1);
        let py = ((y.clamp(0.0, 1.0) * self.height as f32) as usize).min(self.height - 1);
        self.pixels[py * self.width + px]
    }
}

/// A recent key press at canvas position `x`, `y`, `age` seconds ago.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Press {
    pub x: f32,
    pub y: f32,
    pub age: f32,
    /// Half the key's width and height on the canvas.
    pub hw: f32,
    pub hh: f32,
    /// Different for every press, for colours picked at random.
    pub seed: u32,
}

/// Everything an effect may react to besides position and time.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Inputs {
    pub audio: AudioFrame,
    pub screen: ScreenFrame,
    pub presses: Vec<Press>,
    /// Each lit keyboard on the canvas: its rect, and whether it has a number pad.
    pub keyboards: Vec<([f32; 4], bool)>,
    /// Temperatures in °C by sensor id.
    pub temperatures: BTreeMap<String, f32>,
    pub batteries: Batteries,
}

/// One connected device's battery.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct BatteryLevel {
    pub percent: f32,
    pub charging: bool,
}

/// Battery levels by the device's stable key.
pub type Batteries = BTreeMap<String, BatteryLevel>;

/// The battery of `device`, or the lowest one when `device` is empty.
pub fn battery_level(batteries: &Batteries, device: &str) -> Option<BatteryLevel> {
    if device.is_empty() {
        batteries
            .values()
            .filter(|b| b.percent.is_finite())
            .min_by(|a, b| a.percent.total_cmp(&b.percent))
            .copied()
    } else {
        batteries.get(device).copied()
    }
}

/// True when `device`, or any device when it is empty, is at or below `threshold` percent and not charging.
pub fn battery_low(batteries: &Batteries, device: &str, threshold: f32) -> bool {
    let low = |b: &BatteryLevel| !b.charging && b.percent <= threshold;
    if device.is_empty() {
        batteries.values().any(low)
    } else {
        batteries.get(device).is_some_and(low)
    }
}

/// Eases each reading toward `latest` with time constant `tau` seconds. New sensors start at their
/// reading and sensors that went quiet are dropped.
pub fn ease_readings(
    eased: &mut BTreeMap<String, f32>,
    latest: &[(String, f32)],
    dt: f32,
    tau: f32,
) {
    let k = 1.0 - (-dt.max(0.0) / tau.max(1e-3)).exp();
    eased.retain(|id, _| latest.iter().any(|(l, _)| l == id));
    for (id, value) in latest.iter().filter(|(_, v)| v.is_finite()) {
        let now = eased.entry(id.clone()).or_insert(*value);
        *now += (value - *now) * k;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_sample() {
        let frame = ScreenFrame {
            width: 2,
            height: 1,
            pixels: vec![Color(1, 0, 0), Color(2, 0, 0)],
        };
        assert_eq!(frame.sample(0.1, 0.5), Color(1, 0, 0));
        assert_eq!(frame.sample(1.0, 1.0), Color(2, 0, 0));
        assert_eq!(ScreenFrame::default().sample(0.5, 0.5), Color::BLACK);
    }

    #[test]
    fn readings_ease_toward_the_newest() {
        let mut eased = BTreeMap::new();
        ease_readings(&mut eased, &[("cpu".into(), 40.0)], 0.03, 0.3);
        assert_eq!(eased["cpu"], 40.0);
        ease_readings(&mut eased, &[("cpu".into(), 80.0)], 0.1, 0.3);
        assert!(eased["cpu"] > 40.0 && eased["cpu"] < 60.0);
        for _ in 0..40 {
            ease_readings(&mut eased, &[("cpu".into(), 80.0)], 0.03, 0.3);
        }
        assert!((eased["cpu"] - 80.0).abs() < 1.0);
        ease_readings(&mut eased, &[("gpu".into(), f32::NAN)], 0.03, 0.3);
        assert!(eased.is_empty());
    }

    fn batteries(list: &[(&str, f32, bool)]) -> Batteries {
        list.iter()
            .map(|&(k, percent, charging)| (k.to_string(), BatteryLevel { percent, charging }))
            .collect()
    }

    #[test]
    fn lowest_battery_is_picked_when_no_device_is_named() {
        let b = batteries(&[
            ("mouse", 40.0, false),
            ("pad", 12.0, true),
            ("set", 80.0, false),
        ]);
        assert_eq!(battery_level(&b, "").unwrap().percent, 12.0);
        assert_eq!(battery_level(&b, "set").unwrap().percent, 80.0);
        assert_eq!(battery_level(&b, "gone"), None);
        assert_eq!(battery_level(&Batteries::new(), ""), None);
    }

    #[test]
    fn low_means_at_or_under_the_threshold_and_not_charging() {
        let b = batteries(&[
            ("mouse", 20.0, false),
            ("pad", 5.0, true),
            ("set", 60.0, false),
        ]);
        assert!(battery_low(&b, "mouse", 20.0));
        assert!(!battery_low(&b, "mouse", 19.0));
        assert!(!battery_low(&b, "pad", 20.0));
        assert!(!battery_low(&b, "gone", 20.0));
        assert!(battery_low(&b, "", 20.0));
        assert!(!battery_low(&b, "", 10.0));
        assert!(!battery_low(&Batteries::new(), "", 100.0));
    }
}
