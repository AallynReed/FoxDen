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

unsafe fn release(obj: Obj) {
    if !obj.is_null() {
        let f: extern "system" fn(Obj) -> i32 = unsafe { method(obj, 1) };
        f(obj);
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
            let mut system: Obj = null_mut();
            check(init(VERSION, &mut system), "ADLXInitialize")?;

            let get_gpus: GetObj = method(system, 1);
            let mut gpus: Obj = null_mut();
            check(get_gpus(system, &mut gpus), "GetGPUs")?;
            let get_tuning: GetObj = method(system, 8);
            let mut tuning: Obj = null_mut();
            check(get_tuning(system, &mut tuning), "GetGPUTuningServices")?;

            let size: ListSize = method(gpus, 3);
            let at: ListAt = method(gpus, 11);
            for i in 0..size(gpus) {
                let mut gpu: Obj = null_mut();
                if at(gpus, i, &mut gpu) != OK {
                    continue;
                }
                let supported_fn: extern "system" fn(Obj, Obj, *mut u8) -> i32 = method(tuning, 10);
                let mut supported = 0u8;
                if supported_fn(tuning, gpu, &mut supported) != OK || supported == 0 {
                    release(gpu);
                    continue;
                }
                let get_fan: extern "system" fn(Obj, Obj, *mut Obj) -> i32 = method(tuning, 16);
                let mut iface: Obj = null_mut();
                check(get_fan(tuning, gpu, &mut iface), "GetManualFanTuning")?;
                let query: extern "system" fn(Obj, *const u16, *mut Obj) -> i32 = method(iface, 2);
                let mut fan: Obj = null_mut();
                let queried = query(iface, wide("IADLXManualFanTuning").as_ptr(), &mut fan);
                release(iface);
                check(queried, "QueryInterface")?;
                let ranges: extern "system" fn(Obj, *mut Range, *mut Range) -> i32 = method(fan, 3);
                let (mut speed_range, mut temperature_range) = (Range::default(), Range::default());
                check(
                    ranges(fan, &mut speed_range, &mut temperature_range),
                    "GetFanTuningRanges",
                )?;
                release(gpus);
                return Ok(Card {
                    tuning,
                    fan,
                    gpu,
                    speed_range,
                    temperature_range,
                });
            }
            release(gpus);
            Err("no AMD card with fan tuning".into())
        }
    }

    fn states(&self) -> Result<Obj, String> {
        let get: GetObj = unsafe { method(self.fan, 4) };
        let mut list: Obj = null_mut();
        check(get(self.fan, &mut list), "GetFanTuningStates")?;
        Ok(list)
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
                let mut state: Obj = null_mut();
                if at(list, i, &mut state) == OK {
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
        let mut state: Obj = null_mut();
        if at(list, i, &mut state) == OK {
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

use foxrgb_fan::{Fan, FanBackend, Sensor};

/// The card's fan as one controllable fan. A fixed duty is a flat curve; release puts the
/// driver's own curve and zero-RPM setting back.
struct CardFan {
    card: Card,
    /// The driver's curve and zero-RPM state, kept from the first write until release.
    saved: Option<(Vec<Point>, Option<bool>)>,
    /// Last speed written, in percent, so unchanged duties are not written again.
    speed: Option<i32>,
}

impl Fan for CardFan {
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
        let range = self.card.speed_range;
        let speed = ((duty.clamp(0.0, 1.0) * 100.0).round() as i32).clamp(range.min, range.max);
        if self.speed == Some(speed) {
            return Ok(());
        }
        if self.saved.is_none() {
            self.saved = Some((self.card.curve()?, self.card.zero_rpm()));
            let _ = self.card.set_zero_rpm(false);
        }
        let temps = self.card.temperature_range;
        let points: Vec<Point> = (0..5)
            .map(|i| Point {
                speed,
                temperature: temps.min + (temps.max - temps.min) * (i + 1) / 6,
            })
            .collect();
        self.card.set_curve(&points)?;
        self.speed = Some(speed);
        Ok(())
    }

    fn release(&mut self) -> Result<(), String> {
        self.speed = None;
        if let Some((curve, zero_rpm)) = self.saved.take() {
            self.card.set_curve(&curve)?;
            if let Some(on) = zero_rpm {
                self.card.set_zero_rpm(on)?;
            }
        }
        Ok(())
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
            .map(|card| {
                Box::new(CardFan {
                    card,
                    saved: None,
                    speed: None,
                }) as Box<dyn Fan>
            })
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
    Ok(Box::new(CardFan {
        card: Card::open()?,
        saved: None,
        speed: None,
    }))
}
