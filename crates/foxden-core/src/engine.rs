use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::{
    Apps, Backend, Batteries, Color, Device, Effect, Hardware, HardwareSetting, Inputs, Layer,
    Spot, paint, spots,
};
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Frame {
    pub device: String,
    pub colors: Vec<Color>,
}

struct Placed {
    /// `rect` holds the effective placement.
    device: Device,
    /// The rect the backend reported.
    home: [f32; 4],
    /// The key or zone under each LED, for key colour layers.
    spots: Vec<Spot>,
    hardware: Option<Hardware>,
    /// Running a hardware effect, so it gets no frames.
    held: bool,
}

/// Newest colours for each device of one backend; `None` where nothing is waiting.
type Batch = Vec<Option<Vec<Color>>>;

#[derive(Default)]
struct Mail {
    batch: Batch,
    /// The pusher is in the middle of a batch.
    busy: bool,
    closed: bool,
    error: Option<String>,
    /// Hardware effects to start or stop, by device index, before any more colours.
    switches: Vec<(usize, Option<HardwareSetting>)>,
    refused: Vec<(usize, String)>,
}

/// What the engine shares with the thread that owns a backend.
#[derive(Default)]
struct Link {
    mail: Mutex<Mail>,
    wake: Condvar,
}

impl Link {
    fn mail(&self) -> MutexGuard<'_, Mail> {
        self.mail.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Leaves colours for the pusher. A device it has not reached yet simply gets the newer ones.
    fn post(&self, batch: Batch) {
        let mut mail = self.mail();
        if mail.closed || mail.error.is_some() {
            return;
        }
        if mail.batch.len() == batch.len() {
            for (waiting, new) in mail.batch.iter_mut().zip(batch) {
                if new.is_some() {
                    *waiting = new;
                }
            }
        } else {
            mail.batch = batch;
        }
        self.wake.notify_all();
    }

    fn switch(&self, device: usize, setting: Option<HardwareSetting>) {
        self.mail().switches.push((device, setting));
        self.wake.notify_all();
    }
}

/// How long `set_hardware` waits for the device to answer.
const SWITCH_WAIT: Duration = Duration::from_secs(2);

/// The longest a device goes without its colours being sent again, even when they have not changed.
const RESEND: Duration = Duration::from_secs(1);

/// Owns a backend on its own thread, so a slow or dead device never holds up the caller.
fn spawn_pusher(
    mut backend: Box<dyn Backend>,
    devices: Vec<(String, String)>,
    link: Arc<Link>,
    failed: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        // Last colours sent per device: identical frames are skipped, but resent at least every
        // `RESEND` so devices that fall back to their own lighting without a steady signal stay put.
        let mut sent: Vec<Option<(Vec<Color>, Instant)>> = vec![None; devices.len()];
        let mut held = vec![false; devices.len()];
        loop {
            let (batch, switches) = {
                let mut mail = link.mail();
                mail.busy = false;
                link.wake.notify_all();
                while !mail.closed
                    && mail.switches.is_empty()
                    && mail.batch.iter().all(Option::is_none)
                {
                    mail = link.wake.wait(mail).unwrap_or_else(|e| e.into_inner());
                }
                if mail.closed {
                    return;
                }
                mail.busy = true;
                (
                    std::mem::take(&mut mail.batch),
                    std::mem::take(&mut mail.switches),
                )
            };
            for (i, setting) in switches {
                let Some((id, name)) = devices.get(i) else {
                    continue;
                };
                sent[i] = None;
                held[i] = setting.is_some();
                if let Err(e) = backend.set_hardware(id, setting.as_ref()) {
                    held[i] = false;
                    link.mail().refused.push((i, format!("{name}: {e}")));
                }
            }
            let now = Instant::now();
            let mut any = false;
            let pushed = devices
                .iter()
                .zip(batch)
                .zip(sent.iter_mut())
                .zip(&held)
                .filter(|(_, held)| !**held)
                .filter_map(|(((device, colors), last), _)| Some((device, colors?, last)))
                .filter(|(_, colors, last)| {
                    last.as_ref()
                        .is_none_or(|(c, at)| c != colors || now.duration_since(*at) >= RESEND)
                })
                .try_for_each(|((id, name), colors, last)| {
                    any = true;
                    backend
                        .push(id, &colors)
                        .map_err(|e| format!("{name}: {e}"))?;
                    *last = Some((colors, now));
                    Ok(())
                })
                .and_then(|()| if any { backend.flush() } else { Ok(()) });
            if let Err(e) = pushed {
                let mut mail = link.mail();
                mail.error = Some(e);
                mail.busy = false;
                failed.store(true, Ordering::Relaxed);
                link.wake.notify_all();
                return;
            }
        }
    })
}

struct Slot {
    label: String,
    devices: Vec<Placed>,
    link: Arc<Link>,
    pusher: Option<JoinHandle<()>>,
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.link.mail().closed = true;
        self.link.wake.notify_all();
    }
}

/// Backends taken out of the engine. Each lets go of its devices once its last push returns.
pub struct Removed(Vec<JoinHandle<()>>);

impl Removed {
    /// Waits for that, for at most `limit`. Not to be called while holding a lock others need.
    pub fn wait(self, limit: Duration) {
        let deadline = Instant::now() + limit;
        while self.0.iter().any(|pusher| !pusher.is_finished()) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
    }
}

#[derive(Clone, Copy)]
struct Tuning {
    enabled: bool,
    brightness: f32,
    rect: Option<[f32; 4]>,
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning {
            enabled: true,
            brightness: 1.0,
            rect: None,
        }
    }
}

pub struct Engine {
    slots: Vec<Slot>,
    /// Labels in the order their devices are listed and laid out.
    order: Vec<String>,
    tuning: HashMap<String, Tuning>,
    failed: Arc<AtomicBool>,
    pub effect: Effect,
    /// Devices with an effect of their own instead of `effect`.
    pub overrides: HashMap<String, Effect>,
    /// Devices told to run an effect of their own.
    hardware: HashMap<String, HardwareSetting>,
    /// Drawn over the effect, bottom first.
    pub layers: Vec<Layer>,
    /// What layers tied to apps check against.
    pub apps: Apps,
    /// What layers tied to batteries check against.
    pub batteries: Batteries,
    pub brightness: f32,
    /// Every LED is black while set.
    pub paused: bool,
}

impl Engine {
    pub fn new(backends: Vec<Box<dyn Backend>>) -> Self {
        let mut engine = Engine {
            slots: Vec::new(),
            order: Vec::new(),
            tuning: HashMap::new(),
            failed: Arc::default(),
            effect: Effect::default(),
            overrides: HashMap::new(),
            hardware: HashMap::new(),
            layers: Vec::new(),
            apps: Apps::default(),
            batteries: Batteries::new(),
            brightness: 1.0,
            paused: false,
        };
        for backend in backends {
            let label = backend.name().to_string();
            engine.add(&label, vec![backend]);
        }
        engine
    }

    /// Adds backends under `label`, which groups them in `status`. Labels keep the order they first came in.
    pub fn add(&mut self, label: &str, backends: Vec<Box<dyn Backend>>) {
        if !self.order.iter().any(|known| known == label) {
            self.order.push(label.to_string());
        }
        for backend in backends {
            let devices: Vec<Placed> = backend
                .devices()
                .into_iter()
                .map(|device| Placed {
                    home: device.rect,
                    hardware: backend.hardware(&device.id),
                    device,
                    spots: Vec::new(),
                    held: false,
                })
                .collect();
            let names = devices
                .iter()
                .map(|p| (p.device.id.clone(), p.device.name.clone()))
                .collect();
            let link = Arc::new(Link::default());
            let pusher = spawn_pusher(backend, names, link.clone(), self.failed.clone());
            let mut slot = Slot {
                label: label.to_string(),
                devices,
                link,
                pusher: Some(pusher),
            };
            for (i, p) in slot.devices.iter_mut().enumerate() {
                if let Some(setting) = self.runnable(p) {
                    slot.link.switch(i, Some(setting));
                    p.held = true;
                }
            }
            self.slots.push(slot);
        }
        let order = &self.order;
        self.slots
            .sort_by_key(|slot| order.iter().position(|known| *known == slot.label));
        self.layout();
    }

    /// Takes out every backend under `label`. Frames it has not sent yet are dropped.
    pub fn remove(&mut self, label: &str) -> Removed {
        let (gone, kept): (Vec<Slot>, Vec<Slot>) = std::mem::take(&mut self.slots)
            .into_iter()
            .partition(|slot| slot.label == label);
        self.slots = kept;
        self.layout();
        Removed(
            gone.into_iter()
                .filter_map(|mut slot| slot.pusher.take())
                .collect(),
        )
    }

    /// Switch, brightness and canvas placement of one device. `rect: None` means the default spot.
    pub fn set_device(&mut self, id: &str, enabled: bool, brightness: f32, rect: Option<[f32; 4]>) {
        let was_enabled = self.tuning.get(id).is_none_or(|t| t.enabled);
        let tuning = Tuning {
            enabled,
            brightness: brightness.clamp(0.0, 1.0),
            rect,
        };
        self.tuning.insert(id.to_string(), tuning);
        self.layout();
        if was_enabled != enabled {
            self.rehold(id);
        }
        if was_enabled && !enabled {
            self.black(Some(id));
        }
    }

    /// Effects the device can run on its own.
    pub fn hardware(&self, id: &str) -> Option<&Hardware> {
        let (s, d) = self.find(id)?;
        self.slots[s].devices[d].hardware.as_ref()
    }

    pub fn hardware_setting(&self, id: &str) -> Option<&HardwareSetting> {
        self.hardware.get(id)
    }

    /// Has the device run one of its own effects, or with `None` hands it back to FoxDen's.
    /// A device not connected yet starts the effect once it is.
    pub fn set_hardware(
        &mut self,
        id: &str,
        setting: Option<HardwareSetting>,
    ) -> Result<(), String> {
        if let (Some(setting), Some((s, d))) = (&setting, self.find(id)) {
            let options = self.slots[s].devices[d].hardware.as_ref();
            options
                .ok_or("the device has no effects of its own")?
                .fit(setting)?;
        }
        match setting {
            Some(setting) => self.hardware.insert(id.to_string(), setting),
            None => self.hardware.remove(id),
        };
        let Some((s, d)) = self.find(id) else {
            return Ok(());
        };
        let link = self.slots[s].link.clone();
        link.mail().refused.clear();
        self.rehold(id);
        let deadline = Instant::now() + SWITCH_WAIT;
        let mut mail = link.mail();
        while mail.error.is_none() && !mail.closed && (mail.busy || !mail.switches.is_empty()) {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                break;
            };
            mail = link
                .wake
                .wait_timeout(mail, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        let refused = mail
            .refused
            .iter()
            .find(|(i, _)| *i == d)
            .map(|(_, e)| e.clone());
        drop(mail);
        match refused {
            Some(e) => {
                self.hardware.remove(id);
                self.slots[s].devices[d].held = false;
                Err(e)
            }
            None => Ok(()),
        }
    }

    fn find(&self, id: &str) -> Option<(usize, usize)> {
        self.slots.iter().enumerate().find_map(|(s, slot)| {
            let d = slot.devices.iter().position(|p| p.device.id == id)?;
            Some((s, d))
        })
    }

    /// What the device should run now: the effect it was given, if it takes it and is switched on.
    fn runnable(&self, p: &Placed) -> Option<HardwareSetting> {
        let setting = self.hardware.get(&p.device.id)?;
        let enabled = self.tuning.get(&p.device.id).is_none_or(|t| t.enabled);
        let fits = p.hardware.as_ref().is_some_and(|h| h.fit(setting).is_ok());
        (enabled && fits).then(|| setting.clone())
    }

    /// Starts or stops the device's hardware effect to match `runnable`.
    fn rehold(&mut self, id: &str) {
        let Some((s, d)) = self.find(id) else {
            return;
        };
        let setting = self.runnable(&self.slots[s].devices[d]);
        let p = &mut self.slots[s].devices[d];
        if p.held || setting.is_some() {
            p.held = setting.is_some();
            self.slots[s].link.switch(d, setting);
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
        self.views()
            .map(|(_, device, _, _)| device.clone())
            .collect()
    }

    /// Backend label, device, enabled and brightness for every device.
    pub fn views(&self) -> impl Iterator<Item = (&str, &Device, bool, f32)> {
        self.slots.iter().flat_map(move |slot| {
            slot.devices.iter().map(move |p| {
                let tuning = self.tuning.get(&p.device.id).copied().unwrap_or_default();
                (
                    slot.label.as_str(),
                    &p.device,
                    tuning.enabled,
                    tuning.brightness,
                )
            })
        })
    }

    /// Backend label and device for every backend that still takes frames.
    pub fn driven(&self) -> Vec<(&str, &Device)> {
        let working = self
            .slots
            .iter()
            .filter(|slot| slot.link.mail().error.is_none());
        working
            .flat_map(|slot| {
                slot.devices
                    .iter()
                    .map(move |p| (slot.label.as_str(), &p.device))
            })
            .collect()
    }

    /// Label, device count and push error of every backend.
    pub fn status(&self) -> Vec<(String, usize, Option<String>)> {
        self.slots
            .iter()
            .map(|s| {
                (
                    s.label.clone(),
                    s.devices.len(),
                    s.link.mail().error.clone(),
                )
            })
            .collect()
    }

    /// True once after a backend started failing.
    pub fn take_failed(&mut self) -> bool {
        self.failed.swap(false, Ordering::Relaxed)
    }

    /// Queues one black frame for every enabled device.
    pub fn blackout(&mut self) {
        self.black(None);
    }

    /// Waits, for at most `limit`, until the backends have sent everything queued so far.
    pub fn flush(&self, limit: Duration) {
        let deadline = Instant::now() + limit;
        for slot in &self.slots {
            let mut mail = slot.link.mail();
            while mail.error.is_none()
                && (mail.busy
                    || !mail.switches.is_empty()
                    || mail.batch.iter().any(Option::is_some))
            {
                let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                    return;
                };
                mail = slot
                    .link
                    .wake
                    .wait_timeout(mail, left)
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
            }
        }
    }

    /// Whether anything shown reads audio, the screen, and key presses.
    pub fn wants(&self) -> (bool, bool, bool) {
        let effects = || std::iter::once(&self.effect).chain(self.overrides.values());
        let layers = || {
            self.layers
                .iter()
                .filter(|l| l.live(&self.apps, &self.batteries))
        };
        (
            effects().any(Effect::uses_audio) || layers().any(Layer::uses_audio),
            effects().any(Effect::uses_screen) || layers().any(Layer::uses_screen),
            layers().any(Layer::uses_keys),
        )
    }

    /// Renders the effect at time `t`, hands it to every backend and returns what was rendered.
    pub fn render(&mut self, t: f32, inputs: &Inputs) -> Vec<Frame> {
        let mut frames = Vec::new();
        let tables: Vec<_> = self.layers.iter().map(Layer::key_table).collect();
        for slot in &self.slots {
            let mut batch = Batch::with_capacity(slot.devices.len());
            for p in &slot.devices {
                let device = &p.device;
                let tuning = self.tuning.get(&device.id).copied().unwrap_or_default();
                let lit = tuning.enabled && !self.paused;
                let k = if lit {
                    self.brightness * tuning.brightness
                } else {
                    0.0
                };
                let colors: Vec<Color> = if k > 0.0 {
                    let effect = self.overrides.get(&device.id).unwrap_or(&self.effect);
                    let layers: Vec<_> = self
                        .layers
                        .iter()
                        .zip(&tables)
                        .filter(|(l, _)| l.covers(device) && l.live(&self.apps, &self.batteries))
                        .collect();
                    device
                        .canvas_positions()
                        .enumerate()
                        .map(|(i, (x, y))| {
                            let base = effect.sample(x, y, t, inputs);
                            layers
                                .iter()
                                .fold(base, |below, (l, table)| match table {
                                    Some(table) => {
                                        l.over(below, p.spots.get(i).and_then(|s| paint(s, table)))
                                    }
                                    None => l.apply(below, x, y, t, inputs),
                                })
                                .scale(k)
                        })
                        .collect()
                } else {
                    vec![Color::BLACK; device.leds.len()]
                };
                batch.push((tuning.enabled && !p.held).then(|| colors.clone()));
                frames.push(Frame {
                    device: device.id.clone(),
                    colors,
                });
            }
            slot.link.post(batch);
        }
        frames
    }

    fn black(&self, only: Option<&str>) {
        for slot in &self.slots {
            let batch = slot.devices.iter().map(|p| {
                let id = p.device.id.as_str();
                let enabled = self.tuning.get(id).is_none_or(|t| t.enabled);
                (only.map_or(enabled, |o| o == id) && !p.held)
                    .then(|| vec![Color::BLACK; p.device.leds.len()])
            });
            slot.link.post(batch.collect());
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
        for p in self.slots.iter_mut().flat_map(|slot| &mut slot.devices) {
            p.spots = spots(&p.device);
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
            if x + w <= 1.0 - STEP
                && y + h <= 1.0 - STEP
                && !taken.iter().any(|t| overlaps(padded, *t))
            {
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

    const SOON: Duration = Duration::from_secs(5);

    /// Three devices that all claim the whole canvas; counts pushes, can be made to fail or to dawdle.
    struct Fake {
        pushes: Arc<Mutex<Vec<String>>>,
        /// What each device was sent last.
        last: Arc<Mutex<HashMap<String, Vec<Color>>>>,
        fail: bool,
        delay: Duration,
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

        fn push(&mut self, device_id: &str, colors: &[Color]) -> Result<(), String> {
            thread::sleep(self.delay);
            self.pushes.lock().unwrap().push(device_id.to_string());
            self.last
                .lock()
                .unwrap()
                .insert(device_id.to_string(), colors.to_vec());
            if self.fail {
                Err("gone".into())
            } else {
                Ok(())
            }
        }

        fn flush(&mut self) -> Result<(), String> {
            self.pushes.lock().unwrap().push("flush".into());
            Ok(())
        }
    }

    fn fake(fail: bool) -> (Engine, Arc<Mutex<Vec<String>>>) {
        let pushes = Arc::new(Mutex::new(Vec::new()));
        let backend = Fake {
            pushes: pushes.clone(),
            last: Arc::default(),
            fail,
            delay: Duration::ZERO,
        };
        (
            Engine::new(vec![Box::new(VirtualBackend), Box::new(backend)]),
            pushes,
        )
    }

    fn all_black(frame: &Frame) -> bool {
        frame.colors.iter().all(|c| *c == Color::BLACK)
    }

    /// A TKL board with an LED on every key, a four-zone board and a strip.
    struct Boards;

    impl Backend for Boards {
        fn name(&self) -> &str {
            "boards"
        }

        fn devices(&self) -> Vec<Device> {
            let leds = crate::key_layout()
                .iter()
                .filter_map(|k| crate::key_spot(k.vk, false).filter(|_| k.x < 18.0))
                .map(|(x, y)| [x, y])
                .collect();
            let per_key = Device {
                id: "keys".into(),
                name: "Keys".into(),
                kind: DeviceKind::Keyboard,
                rect: [0.0; 4],
                grid: [18, 6],
                leds,
            };
            vec![
                per_key,
                Device::grid("zones", "Zones", DeviceKind::Keyboard, [0.0; 4], 4, 1),
                Device::grid("strip", "Strip", DeviceKind::Strip, [0.0; 4], 4, 1),
            ]
        }

        fn push(&mut self, _: &str, _: &[Color]) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn key_colours_paint_keys_and_zones_with_the_layer_blend() {
        use crate::{Blend, LayerEffect, Post};
        const BASE: Color = Color(0, 0, 100);
        const RED: Color = Color(255, 0, 0);
        let mut engine = Engine::new(vec![Box::new(Boards)]);
        engine.effect = Effect::Static { color: BASE };
        let mut layer = Layer::new(LayerEffect::Post(Post::KeyColours {
            map: vec![(0x41, RED), (0x53, RED)],
        }));
        layer.opacity = 0.5;
        engine.layers = vec![layer];
        let device = |e: &mut Engine, id: &str| {
            e.render(0.0, &Inputs::default())
                .into_iter()
                .find(|f| f.device == id)
                .unwrap()
                .colors
        };
        let half = BASE.lerp(RED, 0.5);
        let keys = device(&mut engine, "keys");
        assert_eq!(keys.iter().filter(|c| **c == half).count(), 2);
        assert_eq!(keys.iter().filter(|c| **c == BASE).count(), keys.len() - 2);
        assert_eq!(device(&mut engine, "zones"), [half, BASE, BASE, BASE]);
        assert_eq!(device(&mut engine, "strip"), [BASE; 4]);
        engine.layers[0].blend = Blend::Add;
        engine.layers[0].opacity = 1.0;
        assert_eq!(device(&mut engine, "zones")[0], Color(255, 0, 100));
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
            assert!(
                x >= 0.0 && y >= 0.0 && x + w <= 1.0 && y + h <= 1.0,
                "{} is off the canvas",
                a.id
            );
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
        let rect_of =
            |e: &Engine, id: &str| e.devices().into_iter().find(|d| d.id == id).unwrap().rect;
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
        engine.flush(SOON);
        assert_eq!(*pushes.lock().unwrap(), ["a", "flush"]);
        let frames = engine.render(1.0, &Inputs::default());
        engine.flush(SOON);
        assert!(all_black(frames.iter().find(|f| f.device == "a").unwrap()));
        assert_eq!(*pushes.lock().unwrap(), ["a", "flush", "b", "c", "flush"]);
    }

    #[test]
    fn failing_backend_is_reported_and_skipped() {
        let (mut engine, pushes) = fake(true);
        assert_eq!(engine.render(1.0, &Inputs::default()).len(), 9);
        engine.flush(SOON);
        assert!(engine.take_failed());
        assert!(!engine.take_failed());
        engine.render(2.0, &Inputs::default());
        engine.flush(SOON);
        assert_eq!(pushes.lock().unwrap().len(), 1);
        let status = engine.status();
        assert_eq!(status[0], ("virtual".to_string(), 6, None));
        assert!(status[1].2.as_deref().unwrap().contains("gone"));
        assert!(engine.driven().iter().all(|(label, _)| *label == "virtual"));
    }

    #[test]
    fn a_slow_backend_holds_up_nothing_and_gets_the_newest_frame() {
        let pushes = Arc::new(Mutex::new(Vec::new()));
        let last = Arc::new(Mutex::new(HashMap::new()));
        let delay = Duration::from_millis(250);
        let slow = Fake {
            pushes: pushes.clone(),
            last: last.clone(),
            fail: false,
            delay,
        };
        let mut engine = Engine::new(vec![Box::new(slow)]);
        let started = Instant::now();
        let mut newest = Vec::new();
        for i in 0..20 {
            newest = engine.render(i as f32, &Inputs::default());
        }
        engine.set_device("b", false, 1.0, None);
        assert!(
            started.elapsed() < Duration::from_millis(200),
            "render waited for the backend"
        );
        engine.flush(SOON);
        // 20 frames for 3 devices went in; only what was newest when the backend came round goes out
        let pushes = pushes.lock().unwrap();
        assert!(pushes.len() <= 9, "{} pushes", pushes.len());
        // however the frames were batched, each device ends on the last thing it was told
        let last = last.lock().unwrap();
        for frame in &newest {
            let black = vec![Color::BLACK; frame.colors.len()];
            let expected = if frame.device == "b" {
                &black
            } else {
                &frame.colors
            };
            assert_eq!(&last[&frame.device], expected, "{}", frame.device);
        }
        assert_ne!(last["a"], vec![Color::BLACK; 10]);
    }

    #[test]
    fn backends_are_swapped_by_label_and_keep_their_order() {
        let (mut engine, pushes) = fake(false);
        let more = Fake {
            pushes: pushes.clone(),
            last: Arc::default(),
            fail: false,
            delay: Duration::ZERO,
        };
        engine.remove("virtual").wait(SOON);
        assert_eq!(engine.status().len(), 1);
        engine.add("virtual", vec![Box::new(VirtualBackend)]);
        engine.add("fake", vec![Box::new(more)]);
        let labels: Vec<String> = engine
            .status()
            .into_iter()
            .map(|(label, ..)| label)
            .collect();
        assert_eq!(labels, ["virtual", "fake", "fake"]);
        engine.remove("fake").wait(SOON);
        engine.render(1.0, &Inputs::default());
        engine.flush(SOON);
        assert!(pushes.lock().unwrap().is_empty());
        assert_eq!(engine.devices().len(), 6);
    }

    /// One device with a hardware mode; logs pushes and switches, refuses mode "bad".
    struct Own(Arc<Mutex<Vec<String>>>);

    impl Backend for Own {
        fn name(&self) -> &str {
            "own"
        }

        fn devices(&self) -> Vec<Device> {
            vec![Device::grid(
                "h",
                "H",
                DeviceKind::Strip,
                [0.0, 0.0, 1.0, 1.0],
                2,
                1,
            )]
        }

        fn push(&mut self, device_id: &str, _: &[Color]) -> Result<(), String> {
            self.0.lock().unwrap().push(format!("push {device_id}"));
            Ok(())
        }

        fn hardware(&self, _: &str) -> Option<Hardware> {
            let modes = ["spin", "bad"].map(|m| crate::HardwareMode::new(m, m));
            Some(Hardware {
                modes: modes.to_vec(),
            })
        }

        fn set_hardware(&mut self, id: &str, s: Option<&HardwareSetting>) -> Result<(), String> {
            let mode = s.map_or("none", |s| s.mode.as_str());
            self.0.lock().unwrap().push(format!("hw {id} {mode}"));
            if mode == "bad" {
                Err("refused".into())
            } else {
                Ok(())
            }
        }
    }

    fn mode(m: &str) -> HardwareSetting {
        serde_json::from_value(serde_json::json!({ "mode": m })).unwrap()
    }

    #[test]
    fn a_device_in_a_hardware_mode_gets_no_frames_until_handed_back() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut engine = Engine::new(Vec::new());
        engine.set_hardware("h", Some(mode("spin"))).unwrap();
        engine.add("own", vec![Box::new(Own(log.clone()))]);
        let taken = |log: &Arc<Mutex<Vec<String>>>| std::mem::take(&mut *log.lock().unwrap());
        engine.render(0.0, &Inputs::default());
        engine.blackout();
        engine.flush(SOON);
        assert_eq!(taken(&log), ["hw h spin"]);
        assert!(engine.set_hardware("h", Some(mode("nope"))).is_err());
        assert!(engine.set_hardware("virtual", Some(mode("spin"))).is_ok());

        engine.set_hardware("h", None).unwrap();
        engine.render(0.0, &Inputs::default());
        engine.flush(SOON);
        assert_eq!(taken(&log), ["hw h none", "push h"]);

        let refused = engine.set_hardware("h", Some(mode("bad")));
        assert_eq!(refused, Err("H: refused".into()));
        assert_eq!(engine.hardware_setting("h"), None);
        engine.render(1.0, &Inputs::default());
        engine.flush(SOON);
        assert_eq!(taken(&log), ["hw h bad", "push h"]);

        engine.set_hardware("h", Some(mode("spin"))).unwrap();
        engine.set_device("h", false, 1.0, None);
        engine.flush(SOON);
        assert_eq!(taken(&log), ["hw h spin", "hw h none", "push h"]);
        engine.set_device("h", true, 1.0, None);
        engine.render(2.0, &Inputs::default());
        engine.flush(SOON);
        assert_eq!(taken(&log), ["hw h spin"]);
        assert_eq!(engine.hardware("h").unwrap().modes.len(), 2);
    }
}
