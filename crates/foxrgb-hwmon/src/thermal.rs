//! ACPI thermal zones through the "Thermal Zone Information" performance counters.
//! Many desktop boards report a fixed number there, so a zone is only used once it has moved.

use std::ptr::{null, null_mut};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use foxrgb_fan::{FanBackend, Sensor};
use windows_sys::Win32::System::Performance::{
    PDH_CSTATUS_NEW_DATA, PDH_CSTATUS_VALID_DATA, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE,
    PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA, PdhAddEnglishCounterW, PdhCloseQuery,
    PdhCollectQueryData, PdhGetFormattedCounterArrayW, PdhOpenQueryW,
};

use crate::{QUICK, plausible, polled, wide};

const COUNTER: &str = r"\Thermal Zone Information(*)\High Precision Temperature";
/// The zones are read this many times, this far apart, before any of them is believed.
const SAMPLES: usize = 4;
const SAMPLE_GAP: Duration = Duration::from_millis(350);

/// Zone names with their temperatures in Celsius.
type Readings = Vec<(String, f32)>;

struct Zones {
    query: PDH_HQUERY,
    counter: PDH_HCOUNTER,
}

// The handles are only used under the mutex that owns this value.
unsafe impl Send for Zones {}

impl Drop for Zones {
    fn drop(&mut self) {
        unsafe { PdhCloseQuery(self.query) };
    }
}

impl Zones {
    fn open() -> Option<Zones> {
        unsafe {
            let mut query = null_mut();
            if PdhOpenQueryW(null(), 0, &mut query) != 0 {
                return None;
            }
            let mut zones = Zones {
                query,
                counter: null_mut(),
            };
            let path = wide(COUNTER);
            let added = PdhAddEnglishCounterW(query, path.as_ptr(), 0, &mut zones.counter);
            (added == 0).then_some(zones)
        }
    }

    /// Every zone's reading as of now. Empty when the counters cannot be read.
    fn collect(&mut self) -> Readings {
        unsafe {
            if PdhCollectQueryData(self.query) != 0 {
                return Vec::new();
            }
            let (mut bytes, mut count) = (0, 0);
            let format = PDH_FMT_DOUBLE;
            let sized = PdhGetFormattedCounterArrayW(
                self.counter,
                format,
                &mut bytes,
                &mut count,
                null_mut(),
            );
            if sized != PDH_MORE_DATA {
                return Vec::new();
            }
            // The names live in the same buffer, after the items.
            let mut buffer = vec![0u64; (bytes as usize).div_ceil(8)];
            let items: *mut PDH_FMT_COUNTERVALUE_ITEM_W = buffer.as_mut_ptr().cast();
            if PdhGetFormattedCounterArrayW(self.counter, format, &mut bytes, &mut count, items)
                != 0
            {
                return Vec::new();
            }
            std::slice::from_raw_parts(items, count as usize)
                .iter()
                .filter(|item| {
                    [PDH_CSTATUS_VALID_DATA, PDH_CSTATUS_NEW_DATA].contains(&item.FmtValue.CStatus)
                })
                .map(|item| {
                    let length = (0..).take_while(|&i| *item.szName.add(i) != 0).count();
                    let name = std::slice::from_raw_parts(item.szName, length);
                    (
                        String::from_utf16_lossy(name),
                        celsius(item.FmtValue.Anonymous.doubleValue),
                    )
                })
                .collect()
        }
    }
}

/// The counter is in tenths of a kelvin.
fn celsius(decikelvin: f64) -> f32 {
    (decikelvin / 10.0 - 273.15) as f32
}

fn find(readings: &Readings, zone: &str) -> Option<f32> {
    let (_, celsius) = readings.iter().find(|(name, _)| name == zone)?;
    plausible(*celsius)
}

/// `\_TZ.TZ00` is shown as TZ00.
fn label(zone: &str) -> &str {
    zone.rsplit(['.', '\\']).next().unwrap_or(zone)
}

/// Zones that were plausible in every sample and did not stay on one value.
fn live(samples: &[Readings]) -> Vec<String> {
    let Some(first) = samples.first() else {
        return Vec::new();
    };
    first
        .iter()
        .filter(|(zone, _)| {
            let seen: Vec<Option<f32>> = samples.iter().map(|sample| find(sample, zone)).collect();
            seen.iter().all(Option::is_some) && seen.iter().any(|reading| *reading != seen[0])
        })
        .map(|(zone, _)| zone.clone())
        .collect()
}

struct ZoneSensor {
    zones: Arc<Mutex<Zones>>,
    zone: String,
    id: String,
    name: String,
}

impl Sensor for ZoneSensor {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self) -> Option<f32> {
        let readings = self
            .zones
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .collect();
        find(&readings, &self.zone)
    }
}

pub fn open() -> Option<Box<dyn FanBackend>> {
    let mut zones = Zones::open()?;
    let mut samples = vec![zones.collect()];
    if samples[0].is_empty() {
        return None;
    }
    for _ in 1..SAMPLES {
        std::thread::sleep(SAMPLE_GAP);
        samples.push(zones.collect());
    }

    let zones = Arc::new(Mutex::new(zones));
    let found = live(&samples)
        .into_iter()
        .filter_map(|zone| {
            let celsius = find(samples.last()?, &zone)?;
            let sensor = ZoneSensor {
                zones: zones.clone(),
                id: format!("acpi/{}", label(&zone).to_lowercase()),
                name: format!("Thermal zone {}", label(&zone)),
                zone,
            };
            Some((sensor, celsius))
        })
        .collect();
    polled("acpi", found, QUICK)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(zones: &[(&str, f32)]) -> Readings {
        zones
            .iter()
            .map(|&(zone, celsius)| (zone.to_string(), celsius))
            .collect()
    }

    #[test]
    fn celsius_from_tenths_of_a_kelvin() {
        assert!((celsius(3010.0) - 27.85).abs() < 0.001);
        assert!((celsius(2731.5) - 0.0).abs() < 0.001);
        assert!((celsius(3731.5) - 100.0).abs() < 0.001);
    }

    #[test]
    fn a_zone_that_never_moves_is_dropped() {
        let samples = [
            sample(&[("fixed", 27.85), ("real", 41.0)]),
            sample(&[("fixed", 27.85), ("real", 41.0)]),
            sample(&[("fixed", 27.85), ("real", 41.5)]),
        ];
        assert_eq!(live(&samples), ["real"]);
    }

    #[test]
    fn a_zone_must_be_plausible_in_every_sample() {
        let samples = [
            sample(&[("wild", 40.0), ("gone", 40.0), ("late", 40.0)]),
            sample(&[("wild", 200.0), ("late", 41.0), ("new", 30.0)]),
            sample(&[
                ("wild", 41.0),
                ("gone", 41.0),
                ("late", 42.0),
                ("new", 31.0),
            ]),
        ];
        assert_eq!(live(&samples), ["late"]);
        assert!(live(&[]).is_empty());
    }

    #[test]
    fn a_zone_is_labelled_by_the_last_part_of_its_path() {
        assert_eq!(label(r"\_TZ.TZ00"), "TZ00");
        assert_eq!(label(r"\_SB.PCI0.LPCB.EC0.THRM"), "THRM");
        assert_eq!(label("CPUZ"), "CPUZ");
    }

    // Runs against whatever this machine has, including zones the filter drops.
    #[test]
    fn a_zone_sensor_reads_what_the_counters_report() {
        let Some(mut zones) = Zones::open() else {
            return;
        };
        let readings = zones.collect();
        let zones = Arc::new(Mutex::new(zones));
        for (zone, celsius) in readings {
            let mut sensor = ZoneSensor {
                zones: zones.clone(),
                id: String::new(),
                name: String::new(),
                zone,
            };
            let read = sensor.read();
            assert_eq!(read.is_some(), plausible(celsius).is_some());
            // A real zone may have moved a little between the two readings.
            assert!(read.is_none_or(|read| (read - celsius).abs() < 5.0));
        }
    }

    #[test]
    #[ignore = "prints what the counters report on this machine"]
    fn dump() {
        let Some(mut zones) = Zones::open() else {
            println!("no thermal zone counters");
            return;
        };
        for _ in 0..3 {
            let at = std::time::Instant::now();
            let readings = zones.collect();
            println!("{readings:?} in {:?}", at.elapsed());
            std::thread::sleep(SAMPLE_GAP);
        }
    }
}
