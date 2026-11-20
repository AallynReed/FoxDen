//! SteelSeries Engine's devices, batteries and firmware, asked from a worker thread: its calls block for seconds when GG is down.

use std::collections::HashSet;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{LazyLock, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use foxden_battery::{Battery, Kind};
use serde::Serialize;
use serde_json::{Value, json};
use steelseries_gg::engine::{EngineDevice, FirmwareConsent, STEELSERIES_VID};
use steelseries_gg::{Endpoints, EngineClient, Error};
use tauri::{AppHandle, Emitter};

use crate::state::{Shared, lock};

const POLL: Duration = Duration::from_secs(20);
const UPDATING_POLL: Duration = Duration::from_secs(3);
const OFF_POLL: Duration = Duration::from_secs(5);
fn offline(e: &Error) -> String {
    match e {
        Error::Offline(_) => "SteelSeries GG is not running".into(),
        other => other.to_string(),
    }
}

/// SteelSeries USB product ids of an Engine device.
fn pids(d: &EngineDevice) -> Vec<u16> {
    let mut out: Vec<u16> = [d.product_id, d.connected_product_id]
        .into_iter()
        .chain(d.physical_devices.iter().map(|p| p.product_id))
        .filter(|id| (id >> 16) as u16 == STEELSERIES_VID)
        .map(|id| id as u16)
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// GG only flashes some headsets above this charge.
pub const MIN_BATTERY: u8 = 15;

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Row {
    pub id: u32,
    pub name: String,
    pub kind: String,
    /// `1038:22a1`, the id Windows sees.
    pub usb: String,
    pub firmware: String,
    pub update: bool,
    pub updating: bool,
    pub battery: Option<u8>,
    pub charging: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct View {
    pub on: bool,
    pub online: bool,
    pub error: Option<String>,
    pub devices: Vec<Row>,
}

enum Cmd {
    Refresh,
    Firmware(u32, FirmwareConsent, Sender<Result<String, String>>),
}

type Charged = (Battery, Vec<u16>);

static VIEW: LazyLock<Mutex<View>> = LazyLock::new(Mutex::default);
/// Engine batteries, keyed like the native SteelSeries driver, with every product id of the device.
static BATTERIES: LazyLock<Mutex<Vec<Charged>>> = LazyLock::new(Mutex::default);
static JOBS: OnceLock<Sender<Cmd>> = OnceLock::new();

fn view() -> View {
    VIEW.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn publish(app: &AppHandle, next: View) {
    let mut v = VIEW.lock().unwrap_or_else(|e| e.into_inner());
    if *v != next {
        *v = next.clone();
        drop(v);
        let _ = app.emit("gg-devices", &next);
    }
}

pub fn spawn(app: AppHandle, shared: Shared) {
    let (tx, rx) = channel();
    let _ = JOBS.set(tx);
    thread::spawn(move || run(app, shared, rx));
}

fn run(app: AppHandle, shared: Shared, rx: Receiver<Cmd>) {
    let mut ep: Option<Endpoints> = None;
    let mut no_battery: HashSet<u32> = HashSet::new();
    loop {
        let on = lock(&shared).config.settings.steelseries_gg;
        let mut wait = POLL;
        if !on {
            ep = None;
            no_battery.clear();
            BATTERIES.lock().unwrap_or_else(|e| e.into_inner()).clear();
            publish(&app, View::default());
            wait = OFF_POLL;
        } else {
            let next = poll(&mut ep, &mut no_battery);
            if next.devices.iter().any(|d| d.updating) {
                wait = UPDATING_POLL;
            }
            publish(&app, next);
        }
        match rx.recv_timeout(wait) {
            Ok(Cmd::Refresh) | Err(RecvTimeoutError::Timeout) => {}
            Ok(Cmd::Firmware(id, consent, reply)) => {
                let out = if on {
                    firmware(&ep, id, consent)
                } else {
                    Err("SteelSeries GG is off in Settings".into())
                };
                let _ = reply.send(out);
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn engine(ep: &mut Option<Endpoints>) -> Result<EngineClient, String> {
    if ep.is_none() {
        *ep = Some(Endpoints::discover().map_err(|e| offline(&e))?);
    }
    ep.as_ref()
        .and_then(Endpoints::engine)
        .ok_or_else(|| "SteelSeries Engine is not available".into())
}

fn poll(ep: &mut Option<Endpoints>, no_battery: &mut HashSet<u32>) -> View {
    let listed = engine(ep).and_then(|e| e.devices().map(|d| (e, d)).map_err(|err| offline(&err)));
    let (engine, devices) = match listed {
        Ok(x) => x,
        Err(error) => {
            // Ports change on every GG start, so look again next time.
            *ep = None;
            BATTERIES.lock().unwrap_or_else(|e| e.into_inner()).clear();
            return View {
                on: true,
                online: false,
                error: Some(error),
                devices: Vec::new(),
            };
        }
    };
    let connected: Vec<&EngineDevice> = devices.iter().filter(|d| d.is_connected()).collect();
    no_battery.retain(|id| connected.iter().any(|d| d.id == *id));
    let mut rows = Vec::new();
    let mut batteries = Vec::new();
    for d in connected {
        let charge = if no_battery.contains(&d.id) {
            None
        } else {
            let read = charge(&engine, d.id);
            if read.is_none() {
                no_battery.insert(d.id);
            }
            read
        };
        let mut row = row(d);
        if let Some((percent, charging)) = charge {
            row.battery = Some(percent);
            row.charging = charging;
            batteries.push((battery(d, percent, charging), pids(d)));
        }
        rows.push(row);
    }
    *BATTERIES.lock().unwrap_or_else(|e| e.into_inner()) = batteries;
    View {
        on: true,
        online: true,
        error: None,
        devices: rows,
    }
}

/// The pid Windows sees: the receiver's when there is one.
fn usb_pid(d: &EngineDevice) -> u16 {
    [d.connected_product_id, d.product_id]
        .into_iter()
        .find(|id| (id >> 16) as u16 == STEELSERIES_VID)
        .map_or(0, |id| id as u16)
}

fn row(d: &EngineDevice) -> Row {
    let firmware = d
        .physical_devices
        .iter()
        .map(|p| p.firmware.version.as_str())
        .find(|v| !v.is_empty())
        .unwrap_or("")
        .to_string();
    Row {
        id: d.id,
        name: d.title().to_string(),
        kind: d.device_type_name.clone(),
        usb: format!("{STEELSERIES_VID:04x}:{:04x}", usb_pid(d)),
        firmware,
        update: d.firmware_update_available(),
        updating: d
            .physical_devices
            .iter()
            .any(|p| p.firmware.update_in_progress),
        battery: None,
        charging: false,
    }
}

fn battery(d: &EngineDevice, percent: u8, charging: bool) -> Battery {
    let name = d.device_type_name.to_lowercase();
    Battery {
        key: format!("steelseries-1038:{:04X}", usb_pid(d)),
        name: d.title().to_string(),
        via: "SteelSeries GG".into(),
        kind: if name.contains("headset") {
            Kind::Headset
        } else if name.contains("mouse") {
            Kind::Mouse
        } else if name.contains("keyboard") {
            Kind::Keyboard
        } else {
            Kind::Other
        },
        percent: Some(percent),
        charging,
        full: charging && percent >= 100,
        grace_secs: 60,
        vid: STEELSERIES_VID,
        ..Default::default()
    }
}

/// Charge level and charging, as GG's own firmware check asks for it.
fn charge(engine: &EngineClient, id: u32) -> Option<(u8, bool)> {
    engine
        .command(id, &json!({ "charge_status": {} }))
        .ok()
        .and_then(|v| read_charge(&v))
        .or_else(|| engine.battery(id).ok().and_then(|v| read_charge(&v)))
}

fn find<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    match v {
        Value::Object(map) => keys
            .iter()
            .find_map(|k| map.get(*k))
            .or_else(|| map.values().find_map(|x| find(x, keys))),
        Value::Array(list) => list.iter().find_map(|x| find(x, keys)),
        _ => None,
    }
}

pub fn read_charge(v: &Value) -> Option<(u8, bool)> {
    let level = find(v, &["battery_level", "batteryLevel", "battery_percentage"])?.as_f64()?;
    if !(0.0..=100.0).contains(&level) {
        return None;
    }
    let charging = match find(v, &["charging", "is_charging", "isCharging"]) {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|n| n != 0.0),
        _ => false,
    };
    Some((level.round() as u8, charging))
}

fn firmware(ep: &Option<Endpoints>, id: u32, consent: FirmwareConsent) -> Result<String, String> {
    let engine = ep
        .as_ref()
        .and_then(Endpoints::engine)
        .ok_or("SteelSeries GG is not running")?;
    let row = view().devices.into_iter().find(|d| d.id == id);
    let row = row.ok_or("the device is not connected")?;
    if !row.update {
        return Err("Engine has no update for this device".into());
    }
    // GG's own pages check this before they flash.
    if let Some(p) = charge(&engine, id).map(|c| c.0)
        && p <= MIN_BATTERY
    {
        return Err(format!(
            "battery at {p}%: charge it above {MIN_BATTERY}% first"
        ));
    }
    engine
        .start_firmware_update(id, None, consent)
        .map_err(|e| offline(&e))?;
    crate::log::info(
        "steelseries",
        format!("Firmware update started on {}", row.name),
    );
    Ok(format!("Updating {}", row.name))
}

/// Engine batteries for devices the native drivers did not report.
pub fn batteries(native: &[Battery]) -> Vec<Battery> {
    let taken: Vec<u16> = native
        .iter()
        .filter(|b| b.vid == STEELSERIES_VID)
        .filter_map(|b| key_pid(&b.key))
        .collect();
    BATTERIES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .filter(|(b, ids)| {
            !native.iter().any(|n| n.key == b.key) && !ids.iter().any(|p| taken.contains(p))
        })
        .map(|(b, _)| b.clone())
        .collect()
}

/// The product id at the end of a native key such as `steelseries-1038:22A1#2`.
fn key_pid(key: &str) -> Option<u16> {
    let tail = key.rsplit(':').next()?;
    let hex = tail.split('#').next()?;
    u16::from_str_radix(hex, 16).ok()
}

#[tauri::command]
pub fn gg_devices() -> View {
    view()
}

#[tauri::command]
pub fn gg_refresh() -> Result<(), String> {
    JOBS.get()
        .ok_or("not started")?
        .send(Cmd::Refresh)
        .map_err(|_| "the SteelSeries thread has stopped".into())
}

/// Only the firmware dialog's confirm button calls this.
#[tauri::command(async)]
pub fn gg_firmware_update(id: u32) -> Result<String, String> {
    let (reply, answer) = channel();
    let consent = FirmwareConsent::confirmed_by_user();
    JOBS.get()
        .ok_or("not started")?
        .send(Cmd::Firmware(id, consent, reply))
        .map_err(|_| "the SteelSeries thread has stopped".to_string())?;
    answer
        .recv_timeout(Duration::from_secs(60))
        .map_err(|_| "SteelSeries GG did not answer".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use steelseries_gg::engine::{Firmware, PhysicalDevice};

    #[test]
    fn charge_is_read_where_gg_puts_it() {
        let v = json!({ "charge_status": { "battery_level": 64, "charging": 1 } });
        assert_eq!(read_charge(&v), Some((64, true)));
        assert_eq!(
            read_charge(&json!({ "battery_level": 12.4 })),
            Some((12, false))
        );
        assert_eq!(read_charge(&json!({ "battery_level": -1 })), None);
        assert_eq!(read_charge(&json!({ "status": "ok" })), None);
    }

    #[test]
    fn native_keys_give_their_product_id() {
        assert_eq!(key_pid("steelseries-1038:22A1"), Some(0x22a1));
        assert_eq!(key_pid("steelseries-1038:22A1#2"), Some(0x22a1));
        assert_eq!(key_pid("logitech-1"), None);
    }

    fn nova() -> EngineDevice {
        EngineDevice {
            id: 358,
            product_id: 0x1038_227e,
            connected_product_id: 0x1038_22a1,
            name: "arctis_nova_7_gen2_tx".into(),
            display_name: "Arctis Nova 7".into(),
            device_type_name: "Headset".into(),
            connected: 1,
            physical_devices: vec![PhysicalDevice {
                product_id: 0x1038_227e,
                firmware: Firmware {
                    version: "1.4.2".into(),
                    should_update: true,
                    ..Default::default()
                },
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn engine_devices_use_the_native_key_and_are_not_listed_twice() {
        let d = nova();
        let b = battery(&d, 80, false);
        assert_eq!(b.key, "steelseries-1038:22A1");
        assert_eq!(
            (b.vid, b.kind, b.percent),
            (0x1038, Kind::Headset, Some(80))
        );
        let r = row(&d);
        assert_eq!(
            (r.usb.as_str(), r.firmware.as_str(), r.update),
            ("1038:22a1", "1.4.2", true)
        );

        *BATTERIES.lock().unwrap() = vec![(b.clone(), pids(&d))];
        assert_eq!(batteries(&[]), std::slice::from_ref(&b));
        let native = Battery {
            key: "steelseries-1038:227E".into(),
            vid: 0x1038,
            ..Default::default()
        };
        assert!(batteries(&[native]).is_empty());
        let other = Battery {
            key: "logitech-1".into(),
            vid: 0x046d,
            ..Default::default()
        };
        assert_eq!(batteries(&[other]).len(), 1);
        BATTERIES.lock().unwrap().clear();
    }
}
