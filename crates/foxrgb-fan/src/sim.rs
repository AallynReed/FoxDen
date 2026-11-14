use std::sync::{Arc, Mutex, MutexGuard};

use crate::{Fan, FanBackend, Sensor};

const AMBIENT: f32 = 25.0;
/// Duty the simulated firmware runs a fan at when nobody controls it.
const FIRMWARE_DUTY: f32 = 0.4;
/// Seconds for a swinging load to go from idle to busy and back.
const SWING: f32 = 90.0;

/// Id, name and degrees above ambient at full load with stopped fans.
const SENSORS: [(&str, &str, f32); 2] = [
    ("sim/cpu", "Simulated CPU", 70.0),
    ("sim/gpu", "Simulated GPU", 60.0),
];
/// Id, name and rpm at full duty.
const FANS: [(&str, &str, f32); 3] = [
    ("sim/cpu_fan", "Simulated CPU fan", 1800.0),
    ("sim/case_fan", "Simulated case fan", 1200.0),
    ("sim/gpu_fan", "Simulated GPU fan", 3000.0),
];

struct Model {
    load: f32,
    temps: [f32; 2],
    failed: [bool; 2],
    /// `None` while the firmware is in control.
    duties: [Option<f32>; 3],
    rpms: [f32; 3],
    /// Seconds into the swing, while the load moves by itself.
    swing: Option<f32>,
}

impl Default for Model {
    fn default() -> Self {
        let mut model = Model {
            load: 0.1,
            temps: [AMBIENT; 2],
            failed: [false; 2],
            duties: [None; 3],
            rpms: [0.0; 3],
            swing: None,
        };
        // Start settled, as a machine that has idled for a while.
        model.step(1000.0);
        model
    }
}

impl Model {
    fn step(&mut self, dt: f32) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        if let Some(at) = &mut self.swing {
            *at = (*at + dt) % SWING;
            self.load = 0.5 - 0.45 * (*at / SWING * std::f32::consts::TAU).cos();
        }
        let duty = self.duties.map(|duty| duty.unwrap_or(FIRMWARE_DUTY));
        // The case fan helps both; each part mostly depends on its own fan.
        let airflow = [0.7 * duty[0] + 0.3 * duty[1], 0.7 * duty[2] + 0.3 * duty[1]];
        let heat_lag = 1.0 - (-dt / 8.0).exp();
        for i in 0..SENSORS.len() {
            let settled = AMBIENT + self.load * SENSORS[i].2 * (1.0 - 0.6 * airflow[i]);
            self.temps[i] += (settled - self.temps[i]) * heat_lag;
        }
        let spin_lag = 1.0 - (-dt).exp();
        for i in 0..FANS.len() {
            self.rpms[i] += (duty[i] * FANS[i].2 - self.rpms[i]) * spin_lag;
        }
    }
}

/// Simulated sensors and fans for tests and demos.
///
/// Clones share one model, so keep a clone to change the load while a controller owns the other.
#[derive(Clone, Default)]
pub struct SimBackend {
    model: Arc<Mutex<Model>>,
}

fn lock(model: &Mutex<Model>) -> MutexGuard<'_, Model> {
    model.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl SimBackend {
    /// A backend whose load rises and falls by itself, so that curves have something to follow.
    pub fn swinging() -> Self {
        let sim = SimBackend::default();
        lock(&sim.model).swing = Some(0.0);
        sim
    }

    /// 0 is idle, 1 is full load. Ends a swing.
    pub fn set_load(&self, load: f32) {
        if load.is_finite() {
            let mut model = lock(&self.model);
            model.load = load.clamp(0.0, 1.0);
            model.swing = None;
        }
    }

    /// Makes a sensor stop or resume reading. Unknown ids are ignored.
    pub fn fail_sensor(&self, id: &str, failed: bool) {
        if let Some(index) = SENSORS.iter().position(|sensor| sensor.0 == id) {
            lock(&self.model).failed[index] = failed;
        }
    }

    /// Duty the fan was last told to run at, or `None` while the firmware controls it.
    pub fn fan_duty(&self, id: &str) -> Option<f32> {
        let index = FANS.iter().position(|fan| fan.0 == id)?;
        lock(&self.model).duties[index]
    }

    /// Moves the model forward by `dt` seconds.
    pub fn step(&self, dt: f32) {
        lock(&self.model).step(dt);
    }
}

impl FanBackend for SimBackend {
    fn name(&self) -> &str {
        "sim"
    }

    fn sensors(&mut self) -> Vec<Box<dyn Sensor>> {
        (0..SENSORS.len())
            .map(|index| {
                Box::new(SimSensor {
                    index,
                    model: self.model.clone(),
                }) as Box<dyn Sensor>
            })
            .collect()
    }

    fn fans(&mut self) -> Vec<Box<dyn Fan>> {
        (0..FANS.len())
            .map(|index| {
                Box::new(SimFan {
                    index,
                    model: self.model.clone(),
                }) as Box<dyn Fan>
            })
            .collect()
    }

    fn advance(&mut self, dt: f32) {
        self.step(dt);
    }
}

struct SimSensor {
    index: usize,
    model: Arc<Mutex<Model>>,
}

impl Sensor for SimSensor {
    fn id(&self) -> &str {
        SENSORS[self.index].0
    }

    fn name(&self) -> &str {
        SENSORS[self.index].1
    }

    fn read(&mut self) -> Option<f32> {
        let model = lock(&self.model);
        (!model.failed[self.index]).then_some(model.temps[self.index])
    }
}

struct SimFan {
    index: usize,
    model: Arc<Mutex<Model>>,
}

impl Fan for SimFan {
    fn id(&self) -> &str {
        FANS[self.index].0
    }

    fn name(&self) -> &str {
        FANS[self.index].1
    }

    fn rpm(&mut self) -> Option<u32> {
        Some(lock(&self.model).rpms[self.index].round() as u32)
    }

    fn set_duty(&mut self, duty: f32) -> Result<(), String> {
        if !duty.is_finite() {
            return Err(format!("{}: duty is not a number", FANS[self.index].0));
        }
        lock(&self.model).duties[self.index] = Some(duty.clamp(0.0, 1.0));
        Ok(())
    }

    fn release(&mut self) -> Result<(), String> {
        lock(&self.model).duties[self.index] = None;
        Ok(())
    }
}
