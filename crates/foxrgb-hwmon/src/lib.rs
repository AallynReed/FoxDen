//! Read-only hardware sensors for the fan engine. Nothing in this crate can set anything.

mod amd;
mod disk;
mod thermal;

use foxrgb_fan::{Fan, FanBackend, Sensor};

/// Every read-only sensor source that works on this machine. None of them control anything.
/// Takes about a second on a machine that has ACPI thermal zones.
pub fn backends() -> Vec<Box<dyn FanBackend>> {
    // Thermal zones are watched for about a second before they are trusted.
    let zones = std::thread::spawn(thermal::open);
    let mut backends = Vec::new();
    backends.extend(amd::open());
    backends.extend(disk::open());
    backends.extend(zones.join().ok().flatten());
    backends
}

/// The sensors of one source. It has no fans.
struct Source<S> {
    name: &'static str,
    sensors: Vec<S>,
}

impl<S: Sensor + Clone + 'static> Source<S> {
    /// `None` when there is nothing to read.
    fn boxed(name: &'static str, sensors: Vec<S>) -> Option<Box<dyn FanBackend>> {
        (!sensors.is_empty()).then(|| Box::new(Source { name, sensors }) as Box<dyn FanBackend>)
    }
}

impl<S: Sensor + Clone + 'static> FanBackend for Source<S> {
    fn name(&self) -> &str {
        self.name
    }

    fn sensors(&mut self) -> Vec<Box<dyn Sensor>> {
        self.sensors
            .iter()
            .cloned()
            .map(|sensor| Box::new(sensor) as Box<dyn Sensor>)
            .collect()
    }

    fn fans(&mut self) -> Vec<Box<dyn Fan>> {
        Vec::new()
    }
}

/// Celsius readings outside -20 to 150 count as failed.
fn plausible(celsius: f32) -> Option<f32> {
    (-20.0..=150.0).contains(&celsius).then_some(celsius)
}

/// The text before the first NUL, trimmed.
fn text(bytes: impl IntoIterator<Item = u8>) -> String {
    let bytes: Vec<u8> = bytes.into_iter().take_while(|&byte| byte != 0).collect();
    String::from_utf8_lossy(&bytes).trim().to_string()
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn plausible_keeps_minus_20_to_150() {
        assert_eq!(plausible(-20.0), Some(-20.0));
        assert_eq!(plausible(41.5), Some(41.5));
        assert_eq!(plausible(150.0), Some(150.0));
        assert_eq!(plausible(-20.1), None);
        assert_eq!(plausible(150.1), None);
        assert_eq!(plausible(-273.15), None);
        assert_eq!(plausible(f32::NAN), None);
        assert_eq!(plausible(f32::INFINITY), None);
    }

    #[test]
    fn text_stops_at_nul_and_trims() {
        assert_eq!(text(*b"WD_BLACK SN850X  \0junk"), "WD_BLACK SN850X");
        assert_eq!(text(*b"no terminator"), "no terminator");
        assert_eq!(text(*b"\0"), "");
    }

    // Runs against whatever this machine has. Reading is all it does.
    #[test]
    fn backends_only_read() {
        // Two lists at once, as when hardware is listed again beside a running control loop.
        let lists: Vec<_> = (0..2).map(|_| std::thread::spawn(list_and_read)).collect();
        for list in lists {
            list.join().unwrap();
        }
    }

    fn list_and_read() {
        let mut ids = HashSet::new();
        for mut backend in backends() {
            assert!(backend.fans().is_empty(), "{} has fans", backend.name());
            let mut sensors = backend.sensors();
            assert!(!sensors.is_empty(), "{} has no sensors", backend.name());
            for sensor in &mut sensors {
                assert!(!sensor.name().is_empty());
                assert!(ids.insert(sensor.id().to_string()), "{} twice", sensor.id());
                if let Some(celsius) = sensor.read() {
                    assert!((-20.0..=150.0).contains(&celsius), "{}", sensor.id());
                }
            }
        }
    }
}
