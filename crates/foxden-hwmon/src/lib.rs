//! Read-only hardware sensors for the fan engine. Nothing in this crate can set anything.

mod amd;
mod disk;
mod thermal;

use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use foxden_fan::{Fan, FanBackend, Sensor};

/// Every read-only sensor source that works on this machine. None of them control anything.
/// Takes about a second on a machine that has ACPI thermal zones.
pub fn backends() -> Vec<Box<dyn FanBackend>> {
    // Thermal zones are watched for about a second before they are trusted.
    let zones = std::thread::Builder::new().name("foxden-thermal-open".into()).spawn(thermal::open).expect("thread start");
    let mut backends = Vec::new();
    backends.extend(amd::open());
    backends.extend(disk::open());
    backends.extend(zones.join().ok().flatten());
    backends
}

/// How a source is read: how often, and how long a reading stays good after that.
#[derive(Clone, Copy)]
struct Pace {
    every: Duration,
    keep: Duration,
}

/// For sources that answer in about a millisecond.
const QUICK: Pace = Pace {
    every: Duration::from_secs(1),
    keep: Duration::from_secs(4),
};

/// When each sensor of a source last answered, and with what.
type Latest = Mutex<Vec<Option<(Instant, f32)>>>;

/// A sensor that is read on a thread of its own. Reading it here only looks up the latest
/// answer, so a driver call that never returns costs the reading and not the caller.
#[derive(Clone)]
struct Polled {
    id: String,
    name: String,
    slot: usize,
    latest: Arc<Latest>,
    keep: Duration,
    /// The thread ends when the last copy of this has gone.
    _alive: Sender<()>,
}

impl Sensor for Polled {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self) -> Option<f32> {
        let (at, celsius) = self.latest.lock().unwrap_or_else(|e| e.into_inner())[self.slot]?;
        (at.elapsed() < self.keep).then_some(celsius)
    }
}

/// The sensors of one source. It has no fans.
struct Source {
    name: &'static str,
    sensors: Vec<Polled>,
}

impl FanBackend for Source {
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

/// Puts sensors, each with its first reading, behind a thread that reads them at `pace`.
/// `None` when there is nothing to read.
fn polled<S: Sensor + 'static>(
    name: &'static str,
    found: Vec<(S, f32)>,
    pace: Pace,
) -> Option<Box<dyn FanBackend>> {
    if found.is_empty() {
        return None;
    }
    let now = Instant::now();
    let first = found.iter().map(|&(_, celsius)| Some((now, celsius)));
    let latest: Arc<Latest> = Arc::new(Mutex::new(first.collect()));
    let (alive, gone) = mpsc::channel();
    let handle = |(slot, (sensor, _)): (usize, &(S, f32))| Polled {
        id: sensor.id().to_string(),
        name: sensor.name().to_string(),
        slot,
        latest: latest.clone(),
        keep: pace.keep,
        _alive: alive.clone(),
    };
    let sensors = found.iter().enumerate().map(handle).collect();

    let mut read: Vec<S> = found.into_iter().map(|(sensor, _)| sensor).collect();
    std::thread::Builder::new().name("foxden-hwmon-read".into()).spawn(move || {
        while let Err(RecvTimeoutError::Timeout) = gone.recv_timeout(pace.every) {
            for (slot, sensor) in read.iter_mut().enumerate() {
                // A read that fails leaves the last one in place until it is too old to trust.
                if let Some(celsius) = sensor.read() {
                    let reading = Some((Instant::now(), celsius));
                    latest.lock().unwrap_or_else(|e| e.into_inner())[slot] = reading;
                }
            }
        }
    }).expect("thread start");
    Some(Box::new(Source { name, sensors }))
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
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    const FAST: Pace = Pace {
        every: Duration::from_millis(10),
        keep: Duration::from_millis(400),
    };

    /// A sensor the test sets, which stops answering while `stuck` is set.
    #[derive(Clone, Default)]
    struct Bench {
        celsius: Arc<Mutex<Option<f32>>>,
        stuck: Arc<AtomicBool>,
    }

    impl Sensor for Bench {
        fn id(&self) -> &str {
            "bench/temperature"
        }

        fn name(&self) -> &str {
            "Bench temperature"
        }

        fn read(&mut self) -> Option<f32> {
            while self.stuck.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(5));
            }
            *self.celsius.lock().unwrap()
        }
    }

    /// The bench sensor behind a polling thread, with 40 degrees as its first reading.
    fn bench() -> (Bench, Box<dyn Sensor>) {
        let bench = Bench::default();
        let mut source = polled("bench", vec![(bench.clone(), 40.0)], FAST).unwrap();
        (bench, source.sensors().remove(0))
    }

    fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_polled_sensor_hands_out_the_latest_reading() {
        let (bench, mut sensor) = bench();
        assert_eq!(sensor.id(), "bench/temperature");
        assert_eq!(sensor.name(), "Bench temperature");
        assert_eq!(sensor.read(), Some(40.0));
        *bench.celsius.lock().unwrap() = Some(55.5);
        wait_for("the new reading", || sensor.read() == Some(55.5));
    }

    #[test]
    fn a_failed_read_keeps_the_last_reading_until_it_is_too_old() {
        let (bench, mut sensor) = bench();
        *bench.celsius.lock().unwrap() = Some(50.0);
        wait_for("the new reading", || sensor.read() == Some(50.0));
        *bench.celsius.lock().unwrap() = None;
        let failed = Instant::now();
        wait_for("the reading to be dropped", || sensor.read().is_none());
        assert!(failed.elapsed() > FAST.keep / 2, "dropped at once");
    }

    #[test]
    fn a_read_that_never_returns_costs_the_reading_and_not_the_caller() {
        let (bench, mut sensor) = bench();
        bench.stuck.store(true, Ordering::Relaxed);
        wait_for("the reading to be dropped", || {
            let asked = Instant::now();
            let reading = sensor.read();
            assert!(
                asked.elapsed() < Duration::from_millis(200),
                "the read waited"
            );
            reading.is_none()
        });
        bench.stuck.store(false, Ordering::Relaxed);
        *bench.celsius.lock().unwrap() = Some(45.0);
        wait_for("a reading again", || sensor.read() == Some(45.0));
    }

    #[test]
    fn the_polling_thread_ends_with_the_last_handle() {
        let (bench, sensor) = bench();
        // The thread holds the only other copy of the bench sensor.
        assert_eq!(Arc::strong_count(&bench.stuck), 2);
        drop(sensor);
        wait_for("the thread to end", || Arc::strong_count(&bench.stuck) == 1);
    }

    #[test]
    fn nothing_to_read_is_no_source() {
        assert!(polled("bench", Vec::<(Bench, f32)>::new(), FAST).is_none());
    }

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
                let celsius = sensor.read().expect("a source starts with a reading");
                assert!((-20.0..=150.0).contains(&celsius), "{}", sensor.id());
            }
        }
    }
}
