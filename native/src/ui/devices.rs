use std::collections::HashMap;

use serde_json::json;
use slint::ComponentHandle;
use tauri::AppHandle;

use super::{AppWindow, DeviceItem, DevicesPage, Line, View, edit_settings, fill, jobs, refresh, report, s, with_view};
use crate::canvas::Dev;
use crate::commands;

pub fn bind(w: &AppWindow, app: &AppHandle) {
    let g = w.global::<DevicesPage>();
    let a = app.clone();
    g.on_select_device(move |id| {
        with_view(|v| v.canvas.selected = Some(id.to_string()));
        refresh(&a);
    });
    let a = app.clone();
    g.on_set_device_enabled(move |id, on| {
        if let Some(d) = with_view(|v| v.canvas.find(&id).cloned()) {
            send_device(&a, &d, on, d.brightness, d.rect);
        }
    });
    let a = app.clone();
    g.on_set_device_brightness(move |id, b| {
        if let Some(d) = with_view(|v| v.canvas.find(&id).cloned()) {
            send_device(&a, &d, d.enabled, b, d.rect);
        }
    });
    let a = app.clone();
    g.on_detect_now(move || report(commands::detect(jobs(&a))));
    let a = app.clone();
    g.on_rescan(move || report(commands::rescan(jobs(&a))));
    let a = app.clone();
    g.on_set_auto_detect(move |on| edit_settings(&a, |s| s["auto_detect"] = json!(on)));
}

pub fn event(_w: &AppWindow, _app: &AppHandle, _view: &mut View, _name: &str) {}

pub fn send_device(app: &AppHandle, d: &Dev, enabled: bool, brightness: f32, rect: [f32; 4]) {
    report(commands::set_device(
        app.clone(),
        super::shared(app),
        d.id.clone(),
        enabled,
        brightness,
        rect,
    ));
}

pub fn apply(w: &AppWindow, _app: &AppHandle, view: &mut View) {
    let g = w.global::<DevicesPage>();
    let snap = &view.snap;
    view.canvas.devices = snap["devices"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|d| {
                    Some(Dev {
                        id: d["id"].as_str()?.into(),
                        name: d["name"].as_str()?.into(),
                        kind: d["kind"].as_str().unwrap_or("other").into(),
                        rect: serde_json::from_value(d["rect"].clone()).ok()?,
                        grid: serde_json::from_value(d["grid"].clone()).unwrap_or([1, 1]),
                        leds: serde_json::from_value(d["leds"].clone()).unwrap_or_default(),
                        enabled: d["enabled"].as_bool().unwrap_or(true),
                        brightness: d["brightness"].as_f64().unwrap_or(1.0) as f32,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let backends: HashMap<&str, &str> = snap["devices"]
        .as_array()
        .map(|l| {
            l.iter()
                .filter_map(|d| Some((d["id"].as_str()?, d["backend"].as_str()?)))
                .collect()
        })
        .unwrap_or_default();
    fill(
        g.get_devices(),
        view.canvas
            .devices
            .iter()
            .map(|d| DeviceItem {
                id: d.id.as_str().into(),
                name: d.name.as_str().into(),
                backend: backends.get(d.id.as_str()).copied().unwrap_or_default().into(),
                enabled: d.enabled,
                brightness: d.brightness,
                selected: view.canvas.selected.as_ref() == Some(&d.id),
            })
            .collect(),
        |m| g.set_devices(m),
    );
    g.set_auto_detect(snap["settings"]["auto_detect"].as_bool().unwrap_or(false));
    fill(
        g.get_detected(),
        snap["detected"]
            .as_array()
            .map(|l| {
                l.iter()
                    .map(|d| Line {
                        title: s(&d["name"]),
                        detail: [&d["kind"], &d["state"], &d["note"]]
                            .iter()
                            .filter_map(|v| v.as_str().filter(|t| !t.is_empty()))
                            .collect::<Vec<_>>()
                            .join(" · ")
                            .into(),
                        ok: d["state"] == "active",
                    })
                    .collect()
            })
            .unwrap_or_default(),
        |m| g.set_detected(m),
    );
}
