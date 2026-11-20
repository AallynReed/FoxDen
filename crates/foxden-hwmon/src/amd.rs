//! AMD graphics cards through the AMD Display Library that ships with the driver.
//! Only query functions are bound. Layouts and indices follow AMD's public ADL SDK headers.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::mem::{size_of, zeroed};
use std::ptr::null_mut;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use foxden_fan::{FanBackend, Sensor};
use windows_sys::Win32::Foundation::{FreeLibrary, HMODULE};
use windows_sys::Win32::System::LibraryLoader::{
    GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
};
use windows_sys::Win32::System::Memory::{GetProcessHeap, HeapAlloc};

use crate::{QUICK, plausible, polled, text, wide};

const ADL_OK: c_int = 0;
const ADL_MAX_PATH: usize = 256;
const ADL_PMLOG_MAX_SENSORS: usize = 256;
/// ADL gives the vendor's hex digits as a decimal number. The real id is accepted too.
const AMD: [c_int; 2] = [1002, 0x1002];
/// One driver query answers every sensor of a card that is read within this time.
const SHARE: Duration = Duration::from_millis(250);

/// ADL_PMLOG_SENSORS index, id suffix and label of each temperature.
const SENSORS: [(usize, &str, &str); 3] = [
    (8, "temperature", "temperature"),
    (27, "hotspot", "hotspot"),
    (9, "memory", "memory temperature"),
];

#[repr(C)]
#[allow(dead_code)]
struct AdapterInfo {
    size: c_int,
    adapter_index: c_int,
    udid: [c_char; ADL_MAX_PATH],
    bus_number: c_int,
    device_number: c_int,
    function_number: c_int,
    vendor_id: c_int,
    adapter_name: [c_char; ADL_MAX_PATH],
    display_name: [c_char; ADL_MAX_PATH],
    present: c_int,
    exist: c_int,
    driver_path: [c_char; ADL_MAX_PATH],
    driver_path_ext: [c_char; ADL_MAX_PATH],
    pnp_string: [c_char; ADL_MAX_PATH],
    os_display_index: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct SingleSensorData {
    supported: c_int,
    value: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
#[allow(dead_code)]
struct PmLogDataOutput {
    size: c_int,
    sensors: [SingleSensorData; ADL_PMLOG_MAX_SENSORS],
}

type Malloc = extern "system" fn(c_int) -> *mut c_void;
type MainControlCreate = unsafe extern "C" fn(Malloc, c_int, *mut *mut c_void) -> c_int;
type MainControlDestroy = unsafe extern "C" fn(*mut c_void) -> c_int;
type NumberOfAdaptersGet = unsafe extern "C" fn(*mut c_void, *mut c_int) -> c_int;
type AdapterInfoGet = unsafe extern "C" fn(*mut c_void, *mut AdapterInfo, c_int) -> c_int;
type QueryPmLogDataGet = unsafe extern "C" fn(*mut c_void, c_int, *mut PmLogDataOutput) -> c_int;

/// ADL allocates through this when it hands back a list. None of the calls here do.
extern "system" fn allocate(size: c_int) -> *mut c_void {
    unsafe { HeapAlloc(GetProcessHeap(), 0, size.max(0) as usize) }
}

/// AMD leaves it to the caller to serialise ADL calls, so one runs at a time in the process.
fn serially<R>(call: impl FnOnce() -> R) -> R {
    static CALLS: Mutex<()> = Mutex::new(());
    let _guard = CALLS.lock().unwrap_or_else(|e| e.into_inner());
    call()
}

struct Library(HMODULE);

impl Library {
    /// `T` must be the function pointer type of the export `name`.
    unsafe fn symbol<T>(&self, name: &CStr) -> Option<T> {
        const { assert!(size_of::<T>() == size_of::<usize>()) };
        let address = unsafe { GetProcAddress(self.0, name.as_ptr().cast()) }?;
        Some(unsafe { std::mem::transmute_copy(&address) })
    }
}

impl Drop for Library {
    fn drop(&mut self) {
        unsafe { FreeLibrary(self.0) };
    }
}

struct Card {
    adapter: c_int,
    /// When the driver was last asked, and what it answered.
    last: (Instant, Option<PmLogDataOutput>),
}

struct Adl {
    context: *mut c_void,
    destroy: MainControlDestroy,
    query: QueryPmLogDataGet,
    cards: Vec<Card>,
    _library: Library,
}

// The context is only used under the mutex that owns this value.
unsafe impl Send for Adl {}

impl Drop for Adl {
    fn drop(&mut self) {
        serially(|| unsafe { (self.destroy)(self.context) });
    }
}

impl Adl {
    /// Loads the library and lists every adapter the driver knows.
    fn load() -> Option<(Adl, Vec<AdapterInfo>)> {
        unsafe {
            let name = wide("atiadlxx.dll");
            let module = LoadLibraryExW(name.as_ptr(), null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32);
            if module.is_null() {
                return None;
            }
            let library = Library(module);
            let create: MainControlCreate = library.symbol(c"ADL2_Main_Control_Create")?;
            let destroy = library.symbol(c"ADL2_Main_Control_Destroy")?;
            let count_adapters: NumberOfAdaptersGet =
                library.symbol(c"ADL2_Adapter_NumberOfAdapters_Get")?;
            let adapter_info: AdapterInfoGet = library.symbol(c"ADL2_Adapter_AdapterInfo_Get")?;
            let query = library.symbol(c"ADL2_New_QueryPMLogData_Get")?;

            let mut context = null_mut();
            if serially(|| create(allocate, 1, &mut context)) != ADL_OK || context.is_null() {
                return None;
            }
            let adl = Adl {
                context,
                destroy,
                query,
                cards: Vec::new(),
                _library: library,
            };

            let mut count = 0;
            if serially(|| count_adapters(context, &mut count)) != ADL_OK || count <= 0 {
                return None;
            }
            let mut adapters: Vec<AdapterInfo> = (0..count).map(|_| zeroed()).collect();
            let bytes = (adapters.len() * size_of::<AdapterInfo>()) as c_int;
            if serially(|| adapter_info(context, adapters.as_mut_ptr(), bytes)) != ADL_OK {
                return None;
            }
            Some((adl, adapters))
        }
    }

    /// The adapter's performance metrics as the driver has them now.
    fn metrics(&self, adapter: c_int) -> Option<PmLogDataOutput> {
        unsafe {
            let mut data: PmLogDataOutput = zeroed();
            let answered = serially(|| (self.query)(self.context, adapter, &mut data));
            (answered == ADL_OK).then_some(data)
        }
    }

    fn temperature(&mut self, card: usize, metric: usize) -> Option<f32> {
        if self.cards[card].last.0.elapsed() >= SHARE {
            let data = self.metrics(self.cards[card].adapter);
            self.cards[card].last = (Instant::now(), data);
        }
        temperature(self.cards[card].last.1.as_ref()?, metric)
    }
}

/// One entry per physical AMD card, in PCI order. ADL lists a card once per display output.
fn cards(adapters: &[AdapterInfo]) -> Vec<&AdapterInfo> {
    let place = |info: &&AdapterInfo| (info.bus_number, info.device_number, info.function_number);
    let mut cards: Vec<&AdapterInfo> = adapters
        .iter()
        .filter(|info| info.bus_number >= 0 && AMD.contains(&info.vendor_id))
        .collect();
    cards.sort_by_key(place);
    cards.dedup_by_key(|info| place(info));
    cards
}

/// A metric the driver supports, as plausible degrees Celsius.
fn temperature(data: &PmLogDataOutput, metric: usize) -> Option<f32> {
    let sensor = data.sensors[metric];
    if sensor.supported == 0 {
        return None;
    }
    plausible(sensor.value as f32)
}

struct GpuSensor {
    adl: Arc<Mutex<Adl>>,
    card: usize,
    metric: usize,
    id: String,
    name: String,
}

impl Sensor for GpuSensor {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self) -> Option<f32> {
        let mut adl = self.adl.lock().unwrap_or_else(|e| e.into_inner());
        adl.temperature(self.card, self.metric)
    }
}

pub fn open() -> Option<Box<dyn FanBackend>> {
    let (mut adl, adapters) = Adl::load()?;
    let mut found = Vec::new();
    for (gpu, info) in cards(&adapters).into_iter().enumerate() {
        let Some(data) = adl.metrics(info.adapter_index) else {
            continue;
        };
        let mut model = text(info.adapter_name.iter().map(|&c| c as u8));
        if model.is_empty() {
            model = format!("Graphics card {gpu}");
        }
        let card = adl.cards.len();
        let before = found.len();
        for (metric, id, label) in SENSORS {
            if let Some(celsius) = temperature(&data, metric) {
                let (id, name) = (format!("gpu{gpu}/{id}"), format!("{model} {label}"));
                found.push((card, metric, id, name, celsius));
            }
        }
        if found.len() > before {
            adl.cards.push(Card {
                adapter: info.adapter_index,
                last: (Instant::now(), Some(data)),
            });
        }
    }

    let adl = Arc::new(Mutex::new(adl));
    let sensor = |(card, metric, id, name, celsius)| {
        let adl = adl.clone();
        let sensor = GpuSensor {
            adl,
            card,
            metric,
            id,
            name,
        };
        (sensor, celsius)
    };
    polled("amd", found.into_iter().map(sensor).collect(), QUICK)
}

#[cfg(test)]
mod tests {
    use std::mem::offset_of;

    use super::*;

    fn adapter(index: c_int, vendor: c_int, bus: c_int) -> AdapterInfo {
        let mut info: AdapterInfo = unsafe { zeroed() };
        info.adapter_index = index;
        info.vendor_id = vendor;
        info.bus_number = bus;
        info
    }

    fn metrics(values: &[(usize, c_int, c_int)]) -> PmLogDataOutput {
        let mut data: PmLogDataOutput = unsafe { zeroed() };
        for &(metric, supported, value) in values {
            data.sensors[metric] = SingleSensorData { supported, value };
        }
        data
    }

    // Windows layout of adl_structures.h: 4-byte ints, 256-byte strings, default packing.
    #[test]
    fn layouts_match_the_adl_headers() {
        assert_eq!(size_of::<AdapterInfo>(), 1572);
        assert_eq!(offset_of!(AdapterInfo, size), 0);
        assert_eq!(offset_of!(AdapterInfo, adapter_index), 4);
        assert_eq!(offset_of!(AdapterInfo, udid), 8);
        assert_eq!(offset_of!(AdapterInfo, bus_number), 264);
        assert_eq!(offset_of!(AdapterInfo, device_number), 268);
        assert_eq!(offset_of!(AdapterInfo, function_number), 272);
        assert_eq!(offset_of!(AdapterInfo, vendor_id), 276);
        assert_eq!(offset_of!(AdapterInfo, adapter_name), 280);
        assert_eq!(offset_of!(AdapterInfo, display_name), 536);
        assert_eq!(offset_of!(AdapterInfo, present), 792);
        assert_eq!(offset_of!(AdapterInfo, exist), 796);
        assert_eq!(offset_of!(AdapterInfo, driver_path), 800);
        assert_eq!(offset_of!(AdapterInfo, driver_path_ext), 1056);
        assert_eq!(offset_of!(AdapterInfo, pnp_string), 1312);
        assert_eq!(offset_of!(AdapterInfo, os_display_index), 1568);

        assert_eq!(size_of::<SingleSensorData>(), 8);
        assert_eq!(offset_of!(SingleSensorData, supported), 0);
        assert_eq!(offset_of!(SingleSensorData, value), 4);
        assert_eq!(size_of::<PmLogDataOutput>(), 4 + 256 * 8);
        assert_eq!(offset_of!(PmLogDataOutput, size), 0);
        assert_eq!(offset_of!(PmLogDataOutput, sensors), 4);
    }

    #[test]
    fn temperature_needs_a_supported_plausible_metric() {
        let data = metrics(&[(8, 1, 47), (27, 1, 300), (9, 0, 52)]);
        assert_eq!(temperature(&data, 8), Some(47.0));
        assert_eq!(temperature(&data, 27), None);
        assert_eq!(temperature(&data, 9), None);
        assert_eq!(temperature(&data, 10), None);
    }

    #[test]
    fn cards_are_physical_amd_adapters_once_each_in_pci_order() {
        let adapters = [
            adapter(0, 1002, 3),
            adapter(1, 1002, 3),
            adapter(2, 0x8086, 0),
            adapter(3, 1002, -1),
            adapter(4, 0x1002, 1),
        ];
        let indices: Vec<c_int> = cards(&adapters)
            .iter()
            .map(|info| info.adapter_index)
            .collect();
        assert_eq!(indices, [4, 0]);
    }

    #[test]
    #[ignore = "prints what the driver reports on this machine"]
    fn dump() {
        let Some((adl, adapters)) = Adl::load() else {
            println!("no AMD driver");
            return;
        };
        for info in &adapters {
            println!(
                "adapter {} vendor {} bus {} device {} function {} present {} exist {} '{}' '{}'",
                info.adapter_index,
                info.vendor_id,
                info.bus_number,
                info.device_number,
                info.function_number,
                info.present,
                info.exist,
                text(info.adapter_name.iter().map(|&c| c as u8)),
                text(info.display_name.iter().map(|&c| c as u8)),
            );
        }
        for info in cards(&adapters) {
            for round in 0..3 {
                let at = Instant::now();
                let data = adl.metrics(info.adapter_index);
                let took = at.elapsed();
                let Some(data) = data else {
                    println!("adapter {}: query failed", info.adapter_index);
                    break;
                };
                let supported: Vec<String> = (0..ADL_PMLOG_MAX_SENSORS)
                    .filter(|&metric| data.sensors[metric].supported != 0)
                    .map(|metric| format!("{metric}={}", data.sensors[metric].value))
                    .collect();
                println!(
                    "adapter {} round {round} size {} in {took:?}: {}",
                    info.adapter_index,
                    data.size,
                    supported.join(" ")
                );
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        }
    }
}
