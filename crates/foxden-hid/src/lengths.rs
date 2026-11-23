//! LED counts the user sets for zones whose length the hardware does not report.

use std::collections::BTreeMap;
use std::sync::Mutex;

/// A zone whose LED count the user chooses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Zone {
    pub device: String,
    pub index: usize,
    pub name: String,
    pub leds: usize,
    pub min: usize,
    pub max: usize,
    /// LEDs per fan; counts are whole multiples of it. 1 for a free count.
    pub step: usize,
}

static LENGTHS: Mutex<BTreeMap<String, usize>> = Mutex::new(BTreeMap::new());
static ZONES: Mutex<Vec<Zone>> = Mutex::new(Vec::new());

/// The settings key of one zone.
pub fn key(device: &str, index: usize) -> String {
    format!("{device}#{index}")
}

/// The saved count, or `default`, clamped to `min..=max`.
pub fn pick(
    lengths: &BTreeMap<String, usize>,
    key: &str,
    default: usize,
    min: usize,
    max: usize,
) -> usize {
    lengths
        .get(key)
        .copied()
        .unwrap_or(default)
        .clamp(min, max.max(min))
}

/// Counts for the next probe, keyed by [`key`].
pub fn set_lengths(lengths: BTreeMap<String, usize>) {
    *LENGTHS.lock().unwrap_or_else(|e| e.into_inner()) = lengths;
}

/// The resizable zones of the devices the last probe opened.
pub fn resizable() -> Vec<Zone> {
    ZONES.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

pub(crate) fn clear() {
    ZONES.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

/// `n` to the nearest multiple of `step` within `min..=max`.
pub fn round(n: usize, step: usize, min: usize, max: usize) -> usize {
    let step = step.max(1);
    let lo = min.div_ceil(step) * step;
    let hi = (max.max(min) / step * step).max(lo);
    ((n + step / 2) / step * step).clamp(lo, hi)
}

/// What a driver calls when it opens: the zone's count, noted as resizable.
pub fn length(
    device: &str,
    index: usize,
    name: &str,
    default: usize,
    min: usize,
    max: usize,
) -> usize {
    length_step(device, index, name, default, min, max, 1)
}

/// [`length`] for a zone of whole fans of `step` LEDs each.
pub fn length_step(
    device: &str,
    index: usize,
    name: &str,
    default: usize,
    min: usize,
    max: usize,
    step: usize,
) -> usize {
    let step = step.max(1);
    let saved = pick(
        &LENGTHS.lock().unwrap_or_else(|e| e.into_inner()),
        &key(device, index),
        default,
        min,
        max,
    );
    let leds = round(saved, step, min, max);
    let mut zones = ZONES.lock().unwrap_or_else(|e| e.into_inner());
    zones.retain(|z| !(z.device == device && z.index == index));
    zones.push(Zone {
        device: device.into(),
        index,
        name: name.into(),
        leds,
        min,
        max: max.max(min),
        step,
    });
    leds
}

/// A saved count from `1..=upto` was stored in fans: it becomes `by` LEDs per fan.
pub(crate) fn fans_to_leds(device: &str, index: usize, upto: usize, by: usize) {
    let mut lengths = LENGTHS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(n) = lengths.get_mut(&key(device, index))
        && (1..=upto).contains(n)
    {
        *n *= by;
    }
}

#[cfg(test)]
pub(crate) fn set_one(device: &str, index: usize, leds: usize) {
    LENGTHS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key(device, index), leds);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_and_clamping() {
        let map = BTreeMap::from([(key("hid:a", 0), 12), (key("hid:a", 1), 500)]);
        assert_eq!(pick(&map, "hid:a#0", 60, 1, 120), 12);
        assert_eq!(pick(&map, "hid:a#1", 60, 1, 120), 120);
        assert_eq!(pick(&map, "hid:a#2", 60, 1, 120), 60);
        assert_eq!(
            pick(&BTreeMap::from([(key("x", 0), 0)]), "x#0", 60, 1, 120),
            1
        );
    }

    #[test]
    fn opened_zones_are_listed() {
        set_one("hid:lengths-test", 1, 7);
        assert_eq!(length("hid:lengths-test", 1, "Header 2", 60, 0, 90), 7);
        assert_eq!(length("hid:lengths-test", 0, "Header 1", 60, 0, 90), 60);
        let mine: Vec<Zone> = resizable()
            .into_iter()
            .filter(|z| z.device == "hid:lengths-test")
            .collect();
        assert_eq!(mine.len(), 2);
        assert_eq!((mine[0].index, mine[0].leds, mine[0].max), (1, 7, 90));
        assert_eq!(mine[0].step, 1);
    }

    #[test]
    fn stepped_counts_are_whole_fans() {
        assert_eq!(
            [0, 7, 8, 23, 24, 40, 64, 99].map(|n| round(n, 16, 0, 64)),
            [0, 0, 16, 16, 32, 48, 64, 64]
        );
        assert_eq!(round(3, 16, 16, 64), 16);
        assert_eq!(round(70, 20, 0, 80), 80);
        assert_eq!(round(5, 1, 0, 90), 5);
        set_one("hid:lengths-step", 0, 40);
        assert_eq!(
            length_step("hid:lengths-step", 0, "Channel 1", 64, 0, 64, 16),
            48
        );
        set_one("hid:lengths-step", 1, 2);
        fans_to_leds("hid:lengths-step", 1, 4, 16);
        assert_eq!(
            length_step("hid:lengths-step", 1, "Channel 2", 64, 0, 64, 16),
            32
        );
        let z = resizable()
            .into_iter()
            .find(|z| z.device == "hid:lengths-step")
            .unwrap();
        assert_eq!((z.leds, z.step), (48, 16));
    }
}
