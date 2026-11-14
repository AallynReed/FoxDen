use std::collections::HashMap;

use crate::{Backend, Color, Device, Effect, Inputs};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct Frame {
    pub device: String,
    pub colors: Vec<Color>,
}

struct Placed {
    /// `rect` holds the effective placement.
    device: Device,
    /// The rect the backend reported.
    home: [f32; 4],
}

struct Slot {
    label: String,
    backend: Box<dyn Backend>,
    devices: Vec<Placed>,
    error: Option<String>,
}

#[derive(Clone, Copy)]
struct Tuning {
    enabled: bool,
    brightness: f32,
    rect: Option<[f32; 4]>,
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning { enabled: true, brightness: 1.0, rect: None }
    }
}

pub struct Engine {
    slots: Vec<Slot>,
    tuning: HashMap<String, Tuning>,
    failed: bool,
    pub effect: Effect,
    pub brightness: f32,
    /// Every LED is black while set.
    pub paused: bool,
}

impl Engine {
    pub fn new(backends: Vec<Box<dyn Backend>>) -> Self {
        let mut engine = Engine {
            slots: Vec::new(),
            tuning: HashMap::new(),
            failed: false,
            effect: Effect::default(),
            brightness: 1.0,
            paused: false,
        };
        engine.set_backends(backends.into_iter().map(|b| (b.name().to_string(), b)).collect());
        engine
    }

    /// Replaces every backend. The label groups backends in `status`.
    pub fn set_backends(&mut self, backends: Vec<(String, Box<dyn Backend>)>) {
        self.slots = backends
            .into_iter()
            .map(|(label, backend)| {
                let devices = backend
                    .devices()
                    .into_iter()
                    .map(|device| Placed { home: device.rect, device })
                    .collect();
                Slot { label, backend, devices, error: None }
            })
            .collect();
        self.layout();
    }

    /// Switch, brightness and canvas placement of one device. `rect: None` means the default spot.
    pub fn set_device(&mut self, id: &str, enabled: bool, brightness: f32, rect: Option<[f32; 4]>) {
        let was_enabled = self.tuning.get(id).is_none_or(|t| t.enabled);
        let tuning = Tuning { enabled, brightness: brightness.clamp(0.0, 1.0), rect };
        self.tuning.insert(id.to_string(), tuning);
        self.layout();
        if was_enabled && !enabled {
            self.black(Some(id));
        }
    }

    pub fn clear_rects(&mut self) {
        for tuning in self.tuning.values_mut() {
            tuning.rect = None;
        }
        self.layout();
    }

    /// Devices with their effective canvas rects.
    pub fn devices(&self) -> Vec<Device> {
        self.views().map(|(_, device, _, _)| device.clone()).collect()
    }

    /// Backend label, device, enabled and brightness for every device.
    pub fn views(&self) -> impl Iterator<Item = (&str, &Device, bool, f32)> {
        self.slots.iter().flat_map(move |slot| {
            slot.devices.iter().map(move |p| {
                let tuning = self.tuning.get(&p.device.id).copied().unwrap_or_default();
                (slot.label.as_str(), &p.device, tuning.enabled, tuning.brightness)
            })
        })
    }

    /// Label, device count and push error of every backend.
    pub fn status(&self) -> Vec<(String, usize, Option<String>)> {
        self.slots.iter().map(|s| (s.label.clone(), s.devices.len(), s.error.clone())).collect()
    }

    /// True once after a backend started failing.
    pub fn take_failed(&mut self) -> bool {
        std::mem::take(&mut self.failed)
    }

    /// Sends one black frame to every enabled device.
    pub fn blackout(&mut self) {
        self.black(None);
    }

    /// Renders the effect at time `t`, pushes it to every device and returns what was sent.
    pub fn render(&mut self, t: f32, inputs: &Inputs) -> Vec<Frame> {
        let mut frames = Vec::new();
        for slot in &mut self.slots {
            for p in &slot.devices {
                let device = &p.device;
                let tuning = self.tuning.get(&device.id).copied().unwrap_or_default();
                let lit = tuning.enabled && !self.paused;
                let k = if lit { self.brightness * tuning.brightness } else { 0.0 };
                let colors: Vec<Color> = if k > 0.0 {
                    device
                        .canvas_positions()
                        .map(|(x, y)| self.effect.sample(x, y, t, inputs).scale(k))
                        .collect()
                } else {
                    vec![Color::BLACK; device.leds.len()]
                };
                let pushed = if tuning.enabled && slot.error.is_none() {
                    slot.backend.push(&device.id, &colors)
                } else {
                    Ok(())
                };
                if let Err(e) = pushed {
                    slot.error = Some(format!("{}: {e}", device.name));
                    self.failed = true;
                }
                frames.push(Frame { device: device.id.clone(), colors });
            }
        }
        frames
    }

    fn black(&mut self, only: Option<&str>) {
        for slot in self.slots.iter_mut().filter(|s| s.error.is_none()) {
            for p in &slot.devices {
                let id = p.device.id.as_str();
                let enabled = self.tuning.get(id).is_none_or(|t| t.enabled);
                if only.map_or(enabled, |o| o == id) {
                    let _ = slot.backend.push(id, &vec![Color::BLACK; p.device.leds.len()]);
                }
            }
        }
    }

    /// Saved rects win, virtual devices keep their own, everything else gets a free spot.
    fn layout(&mut self) {
        let mut taken = Vec::new();
        let mut unplaced = Vec::new();
        for (s, slot) in self.slots.iter_mut().enumerate() {
            for (d, p) in slot.devices.iter_mut().enumerate() {
                match self.tuning.get(&p.device.id).and_then(|t| t.rect) {
                    Some(rect) => p.device.rect = rect,
                    None if slot.label == "virtual" => p.device.rect = p.home,
                    None => {
                        unplaced.push((s, d));
                        continue;
                    }
                }
                taken.push(p.device.rect);
            }
        }
        for (s, d) in unplaced {
            let device = &mut self.slots[s].devices[d].device;
            device.rect = free_spot(device.grid, &taken);
            taken.push(device.rect);
        }
    }
}

/// First gap big enough for a device of this grid, scanning the canvas row by row.
fn free_spot(grid: [u32; 2], taken: &[[f32; 4]]) -> [f32; 4] {
    const STEP: f32 = 0.02;
    const GAP: f32 = 0.01;
    let w = (grid[0] as f32 * 0.02).clamp(0.04, 0.5);
    let h = (grid[1] as f32 * 0.04).clamp(0.04, 0.3);
    for row in 1..50 {
        for col in 1..50 {
            let (x, y) = (col as f32 * STEP, row as f32 * STEP);
            let padded = [x - GAP, y - GAP, w + 2.0 * GAP, h + 2.0 * GAP];
            if x + w <= 1.0 - STEP && y + h <= 1.0 - STEP && !taken.iter().any(|t| overlaps(padded, *t)) {
                return [x, y, w, h];
            }
        }
    }
    [STEP, STEP, w, h]
}

fn overlaps(a: [f32; 4], b: [f32; 4]) -> bool {
    a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DeviceKind, VirtualBackend};
    use std::sync::{Arc, Mutex};

    /// Three devices that all claim the whole canvas; counts pushes and can be made to fail.
    struct Fake {
        pushes: Arc<Mutex<Vec<String>>>,
        fail: bool,
    }

    impl Backend for Fake {
        fn name(&self) -> &str {
            "fake"
        }

        fn devices(&self) -> Vec<Device> {
            ["a", "b", "c"]
                .iter()
                .map(|id| Device::grid(id, id, DeviceKind::Strip, [0.0, 0.0, 1.0, 1.0], 10, 1))
                .collect()
        }

        fn push(&mut self, device_id: &str, _colors: &[Color]) -> Result<(), String> {
            self.pushes.lock().unwrap().push(device_id.to_string());
            if self.fail { Err("gone".into()) } else { Ok(()) }
        }
    }

    fn fake(fail: bool) -> (Engine, Arc<Mutex<Vec<String>>>) {
        let pushes = Arc::new(Mutex::new(Vec::new()));
        let backend = Fake { pushes: pushes.clone(), fail };
        (Engine::new(vec![Box::new(VirtualBackend), Box::new(backend)]), pushes)
    }

    fn all_black(frame: &Frame) -> bool {
        frame.colors.iter().all(|c| *c == Color::BLACK)
    }

    #[test]
    fn one_frame_per_device_one_color_per_led() {
        let mut engine = Engine::new(vec![Box::new(VirtualBackend)]);
        let devices = engine.devices();
        let frames = engine.render(1.0, &Inputs::default());
        assert_eq!(frames.len(), devices.len());
        for (frame, device) in frames.iter().zip(&devices) {
            assert_eq!(frame.device, device.id);
            assert_eq!(frame.colors.len(), device.leds.len());
        }
    }

    #[test]
    fn zero_brightness_and_pause_are_black() {
        let mut engine = Engine::new(vec![Box::new(VirtualBackend)]);
        engine.brightness = 0.0;
        assert!(engine.render(3.0, &Inputs::default()).iter().all(all_black));
        engine.brightness = 1.0;
        engine.paused = true;
        assert!(engine.render(3.0, &Inputs::default()).iter().all(all_black));
    }

    #[test]
    fn default_placement_does_not_overlap() {
        let (engine, _) = fake(false);
        let devices = engine.devices();
        for (i, a) in devices.iter().enumerate() {
            let [x, y, w, h] = a.rect;
            assert!(x >= 0.0 && y >= 0.0 && x + w <= 1.0 && y + h <= 1.0, "{} is off the canvas", a.id);
            for b in &devices[i + 1..] {
                assert!(!overlaps(a.rect, b.rect), "{} overlaps {}", a.id, b.id);
            }
        }
        let board = devices.iter().find(|d| d.id == "virtual:board").unwrap();
        assert_eq!(board.rect, VirtualBackend.devices()[0].rect);
    }

    #[test]
    fn saved_rect_wins_and_reset_restores_the_default() {
        let (mut engine, _) = fake(false);
        let rect_of = |e: &Engine, id: &str| e.devices().into_iter().find(|d| d.id == id).unwrap().rect;
        let default = rect_of(&engine, "a");
        engine.set_device("a", true, 1.0, Some([0.1, 0.2, 0.3, 0.4]));
        assert_eq!(rect_of(&engine, "a"), [0.1, 0.2, 0.3, 0.4]);
        engine.clear_rects();
        assert_eq!(rect_of(&engine, "a"), default);
    }

    #[test]
    fn disabled_device_goes_black_once_then_is_left_alone() {
        let (mut engine, pushes) = fake(false);
        engine.set_device("a", false, 1.0, None);
        assert_eq!(*pushes.lock().unwrap(), ["a"]);
        let frames = engine.render(1.0, &Inputs::default());
        assert!(all_black(frames.iter().find(|f| f.device == "a").unwrap()));
        assert_eq!(*pushes.lock().unwrap(), ["a", "b", "c"]);
    }

    #[test]
    fn failing_backend_is_reported_and_skipped() {
        let (mut engine, pushes) = fake(true);
        assert_eq!(engine.render(1.0, &Inputs::default()).len(), 9);
        assert!(engine.take_failed());
        assert!(!engine.take_failed());
        engine.render(2.0, &Inputs::default());
        assert_eq!(pushes.lock().unwrap().len(), 1);
        let status = engine.status();
        assert_eq!(status[0], ("virtual".to_string(), 6, None));
        assert!(status[1].2.as_deref().unwrap().contains("gone"));
    }
}
