//! Fan and pump controllers: Corsair Commander Pro, Commander Core / Core XT, iCUE LINK hub,
//! Hydro Platinum / Pro XT / RGB Elite AIOs and ThermalTake Riing / LEDBOX controllers, plus
//! lighting for the ids FoxRGB lacks (those, and Corsair Bragi peripherals).

mod bragi;
mod bragi_models;
mod commander_pro;
mod modern;
mod platinum;
mod thermaltake;
mod util;

use foxrgb_core::Backend;
use foxrgb_fan::FanBackend;
use foxrgb_hid::registry::{Detector, Info};
use hidapi::HidApi;

pub fn detectors() -> Vec<&'static Detector> {
    thermaltake::DETECTORS
        .iter()
        .chain(platinum::DETECTORS.iter())
        .chain(bragi::DETECTORS.iter())
        .collect()
}

pub fn fan_ids() -> Vec<(u16, u16, &'static str)> {
    let mut v: Vec<_> = thermaltake::IDS
        .iter()
        .map(|&(pid, name)| (thermaltake::VID, pid, name))
        .collect();
    v.extend(
        commander_pro::IDS
            .iter()
            .map(|&(pid, name)| (commander_pro::VID, pid, name)),
    );
    v.extend(
        modern::IDS
            .iter()
            .map(|&(pid, name)| (modern::VID, pid, name)),
    );
    v.extend(
        platinum::IDS
            .iter()
            .map(|&(pid, name, _)| (platinum::VID, pid, name)),
    );
    v
}

pub fn other_ids() -> Vec<(u16, u16, &'static str)> {
    Vec::new()
}

pub fn probe_other() -> Vec<Box<dyn Backend>> {
    Vec::new()
}

type Make = fn(&HidApi, &Info) -> Option<Box<dyn FanBackend>>;

/// Which family drives a collection, and how its physical device is keyed.
fn family(info: &Info) -> Option<(Make, String)> {
    if thermaltake::wants(info) {
        Some((thermaltake::hub, thermaltake::key(info)))
    } else if commander_pro::wants(info) {
        Some((commander_pro::hub, commander_pro::key(info)))
    } else if modern::wants(info) {
        Some((modern::hub, modern::key(info)))
    } else if platinum::wants(info) {
        Some((platinum::hub, platinum::key(info)))
    } else {
        None
    }
}

pub fn fan_backends(api: &HidApi) -> Vec<Box<dyn FanBackend>> {
    let mut seen = Vec::new();
    let mut out = Vec::new();
    for raw in api.device_list() {
        let info = Info::from_hidapi(raw);
        let Some((make, key)) = family(&info) else {
            continue;
        };
        if seen.contains(&key) {
            continue;
        }
        if let Some(hub) = make(api, &info) {
            seen.push(key);
            out.push(hub);
        }
    }
    out
}
