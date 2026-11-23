//! AMD graphics card fans through ADLX (amdadlx64.dll, shipped with the AMD driver).
//! Interface layout from AMD's ADLX SDK headers (MIT).

use std::ffi::c_void;
use std::ptr::null_mut;

use windows_sys::Win32::System::LibraryLoader::{
    GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
};

const VERSION: u64 = (2 << 48) | 125;
const OK: i32 = 0;

type Obj = *mut c_void;

/// Method `index` of a COM-style object's vtable.
unsafe fn method<T: Copy>(obj: Obj, index: usize) -> T {
    let vtable = unsafe { *(obj as *const *const usize) };
    let entry = unsafe { *vtable.add(index) };
    unsafe { std::mem::transmute_copy(&entry) }
}

fn check(result: i32, what: &str) -> Result<(), String> {
    if result == OK {
        Ok(())
    } else {
        Err(format!("{what} failed ({result})"))
    }
}

/// Runs a call that hands back an object; only a success with an object counts.
fn take(what: &str, call: impl FnOnce(*mut Obj) -> i32) -> Result<Obj, String> {
    let mut obj: Obj = null_mut();
    check(call(&mut obj), what)?;
    if obj.is_null() {
        return Err(format!("{what} gave nothing back"));
    }
    Ok(obj)
}

unsafe fn release(obj: Obj) {
    if !obj.is_null() {
        let f: extern "system" fn(Obj) -> i32 = unsafe { method(obj, 1) };
        f(obj);
    }
}

/// Releases its interface when dropped, unless handed on with `keep`.
struct Held(Obj);

impl Held {
    fn keep(self) -> Obj {
        let obj = self.0;
        std::mem::forget(self);
        obj
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        unsafe { release(self.0) };
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Range {
    pub min: i32,
    pub max: i32,
    pub step: i32,
}

/// One point of the card's fan curve: speed in percent at a temperature in degrees Celsius.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    pub speed: i32,
    pub temperature: i32,
}

pub struct Card {
    tuning: Obj,
    fan: Obj,
    gpu: Obj,
    pub speed_range: Range,
    pub temperature_range: Range,
    pub zero_rpm_supported: bool,
}

unsafe impl Send for Card {}

type GetObj = extern "system" fn(Obj, *mut Obj) -> i32;
type ListAt = extern "system" fn(Obj, u32, *mut Obj) -> i32;
type ListSize = extern "system" fn(Obj) -> u32;

impl Card {
    /// The first AMD card whose fans ADLX lets software tune.
    pub fn open() -> Result<Card, String> {
        unsafe {
            let dll = LoadLibraryExW(
                wide("amdadlx64.dll").as_ptr(),
                null_mut(),
                LOAD_LIBRARY_SEARCH_SYSTEM32,
            );
            if dll.is_null() {
                return Err("no AMD driver (amdadlx64.dll)".into());
            }
            let init = GetProcAddress(dll, c"ADLXInitialize".as_ptr().cast())
                .ok_or("no ADLXInitialize")?;
            let init: extern "C" fn(u64, *mut Obj) -> i32 = std::mem::transmute(init);
            let system = take("ADLXInitialize", |o| init(VERSION, o))?;

            let get_gpus: GetObj = method(system, 1);
            let gpus = Held(take("GetGPUs", |o| get_gpus(system, o))?);
            let get_tuning: GetObj = method(system, 8);
            let tuning = Held(take("GetGPUTuningServices", |o| get_tuning(system, o))?);
            let (gpus_obj, tuning_obj) = (gpus.0, tuning.0);

            let size: ListSize = method(gpus_obj, 3);
            let at: ListAt = method(gpus_obj, 11);
            for i in 0..size(gpus_obj) {
                let Ok(gpu) = take("GPU", |o| at(gpus_obj, i, o)).map(Held) else {
                    continue;
                };
                let supported_fn: extern "system" fn(Obj, Obj, *mut u8) -> i32 =
                    method(tuning_obj, 10);
                let mut supported = 0u8;
                if supported_fn(tuning_obj, gpu.0, &mut supported) != OK || supported == 0 {
                    continue;
                }
                let get_fan: extern "system" fn(Obj, Obj, *mut Obj) -> i32 = method(tuning_obj, 16);
                let iface = Held(take("GetManualFanTuning", |o| {
                    get_fan(tuning_obj, gpu.0, o)
                })?);
                let query: extern "system" fn(Obj, *const u16, *mut Obj) -> i32 =
                    method(iface.0, 2);
                let name = wide("IADLXManualFanTuning");
                let fan = Held(take("QueryInterface", |o| {
                    query(iface.0, name.as_ptr(), o)
                })?);
                drop(iface);
                let ranges: extern "system" fn(Obj, *mut Range, *mut Range) -> i32 =
                    method(fan.0, 3);
                let (mut speed_range, mut temperature_range) = (Range::default(), Range::default());
                check(
                    ranges(fan.0, &mut speed_range, &mut temperature_range),
                    "GetFanTuningRanges",
                )?;
                let zero_supported: extern "system" fn(Obj, *mut u8) -> i32 = method(fan.0, 8);
                let mut zero = 0u8;
                let zero_rpm_supported = zero_supported(fan.0, &mut zero) == OK && zero != 0;
                drop(gpus);
                return Ok(Card {
                    tuning: tuning.keep(),
                    fan: fan.keep(),
                    gpu: gpu.keep(),
                    speed_range,
                    temperature_range,
                    zero_rpm_supported,
                });
            }
            Err("no AMD card with fan tuning".into())
        }
    }

    fn states(&self) -> Result<Obj, String> {
        let get: GetObj = unsafe { method(self.fan, 4) };
        take("GetFanTuningStates", |o| get(self.fan, o))
    }

    pub fn curve(&self) -> Result<Vec<Point>, String> {
        let list = self.states()?;
        let points = unsafe { read_list(list) };
        unsafe { release(list) };
        Ok(points)
    }

    /// Writes the fan curve. ADLX checks it before taking it.
    pub fn set_curve(&self, points: &[Point]) -> Result<(), String> {
        let list = self.states()?;
        unsafe {
            let size: ListSize = method(list, 3);
            let at: ListAt = method(list, 11);
            for (i, p) in (0..size(list)).zip(points) {
                if let Ok(state) = take("state", |o| at(list, i, o)) {
                    let set_speed: extern "system" fn(Obj, i32) -> i32 = method(state, 4);
                    let set_temp: extern "system" fn(Obj, i32) -> i32 = method(state, 6);
                    set_speed(state, p.speed);
                    set_temp(state, p.temperature);
                    release(state);
                }
            }
            let valid: extern "system" fn(Obj, Obj, *mut i32) -> i32 = method(self.fan, 6);
            let mut bad = -1;
            let result = valid(self.fan, list, &mut bad);
            if result != OK {
                release(list);
                return Err(format!("curve refused at point {bad} ({result})"));
            }
            let set: extern "system" fn(Obj, Obj) -> i32 = method(self.fan, 7);
            let result = set(self.fan, list);
            release(list);
            check(result, "SetFanTuningStates")
        }
    }

    pub fn zero_rpm(&self) -> Option<bool> {
        let get: extern "system" fn(Obj, *mut u8) -> i32 = unsafe { method(self.fan, 9) };
        let mut on = 0u8;
        (get(self.fan, &mut on) == OK).then_some(on != 0)
    }

    pub fn set_zero_rpm(&self, on: bool) -> Result<(), String> {
        let set: extern "system" fn(Obj, u8) -> i32 = unsafe { method(self.fan, 10) };
        check(set(self.fan, on as u8), "SetZeroRPMState")
    }
}

unsafe fn read_list(list: Obj) -> Vec<Point> {
    let size: ListSize = unsafe { method(list, 3) };
    let at: ListAt = unsafe { method(list, 11) };
    let mut points = Vec::new();
    for i in 0..size(list) {
        if let Ok(state) = take("state", |o| at(list, i, o)) {
            let get_speed: extern "system" fn(Obj, *mut i32) -> i32 = unsafe { method(state, 3) };
            let get_temp: extern "system" fn(Obj, *mut i32) -> i32 = unsafe { method(state, 5) };
            let (mut speed, mut temperature) = (0, 0);
            get_speed(state, &mut speed);
            get_temp(state, &mut temperature);
            points.push(Point { speed, temperature });
            unsafe { release(state) };
        }
    }
    points
}

impl Drop for Card {
    fn drop(&mut self) {
        unsafe {
            release(self.fan);
            release(self.gpu);
            release(self.tuning);
        }
    }
}

use foxden_fan::{Fan, FanBackend, Sensor};

/// What `CardFan` needs from the card, so it can be tested without one.
trait Tuning: Send {
    fn speed_range(&self) -> Range;
    fn temperature_range(&self) -> Range;
    fn zero_rpm_supported(&self) -> bool;
    fn curve(&mut self) -> Result<Vec<Point>, String>;
    fn set_curve(&mut self, points: &[Point]) -> Result<(), String>;
    fn zero_rpm(&mut self) -> Option<bool>;
    fn set_zero_rpm(&mut self, on: bool) -> Result<(), String>;
}

impl Tuning for Card {
    fn speed_range(&self) -> Range {
        self.speed_range
    }
    fn temperature_range(&self) -> Range {
        self.temperature_range
    }
    fn zero_rpm_supported(&self) -> bool {
        self.zero_rpm_supported
    }
    fn curve(&mut self) -> Result<Vec<Point>, String> {
        Card::curve(self)
    }
    fn set_curve(&mut self, points: &[Point]) -> Result<(), String> {
        Card::set_curve(self, points)
    }
    fn zero_rpm(&mut self) -> Option<bool> {
        Card::zero_rpm(self)
    }
    fn set_zero_rpm(&mut self, on: bool) -> Result<(), String> {
        Card::set_zero_rpm(self, on)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// A flat curve at this speed, zero RPM off.
    Flat(i32),
    /// The driver's own curve with zero RPM on, so the fan stops while the card is cool.
    Stop,
}

/// The card's fan as one controllable fan. A fixed duty is a flat curve, and 0% lets the
/// driver stop the fan; release puts the driver's own curve and zero-RPM setting back.
struct CardFan<T: Tuning> {
    card: T,
    /// The driver's curve and zero-RPM state, kept from the first write until release.
    saved: Option<(Vec<Point>, Option<bool>)>,
    /// Last mode written, so unchanged duties are not written again.
    mode: Option<Mode>,
    /// Last zero-RPM state written.
    zero: Option<bool>,
}

impl<T: Tuning> CardFan<T> {
    fn new(card: T) -> Self {
        CardFan {
            card,
            saved: None,
            mode: None,
            zero: None,
        }
    }

    fn mode(&self, duty: f32) -> Mode {
        let percent = (duty.clamp(0.0, 1.0) * 100.0).round() as i32;
        if percent == 0 && self.card.zero_rpm_supported() {
            return Mode::Stop;
        }
        let range = self.card.speed_range();
        Mode::Flat(percent.clamp(range.min, range.max))
    }
}

impl<T: Tuning> Fan for CardFan<T> {
    fn id(&self) -> &str {
        "gpu0/fan"
    }

    fn name(&self) -> &str {
        "Graphics card fan"
    }

    fn rpm(&mut self) -> Option<u32> {
        None
    }

    fn set_duty(&mut self, duty: f32) -> Result<(), String> {
        let mode = self.mode(duty);
        if self.mode == Some(mode) {
            return Ok(());
        }
        if self.saved.is_none() {
            self.saved = Some((self.card.curve()?, self.card.zero_rpm()));
        }
        self.mode = None;
        let (curve, zero) = match mode {
            Mode::Stop => (
                self.saved.as_ref().map(|s| s.0.clone()).unwrap_or_default(),
                true,
            ),
            Mode::Flat(speed) => {
                let temps = self.card.temperature_range();
                let points = (0..5)
                    .map(|i| Point {
                        speed,
                        temperature: temps.min + (temps.max - temps.min) * (i + 1) / 6,
                    })
                    .collect();
                (points, false)
            }
        };
        self.card.set_curve(&curve)?;
        if self.zero != Some(zero) {
            match self.card.set_zero_rpm(zero) {
                Ok(()) => self.zero = Some(zero),
                // A card that cannot turn zero RPM off still takes the curve.
                Err(_) if !zero => {}
                Err(e) => return Err(e),
            }
        }
        self.mode = Some(mode);
        Ok(())
    }

    fn release(&mut self) -> Result<(), String> {
        self.mode = None;
        self.zero = None;
        let Some((curve, zero)) = self.saved.take() else {
            return Ok(());
        };
        let restored = self.card.set_curve(&curve);
        let zero_restored = zero.map_or(Ok(()), |on| self.card.set_zero_rpm(on));
        let result = restored.and(zero_restored);
        if result.is_err() {
            // Kept, so the next release tries again.
            self.saved = Some((curve, zero));
        }
        result
    }
}

struct CardBackend(Option<Card>);

impl FanBackend for CardBackend {
    fn name(&self) -> &str {
        "amd adlx"
    }

    fn sensors(&mut self) -> Vec<Box<dyn Sensor>> {
        Vec::new()
    }

    fn fans(&mut self) -> Vec<Box<dyn Fan>> {
        self.0
            .take()
            .map(|card| Box::new(CardFan::new(card)) as Box<dyn Fan>)
            .into_iter()
            .collect()
    }
}

/// The AMD card's fan, if the driver allows tuning it.
pub fn backends() -> Vec<Box<dyn FanBackend>> {
    Card::open()
        .map(|card| Box::new(CardBackend(Some(card))) as Box<dyn FanBackend>)
        .into_iter()
        .collect()
}

/// The AMD card's fan on its own, for a process in the user's session.
pub fn fan() -> Result<Box<dyn Fan>, String> {
    Ok(Box::new(CardFan::new(Card::open()?)))
}

#[cfg(test)]
mod tests;
