//! The Apps and Devices panels: Windows' volume, mute and level per app session and per device.
//! The COM objects live on one worker thread, which only works while the panel asks.

use std::cell::RefCell;
use std::sync::mpsc::{Sender, channel, sync_channel};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use foxden_mixer::system;
use foxden_mixer::volumes::{AppVolume, DeviceVolume, Volumes};
use serde::Serialize;
use tauri::State;

use crate::mixer::Mixer;

type Job = Box<dyn FnOnce(&mut Volumes) + Send>;

const VCABLE_EVERY: Duration = Duration::from_secs(2);

/// VB-CABLE's endpoints, read again at most every few seconds. Worker thread only.
fn vcable() -> Vec<foxden_vcable::Endpoint> {
    thread_local! {
        static CACHE: RefCell<Option<(Instant, Vec<foxden_vcable::Endpoint>)>> = const { RefCell::new(None) };
    }
    CACHE.with_borrow_mut(|c| {
        if c.as_ref()
            .is_none_or(|(at, _)| at.elapsed() >= VCABLE_EVERY)
        {
            *c = Some((
                Instant::now(),
                foxden_vcable::endpoints().unwrap_or_default(),
            ));
        }
        c.as_ref().map(|(_, e)| e.clone()).unwrap_or_default()
    })
}

fn worker() -> Result<Sender<Job>, String> {
    static TX: OnceLock<Mutex<Sender<Job>>> = OnceLock::new();
    let tx = TX.get_or_init(|| {
        let (tx, rx) = channel::<Job>();
        let _ = thread::Builder::new()
            .name("audio-volumes".into())
            .spawn(move || {
                system::com_init();
                let mut v = Volumes::default();
                while let Ok(job) = rx.recv() {
                    job(&mut v);
                }
            });
        Mutex::new(tx)
    });
    Ok(tx.lock().unwrap_or_else(|e| e.into_inner()).clone())
}

fn run<T: Send + 'static>(
    job: impl FnOnce(&mut Volumes) -> T + Send + 'static,
) -> Result<T, String> {
    let (tx, rx) = sync_channel(1);
    worker()?
        .send(Box::new(move |v| {
            let _ = tx.send(job(v));
        }))
        .map_err(|_| "the volume worker has stopped".to_string())?;
    rx.recv()
        .map_err(|_| "the volume worker has stopped".to_string())
}

#[derive(Serialize)]
pub struct AppRow {
    #[serde(flatten)]
    app: AppVolume,
    device: String,
}

#[derive(Serialize)]
pub struct DeviceRow {
    #[serde(flatten)]
    device: DeviceVolume,
    /// Set for FoxDen's own endpoints, which are not ordinary choices.
    note: Option<&'static str>,
}

#[derive(Serialize)]
pub struct View {
    apps: Vec<AppRow>,
    devices: Vec<DeviceRow>,
}

fn view(parking: Option<String>) -> Result<View, String> {
    let own = std::process::id();
    run(move |v| {
        let (apps, devices) = v.read(own)?;
        let vcable = vcable();
        let devices: Vec<DeviceRow> = devices
            .into_iter()
            .map(|d| {
                let note = match vcable.iter().find(|e| e.id == d.id) {
                    Some(e) if e.capture => Some("FoxDen virtual mic"),
                    Some(_) => Some("FoxDen virtual mic feed"),
                    None if !d.capture && parking.as_deref() == Some(d.name.as_str()) => {
                        Some("Parking output, muted while the mixer runs")
                    }
                    None => None,
                };
                DeviceRow { device: d, note }
            })
            .collect();
        let apps = apps
            .into_iter()
            .map(|a| AppRow {
                device: devices
                    .iter()
                    .find(|d| d.device.id == a.endpoint)
                    .map(|d| d.device.name.clone())
                    .unwrap_or_default(),
                app: a,
            })
            .collect();
        Ok(View { apps, devices })
    })?
}

#[tauri::command(async)]
pub fn get_volumes(mixer: State<'_, Mixer>) -> Result<View, String> {
    view(mixer.parking())
}

#[tauri::command(async)]
pub fn set_app_volume(
    mixer: State<'_, Mixer>,
    exe: String,
    endpoint: String,
    volume: Option<f32>,
    muted: Option<bool>,
) -> Result<View, String> {
    let volume = volume.filter(|v| v.is_finite());
    run(move |v| v.set_app(&exe, &endpoint, volume, muted))??;
    view(mixer.parking())
}

#[tauri::command(async)]
pub fn set_device_volume(
    mixer: State<'_, Mixer>,
    id: String,
    volume: Option<f32>,
    muted: Option<bool>,
) -> Result<View, String> {
    let volume = volume.filter(|v| v.is_finite());
    run(move |v| v.set_device(&id, volume, muted))??;
    view(mixer.parking())
}

/// Makes a device Windows' default for every role, as the output priority does.
#[tauri::command(async)]
pub fn make_default_device(mixer: State<'_, Mixer>, id: String) -> Result<View, String> {
    run(move |v| {
        if foxden_vcable::endpoints()
            .unwrap_or_default()
            .iter()
            .any(|e| e.id == id)
        {
            return Err("FoxDen's virtual mic endpoints are not made default here".to_string());
        }
        let out = foxden_vcable::set_default(&id);
        v.stale();
        out
    })??;
    crate::log::info("audio", "default device set from the Devices panel");
    view(mixer.parking())
}
