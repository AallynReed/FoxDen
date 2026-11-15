//! Turns one USB fan controller or AIO into FoxRGB fans and temperature sensors.

use foxrgb_fan::{Fan, FanBackend, Sensor};
use foxrgb_hid::port::{Device, Hold};
use foxrgb_hid::registry::Info;
use std::sync::{Arc, Mutex};

/// Lowest duty ever sent to a pump: below it most AIO pumps stall or report a fault.
pub const PUMP_MIN: f32 = 0.5;

#[derive(Clone, Debug)]
pub struct Channel {
    pub name: String,
    pub pump: bool,
    /// Duty below this is raised to it (a pump always gets at least `PUMP_MIN`).
    pub min_duty: f32,
}

impl Channel {
    pub fn fan(name: impl Into<String>) -> Self {
        Channel {
            name: name.into(),
            pump: false,
            min_duty: 0.0,
        }
    }
    pub fn pump(name: impl Into<String>) -> Self {
        Channel {
            name: name.into(),
            pump: true,
            min_duty: PUMP_MIN,
        }
    }
}

/// One controller with numbered channels. Every call takes the hub's lock, so a driver can keep
/// one device handle for all its fans.
pub trait FanHub: Send {
    /// Stable id, e.g. `fan:arctic-3904f001-<serial>`.
    fn id(&self) -> String;
    fn name(&self) -> String;
    fn channels(&self) -> Vec<Channel>;
    /// Temperature sensors the controller reports (coolant, probes), by name.
    fn sensors(&self) -> Vec<String> {
        Vec::new()
    }
    fn rpm(&mut self, channel: usize) -> Option<u32>;
    /// `duty` is 0..=1, already clamped to the channel's minimum.
    fn set_duty(&mut self, channel: usize, duty: f32) -> Result<(), String>;
    /// Hands the channel back to the controller's own curve or default.
    fn release(&mut self, channel: usize) -> Result<(), String>;
    fn temperature(&mut self, _sensor: usize) -> Option<f32> {
        None
    }
}

type Shared = Arc<Mutex<Box<dyn FanHub>>>;

pub struct HubBackend {
    hub: Shared,
    name: String,
    device: Option<Arc<Device>>,
}

pub fn backend(hub: impl FanHub + 'static) -> Box<dyn FanBackend> {
    with_device(None, hub)
}

/// A hub on `info`'s device, holding that device's lock for every call when it is shared.
pub fn backend_on(info: &Info, hub: impl FanHub + 'static) -> Box<dyn FanBackend> {
    with_device(foxrgb_hid::port::device_for(info), hub)
}

pub fn with_device(device: Option<Arc<Device>>, hub: impl FanHub + 'static) -> Box<dyn FanBackend> {
    let name = hub.name();
    Box::new(HubBackend {
        hub: Arc::new(Mutex::new(Box::new(hub))),
        name,
        device,
    })
}

fn hold(device: &Option<Arc<Device>>) -> Option<Hold> {
    device.as_ref().map(|d| d.hold())
}

impl FanBackend for HubBackend {
    fn name(&self) -> &str {
        &self.name
    }
    fn sensors(&mut self) -> Vec<Box<dyn Sensor>> {
        let hub = self.hub.lock().unwrap();
        let base = hub.id();
        hub.sensors()
            .into_iter()
            .enumerate()
            .map(|(i, n)| {
                Box::new(HubSensor {
                    hub: self.hub.clone(),
                    device: self.device.clone(),
                    index: i,
                    id: format!("{base}:t{i}"),
                    name: format!("{} {n}", hub.name()),
                }) as Box<dyn Sensor>
            })
            .collect()
    }
    fn fans(&mut self) -> Vec<Box<dyn Fan>> {
        let hub = self.hub.lock().unwrap();
        let base = hub.id();
        hub.channels()
            .into_iter()
            .enumerate()
            .map(|(i, c)| {
                Box::new(HubFan {
                    hub: self.hub.clone(),
                    device: self.device.clone(),
                    index: i,
                    id: format!("{base}:{i}"),
                    name: format!("{} {}", hub.name(), c.name),
                    min: if c.pump {
                        c.min_duty.max(PUMP_MIN)
                    } else {
                        c.min_duty
                    },
                }) as Box<dyn Fan>
            })
            .collect()
    }
}

struct HubFan {
    hub: Shared,
    device: Option<Arc<Device>>,
    index: usize,
    id: String,
    name: String,
    min: f32,
}

impl Fan for HubFan {
    fn id(&self) -> &str {
        &self.id
    }
    fn name(&self) -> &str {
        &self.name
    }
    fn rpm(&mut self) -> Option<u32> {
        let mut hub = self.hub.lock().ok()?;
        let _hold = hold(&self.device);
        hub.rpm(self.index)
    }
    fn set_duty(&mut self, duty: f32) -> Result<(), String> {
        let d = duty.clamp(0.0, 1.0).max(self.min);
        let mut hub = self.hub.lock().map_err(|e| e.to_string())?;
        let _hold = hold(&self.device);
        hub.set_duty(self.index, d)
    }
    fn release(&mut self) -> Result<(), String> {
        let mut hub = self.hub.lock().map_err(|e| e.to_string())?;
        let _hold = hold(&self.device);
        hub.release(self.index)
    }
}

struct HubSensor {
    hub: Shared,
    device: Option<Arc<Device>>,
    index: usize,
    id: String,
    name: String,
}

impl Sensor for HubSensor {
    fn id(&self) -> &str {
        &self.id
    }
    fn name(&self) -> &str {
        &self.name
    }
    fn read(&mut self) -> Option<f32> {
        let mut hub = self.hub.lock().ok()?;
        let _hold = hold(&self.device);
        hub.temperature(self.index)
    }
}

/// `duty` 0..=1 as a whole percent.
pub fn percent(duty: f32) -> u8 {
    (duty.clamp(0.0, 1.0) * 100.0).round() as u8
}

/// `duty` 0..=1 as a byte 0..=255.
pub fn byte(duty: f32) -> u8 {
    (duty.clamp(0.0, 1.0) * 255.0).round() as u8
}
