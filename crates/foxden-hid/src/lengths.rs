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

/// What a driver calls when it opens: the zone's count, noted as resizable.
pub fn length(
    device: &str,
    index: usize,
    name: &str,
    default: usize,
    min: usize,
    max: usize,
) -> usize {
    let leds = pick(
        &LENGTHS.lock().unwrap_or_else(|e| e.into_inner()),
        &key(device, index),
        default,
        min,
        max,
    );
    let mut zones = ZONES.lock().unwrap_or_else(|e| e.into_inner());
    zones.retain(|z| !(z.device == device && z.index == index));
    zones.push(Zone {
        device: device.into(),
        index,
        name: name.into(),
        leds,
        min,
        max: max.max(min),
    });
    leds
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
    }
}
