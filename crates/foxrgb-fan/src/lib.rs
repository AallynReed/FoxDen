//! Fan control engine: sensors, fans, curves and the controller that ties them together.

mod sim;
#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::ops::RangeInclusive;

use serde::{Deserialize, Serialize};

pub use sim::SimBackend;

/// Degrees Celsius a working sensor can report. Anything else counts as unreadable.
const PLAUSIBLE: RangeInclusive<f32> = -20.0..=150.0;
/// Refused writes in a row before a fan is handed back to the firmware.
const WRITE_TRIES: u32 = 3;
/// Seconds a fan handed back that way is left alone before the next try.
const RETRY_AFTER: f32 = 30.0;

/// A temperature source, in degrees Celsius.
pub trait Sensor: Send {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    fn read(&mut self) -> Option<f32>;
}

/// One controllable fan header or pump.
pub trait Fan: Send {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    fn rpm(&mut self) -> Option<u32>;
    /// `duty` is 0..=1.
    fn set_duty(&mut self, duty: f32) -> Result<(), String>;
    /// Hands control back to the firmware.
    fn release(&mut self) -> Result<(), String>;
}

pub trait FanBackend: Send {
    fn name(&self) -> &str;
    fn sensors(&mut self) -> Vec<Box<dyn Sensor>>;
    fn fans(&mut self) -> Vec<Box<dyn Fan>>;
    /// Called once per tick with the elapsed seconds. Only simulated hardware needs it.
    fn advance(&mut self, _dt: f32) {}
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FanConfig {
    pub curves: Vec<Curve>,
    pub rules: Vec<FanRule>,
}

/// Duty as a function of temperature.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Curve {
    pub name: String,
    /// Sensor ids. The hottest one drives the curve, and all of them have to read.
    pub sensors: Vec<String>,
    /// `(temperature, duty)` points in rising temperature order; duty is 0..=1.
    pub points: Vec<(f32, f32)>,
    /// Degrees the temperature must fall before duty drops.
    #[serde(default = "default_hysteresis")]
    pub hysteresis: f32,
}

/// Binds a fan to a curve.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FanRule {
    pub fan: String,
    pub curve: String,
    #[serde(default)]
    pub min_duty: f32,
    #[serde(default = "default_max_duty")]
    pub max_duty: f32,
    /// Largest duty change per second.
    #[serde(default = "default_ramp")]
    pub ramp: f32,
}

fn default_hysteresis() -> f32 {
    2.0
}

fn default_max_duty() -> f32 {
    1.0
}

fn default_ramp() -> f32 {
    0.2
}

impl Curve {
    /// Duty at `temp`: linear between points, flat beyond the first and last.
    pub fn duty_at(&self, temp: f32) -> f32 {
        let Some(&(first_temp, first_duty)) = self.points.first() else {
            return 1.0;
        };
        if temp <= first_temp {
            return first_duty;
        }
        for pair in self.points.windows(2) {
            let ((t0, d0), (t1, d1)) = (pair[0], pair[1]);
            if temp <= t1 {
                // In f64, where the width of a segment cannot overflow.
                let along = (f64::from(temp) - f64::from(t0)) / (f64::from(t1) - f64::from(t0));
                return d0 + (d1 - d0) * along as f32;
            }
        }
        self.points[self.points.len() - 1].1
    }
}

impl FanConfig {
    pub fn validate(&self) -> Result<(), String> {
        let mut names = HashSet::new();
        for curve in &self.curves {
            let name = &curve.name;
            if !names.insert(name.as_str()) {
                return Err(format!("curve name '{name}' is used more than once"));
            }
            if curve.sensors.is_empty() {
                return Err(format!("curve '{name}' has no sensors"));
            }
            if curve.points.is_empty() {
                return Err(format!("curve '{name}' has no points"));
            }
            for &(temp, duty) in &curve.points {
                if !temp.is_finite() || !duty.is_finite() {
                    return Err(format!("curve '{name}' has a point that is not a finite number"));
                }
                if !(0.0..=1.0).contains(&duty) {
                    return Err(format!("curve '{name}' has a duty outside 0 to 1"));
                }
            }
            if curve.points.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
                return Err(format!("curve '{name}' points must be sorted by rising temperature"));
            }
            if !(0.0..=30.0).contains(&curve.hysteresis) {
                return Err(format!("curve '{name}' hysteresis must be from 0 to 30"));
            }
        }

        let mut fans = HashSet::new();
        for rule in &self.rules {
            let fan = &rule.fan;
            if !fans.insert(fan.as_str()) {
                return Err(format!("fan '{fan}' has more than one rule"));
            }
            if !names.contains(rule.curve.as_str()) {
                return Err(format!("fan '{fan}' uses unknown curve '{}'", rule.curve));
            }
            // Written so that NaN fails every comparison.
            if !(rule.min_duty >= 0.0 && rule.min_duty <= rule.max_duty && rule.max_duty <= 1.0) {
                return Err(format!(
                    "fan '{fan}' needs duties from 0 to 1, with the minimum not above the maximum"
                ));
            }
            if !(rule.ramp >= 0.01 && rule.ramp.is_finite()) {
                return Err(format!("fan '{fan}' ramp must be a finite number of at least 0.01"));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SensorStatus {
    pub id: String,
    pub name: String,
    pub value: Option<f32>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FanStatus {
    pub id: String,
    pub name: String,
    pub rpm: Option<u32>,
    /// Last duty the fan accepted. `None` while the firmware is in control.
    pub duty: Option<f32>,
    pub curve: Option<String>,
    pub failsafe: bool,
    /// Why the last write or release failed.
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Status {
    pub sensors: Vec<SensorStatus>,
    pub fans: Vec<FanStatus>,
}

struct Slot {
    fan: Box<dyn Fan>,
    duty: Option<f32>,
    touched: bool,
    /// Writes refused in a row, and what the last refusal said.
    failures: u32,
    fault: Option<String>,
    /// Seconds left before a fan that was handed back is tried again.
    rest: f32,
}

impl Slot {
    fn new(fan: Box<dyn Fan>) -> Self {
        Slot {
            fan,
            duty: None,
            touched: false,
            failures: 0,
            fault: None,
            rest: 0.0,
        }
    }

    /// A fan that keeps refusing goes back to the firmware, which at least runs its own curve.
    fn write(&mut self, duty: f32) {
        self.touched = true;
        match self.fan.set_duty(duty) {
            Ok(()) => {
                self.duty = Some(duty);
                self.failures = 0;
                self.fault = None;
            }
            Err(error) => {
                self.failures += 1;
                self.fault = Some(if self.failures < WRITE_TRIES {
                    error
                } else {
                    match self.release() {
                        Ok(()) => {
                            self.rest = RETRY_AFTER;
                            format!("{error}; fan handed back to the firmware")
                        }
                        Err(release) => format!("{error}; {release}"),
                    }
                });
            }
        }
    }

    /// A fan that refuses stays marked, so the next call tries again.
    fn release(&mut self) -> Result<(), String> {
        if self.touched {
            self.fan.release()?;
            self.touched = false;
            self.duty = None;
            self.failures = 0;
        }
        Ok(())
    }
}

/// The sensor side of the hardware. It can be read apart from the fans, so that a read that
/// never returns holds nothing a fan needs.
#[derive(Default)]
pub struct Probe {
    backends: Vec<Box<dyn FanBackend>>,
    sensors: Vec<Box<dyn Sensor>>,
}

impl Probe {
    /// Moves the backends on by `dt` seconds and reads every sensor.
    pub fn read(&mut self, dt: f32) -> Vec<SensorStatus> {
        for backend in &mut self.backends {
            backend.advance(seconds(dt));
        }
        self.sensors
            .iter_mut()
            .map(|sensor| SensorStatus {
                id: sensor.id().to_string(),
                name: sensor.name().to_string(),
                value: sensor.read().filter(|value| PLAUSIBLE.contains(value)),
            })
            .collect()
    }
}

/// Elapsed seconds, with anything that is not a positive number counted as none.
fn seconds(dt: f32) -> f32 {
    if dt.is_finite() && dt > 0.0 { dt } else { 0.0 }
}

pub struct Controller {
    probe: Probe,
    fans: Vec<Slot>,
    config: FanConfig,
    /// Per curve, the temperature that set its current duty.
    anchors: Vec<Option<f32>>,
}

impl Controller {
    /// An invalid `config` is replaced by an empty one, which leaves every fan alone.
    pub fn new(backends: Vec<Box<dyn FanBackend>>, config: FanConfig) -> Self {
        let config = if config.validate().is_ok() {
            config
        } else {
            FanConfig::default()
        };
        let mut controller = Controller {
            probe: Probe::default(),
            fans: Vec::new(),
            anchors: vec![None; config.curves.len()],
            config,
        };
        controller.attach(backends);
        controller
    }

    fn attach(&mut self, mut backends: Vec<Box<dyn FanBackend>>) {
        self.fans.clear();
        let mut sensors = Vec::new();
        for backend in &mut backends {
            sensors.extend(backend.sensors());
            self.fans.extend(backend.fans().into_iter().map(Slot::new));
        }
        self.probe = Probe { backends, sensors };
    }

    /// Swaps the hardware. Every fan the controller had touched is handed back first; if one
    /// refuses, the old backends stay.
    pub fn set_backends(&mut self, backends: Vec<Box<dyn FanBackend>>) -> Result<(), String> {
        self.release_all()?;
        self.attach(backends);
        Ok(())
    }

    /// Validates and applies a new configuration. On error the old one stays in force.
    pub fn set_config(&mut self, config: FanConfig) -> Result<(), String> {
        config.validate()?;
        self.anchors = vec![None; config.curves.len()];
        self.config = config;
        Ok(())
    }

    pub fn config(&self) -> &FanConfig {
        &self.config
    }

    /// Takes the sensors out, to be read without holding the controller. `tick` has none to
    /// read after that: hand the readings to `apply` instead.
    pub fn take_probe(&mut self) -> Probe {
        std::mem::take(&mut self.probe)
    }

    /// Reads sensors, evaluates curves and applies duties. `dt` is seconds since the last tick.
    pub fn tick(&mut self, dt: f32) -> Status {
        let sensors = self.probe.read(dt);
        self.apply(dt, sensors)
    }

    /// Evaluates the curves against `sensors` and applies duties. `dt` is seconds since the
    /// last call.
    pub fn apply(&mut self, dt: f32, sensors: Vec<SensorStatus>) -> Status {
        let dt = seconds(dt);
        let reading = |id: &String| sensors.iter().find(|sensor| &sensor.id == id)?.value;

        // Per curve: the duty it asks for, or None when one of its sensors gives no reading.
        let mut wanted = Vec::with_capacity(self.config.curves.len());
        for (curve, anchor) in self.config.curves.iter().zip(&mut self.anchors) {
            // A sensor that is lost may have been the hottest, so the others are not enough.
            let hottest = curve
                .sensors
                .iter()
                .map(reading)
                .try_fold(f32::NEG_INFINITY, |hottest, temp| Some(hottest.max(temp?)))
                .filter(|temp| temp.is_finite());
            *anchor = match (hottest, *anchor) {
                // Held only while that keeps the duty at or above what the curve asks for now.
                (Some(temp), Some(held))
                    if temp < held
                        && temp > held - curve.hysteresis
                        && curve.duty_at(temp) <= curve.duty_at(held) =>
                {
                    Some(held)
                }
                (temp, _) => temp,
            };
            let duty = anchor.map(|temp| curve.duty_at(temp));
            // A duty that is not a number is treated like a lost sensor.
            wanted.push(duty.filter(|duty| duty.is_finite()));
        }

        let mut fans = Vec::with_capacity(self.fans.len());
        for slot in &mut self.fans {
            let mut status = FanStatus {
                id: slot.fan.id().to_string(),
                name: slot.fan.name().to_string(),
                ..FanStatus::default()
            };
            slot.rest = (slot.rest - dt).max(0.0);
            match self.config.rules.iter().find(|rule| rule.fan == status.id) {
                Some(rule) => {
                    let curve = self.config.curves.iter().position(|c| c.name == rule.curve);
                    let duty = match curve.and_then(|index| wanted[index]) {
                        Some(duty) => {
                            let target = duty.clamp(rule.min_duty, rule.max_duty);
                            let step = rule.ramp * dt;
                            match slot.duty {
                                Some(now) if (target - now).abs() > step => {
                                    now + step.copysign(target - now)
                                }
                                _ => target,
                            }
                        }
                        None => {
                            status.failsafe = true;
                            1.0
                        }
                    };
                    if slot.rest == 0.0 {
                        slot.write(duty);
                    }
                    status.curve = Some(rule.curve.clone());
                    status.error = slot.fault.clone();
                }
                // A fan whose rule was removed goes back to the firmware.
                None => status.error = slot.release().err(),
            }
            status.duty = slot.duty;
            status.rpm = slot.fan.rpm();
            fans.push(status);
        }

        Status { sensors, fans }
    }

    /// Hands every fan back to the firmware and names the ones that refused. Also runs on drop.
    pub fn release_all(&mut self) -> Result<(), String> {
        self.anchors.fill(None);
        let refused: Vec<String> = self
            .fans
            .iter_mut()
            .filter_map(|slot| {
                let error = slot.release().err()?;
                Some(format!("{}: {error}", slot.fan.id()))
            })
            .collect();
        if refused.is_empty() {
            Ok(())
        } else {
            Err(refused.join("; "))
        }
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        let _ = self.release_all();
    }
}
