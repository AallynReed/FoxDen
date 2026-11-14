//! Fan control engine: sensors, fans, curves and the controller that ties them together.

mod sim;
#[cfg(test)]
mod tests;

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

pub use sim::SimBackend;

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
    /// Sensor ids. The hottest one drives the curve.
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
            if curve.points.is_empty() {
                return Err(format!("curve '{name}' has no points"));
            }
            for &(temp, duty) in &curve.points {
                if !temp.is_finite() || !duty.is_finite() {
                    return Err(format!("curve '{name}' has a point that is not a finite number"));
                }
                if !(0.0..=1.0).contains(&duty) {
                    return Err(format!("curve '{name}' has a duty outside 0..=1"));
                }
            }
            if curve.points.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
                return Err(format!("curve '{name}' points must be sorted by rising temperature"));
            }
            if !(0.0..=30.0).contains(&curve.hysteresis) {
                return Err(format!("curve '{name}' hysteresis must be within 0..=30"));
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
                return Err(format!("fan '{fan}' needs 0 <= min_duty <= max_duty <= 1"));
            }
            if !(rule.ramp > 0.0 && rule.ramp.is_finite()) {
                return Err(format!("fan '{fan}' ramp must be a finite number above 0"));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct SensorStatus {
    pub id: String,
    pub name: String,
    pub value: Option<f32>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
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

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Status {
    pub sensors: Vec<SensorStatus>,
    pub fans: Vec<FanStatus>,
}

struct Slot {
    fan: Box<dyn Fan>,
    duty: Option<f32>,
    touched: bool,
}

pub struct Controller {
    backends: Vec<Box<dyn FanBackend>>,
    sensors: Vec<Box<dyn Sensor>>,
    fans: Vec<Slot>,
    config: FanConfig,
    /// Per curve, the temperature that set its current duty.
    anchors: Vec<Option<f32>>,
}

impl Controller {
    /// An invalid `config` is replaced by an empty one, which leaves every fan alone.
    pub fn new(mut backends: Vec<Box<dyn FanBackend>>, config: FanConfig) -> Self {
        let mut sensors = Vec::new();
        let mut fans = Vec::new();
        for backend in &mut backends {
            sensors.extend(backend.sensors());
            fans.extend(backend.fans().into_iter().map(|fan| Slot {
                fan,
                duty: None,
                touched: false,
            }));
        }
        let config = if config.validate().is_ok() {
            config
        } else {
            FanConfig::default()
        };
        let anchors = vec![None; config.curves.len()];
        Controller {
            backends,
            sensors,
            fans,
            config,
            anchors,
        }
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

    /// Reads sensors, evaluates curves and applies duties. `dt` is seconds since the last tick.
    pub fn tick(&mut self, dt: f32) -> Status {
        let dt = if dt.is_finite() && dt > 0.0 { dt } else { 0.0 };
        for backend in &mut self.backends {
            backend.advance(dt);
        }

        let sensors: Vec<SensorStatus> = self
            .sensors
            .iter_mut()
            .map(|sensor| SensorStatus {
                id: sensor.id().to_string(),
                name: sensor.name().to_string(),
                value: sensor.read().filter(|value| value.is_finite()),
            })
            .collect();

        // Per curve: the duty it asks for, or None when no sensor could be read.
        let mut wanted = Vec::with_capacity(self.config.curves.len());
        for (curve, anchor) in self.config.curves.iter().zip(&mut self.anchors) {
            let hottest = sensors
                .iter()
                .filter(|sensor| curve.sensors.contains(&sensor.id))
                .filter_map(|sensor| sensor.value)
                .reduce(f32::max);
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
                    slot.touched = true;
                    match slot.fan.set_duty(duty) {
                        Ok(()) => slot.duty = Some(duty),
                        Err(error) => status.error = Some(error),
                    }
                    status.curve = Some(rule.curve.clone());
                }
                // The rule was removed, so the fan goes back to the firmware.
                None if slot.touched => match slot.fan.release() {
                    Ok(()) => {
                        slot.touched = false;
                        slot.duty = None;
                    }
                    Err(error) => status.error = Some(error),
                },
                None => {}
            }
            status.duty = slot.duty;
            status.rpm = slot.fan.rpm();
            fans.push(status);
        }

        Status { sensors, fans }
    }

    /// Hands every fan back to the firmware. Also runs on drop.
    pub fn release_all(&mut self) {
        for slot in &mut self.fans {
            // A fan that refuses stays marked, so the next call tries again.
            if slot.touched && slot.fan.release().is_ok() {
                slot.touched = false;
                slot.duty = None;
            }
        }
        self.anchors.fill(None);
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        self.release_all();
    }
}
