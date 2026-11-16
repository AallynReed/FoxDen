//! The fan backend every Super I/O and EC family shares, with one cached read per tick.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use foxrgb_fan::{Fan, FanBackend, Sensor};
use foxrgb_pawnio::SharedLock;

/// The pump never goes below this, whatever a curve asks.
pub(crate) const PUMP_FLOOR: f32 = 0.7;
/// Readings stay good for this long, so one tick costs one pass over the chip.
const FRESH: Duration = Duration::from_millis(500);

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Values {
    pub temps: Vec<Option<f32>>,
    pub fans: Vec<Option<f32>>,
}

/// One chip's sensors and controls. Calls arrive with the bus lock held.
pub(crate) trait Monitor: Send {
    /// How many temperatures, fan speeds and controls the chip has.
    fn size(&self) -> (usize, usize, usize);
    fn update(&mut self, values: &mut Values);
    /// `Some` takes the control at that duty (0 to 255), `None` gives it back.
    fn set_control(&mut self, index: usize, value: Option<u8>) -> Result<(), String>;
}

/// What a board calls each input. Indices refer to the chip's own lists.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Layout {
    pub temps: Vec<(String, usize)>,
    pub fans: Vec<(String, usize)>,
    pub controls: Vec<(String, usize)>,
}

impl Layout {
    /// Numbered names for everything the chip has.
    pub fn generic(size: (usize, usize, usize)) -> Layout {
        Layout {
            temps: (0..size.0)
                .map(|i| (format!("Temperature {}", i + 1), i))
                .collect(),
            fans: (0..size.1).map(|i| (format!("Fan {}", i + 1), i)).collect(),
            controls: (0..size.2).map(|i| (format!("Fan {}", i + 1), i)).collect(),
        }
    }
}

struct Shared {
    lock: Option<SharedLock>,
    chip: Box<dyn Monitor>,
    values: Values,
    at: Option<Instant>,
}

impl Shared {
    fn fresh(&mut self) -> &Values {
        if self.at.is_none_or(|at| at.elapsed() > FRESH) {
            let Shared {
                lock, chip, values, ..
            } = self;
            match lock {
                Some(lock) => lock.run(|| chip.update(values)),
                None => chip.update(values),
            }
            self.at = Some(Instant::now());
        }
        &self.values
    }

    fn set(&mut self, index: usize, value: Option<u8>) -> Result<(), String> {
        let Shared { lock, chip, .. } = self;
        match lock {
            Some(lock) => lock.run(|| chip.set_control(index, value)),
            None => chip.set_control(index, value),
        }
    }
}

type Handle = Arc<Mutex<Shared>>;

fn lock(handle: &Handle) -> MutexGuard<'_, Shared> {
    handle.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) struct Board {
    name: String,
    prefix: String,
    layout: Layout,
    shared: Handle,
}

impl Board {
    /// `prefix` starts every id, for example `board/it8688e-2e`.
    pub fn new(
        name: String,
        prefix: String,
        chip: Box<dyn Monitor>,
        layout: Layout,
        lock: Option<SharedLock>,
    ) -> Board {
        let (temps, fans, controls) = chip.size();
        let layout = Layout {
            temps: layout.temps.into_iter().filter(|t| t.1 < temps).collect(),
            fans: layout.fans.into_iter().filter(|f| f.1 < fans).collect(),
            controls: layout
                .controls
                .into_iter()
                .filter(|c| c.1 < controls)
                .collect(),
        };
        let values = Values {
            temps: vec![None; temps],
            fans: vec![None; fans],
        };
        let shared = Shared {
            lock,
            chip,
            values,
            at: None,
        };
        Board {
            name,
            prefix,
            layout,
            shared: Arc::new(Mutex::new(shared)),
        }
    }
}

struct Temp {
    shared: Handle,
    id: String,
    name: String,
    index: usize,
}

impl Sensor for Temp {
    fn id(&self) -> &str {
        &self.id
    }
    fn name(&self) -> &str {
        &self.name
    }
    fn read(&mut self) -> Option<f32> {
        lock(&self.shared).fresh().temps[self.index]
    }
}

struct Header {
    shared: Handle,
    id: String,
    name: String,
    speed: Option<usize>,
    control: Option<usize>,
    taken: bool,
}

impl Fan for Header {
    fn id(&self) -> &str {
        &self.id
    }
    fn name(&self) -> &str {
        &self.name
    }
    fn rpm(&mut self) -> Option<u32> {
        let rpm = lock(&self.shared).fresh().fans[self.speed?]?;
        Some(rpm.round() as u32)
    }
    fn set_duty(&mut self, duty: f32) -> Result<(), String> {
        let index = self.control.ok_or("this header can only be read")?;
        let mut duty = duty.clamp(0.0, 1.0);
        if self.name.to_lowercase().contains("pump") {
            duty = duty.max(PUMP_FLOOR);
        }
        let value = (duty * 255.0).round() as u8;
        lock(&self.shared).set(index, Some(value))?;
        self.taken = true;
        Ok(())
    }
    fn release(&mut self) -> Result<(), String> {
        match self.control {
            Some(index) if self.taken => {
                lock(&self.shared).set(index, None)?;
                self.taken = false;
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

impl FanBackend for Board {
    fn name(&self) -> &str {
        &self.name
    }
    fn sensors(&mut self) -> Vec<Box<dyn Sensor>> {
        self.layout
            .temps
            .iter()
            .map(|(name, index)| {
                Box::new(Temp {
                    shared: self.shared.clone(),
                    id: format!("{}/temp{index}", self.prefix),
                    name: name.clone(),
                    index: *index,
                }) as Box<dyn Sensor>
            })
            .collect()
    }
    fn fans(&mut self) -> Vec<Box<dyn Fan>> {
        let mut indices: Vec<usize> = self.layout.fans.iter().map(|f| f.1).collect();
        indices.extend(self.layout.controls.iter().map(|c| c.1));
        let mut seen = Vec::new();
        indices.retain(|i| {
            let new = !seen.contains(i);
            seen.push(*i);
            new
        });
        indices
            .into_iter()
            .map(|index| {
                let speed = self.layout.fans.iter().find(|f| f.1 == index);
                let control = self.layout.controls.iter().find(|c| c.1 == index);
                let name = speed.or(control).map(|n| n.0.clone()).unwrap_or_default();
                Box::new(Header {
                    shared: self.shared.clone(),
                    id: format!("{}/fan{index}", self.prefix),
                    name,
                    speed: speed.map(|_| index),
                    control: control.map(|_| index),
                    taken: false,
                }) as Box<dyn Fan>
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Probe {
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl Monitor for Probe {
        fn size(&self) -> (usize, usize, usize) {
            (2, 3, 2)
        }
        fn update(&mut self, values: &mut Values) {
            self.calls.lock().unwrap().push("update".into());
            values.temps = vec![Some(40.0), Some(50.0)];
            values.fans = vec![Some(1200.4), None, Some(800.0)];
        }
        fn set_control(&mut self, index: usize, value: Option<u8>) -> Result<(), String> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("{index} {value:?}"));
            Ok(())
        }
    }

    #[test]
    fn pumps_keep_a_floor() {
        let probe = Probe::default();
        let calls = probe.calls.clone();
        let layout = Layout {
            controls: vec![("AIO pump".into(), 0), ("CPU fan".into(), 1)],
            ..Layout::default()
        };
        let mut board = Board::new("x".into(), "board/x".into(), Box::new(probe), layout, None);
        let mut fans = board.fans();
        fans[0].set_duty(0.0).unwrap();
        fans[1].set_duty(0.0).unwrap();
        assert_eq!(*calls.lock().unwrap(), ["0 Some(179)", "1 Some(0)"]);
    }

    #[test]
    fn headers_take_names_speeds_and_controls_from_the_layout() {
        let probe = Probe::default();
        let calls = probe.calls.clone();
        let layout = Layout {
            temps: vec![("CPU".into(), 1), ("Gone".into(), 7)],
            fans: vec![("CPU fan".into(), 0), ("Pump".into(), 2)],
            controls: vec![("CPU fan".into(), 0), ("Extra".into(), 1)],
        };
        let mut board = Board::new(
            "x".into(),
            "board/x-2e".into(),
            Box::new(probe),
            layout,
            None,
        );
        let mut sensors = board.sensors();
        assert_eq!(sensors.len(), 1);
        assert_eq!(sensors[0].id(), "board/x-2e/temp1");
        assert_eq!(sensors[0].read(), Some(50.0));
        let mut fans = board.fans();
        let ids: Vec<&str> = fans.iter().map(|f| f.id()).collect();
        assert_eq!(
            ids,
            ["board/x-2e/fan0", "board/x-2e/fan2", "board/x-2e/fan1"]
        );
        assert_eq!(fans[0].rpm(), Some(1200));
        assert_eq!(fans[2].rpm(), None);
        assert!(fans[1].set_duty(0.5).is_err());
        fans[1].release().unwrap();
        fans[0].set_duty(0.5).unwrap();
        fans[0].release().unwrap();
        fans[0].release().unwrap();
        // one update for all reads within the fresh window
        assert_eq!(*calls.lock().unwrap(), ["update", "0 Some(128)", "0 None"]);
    }
}
