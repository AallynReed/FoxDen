use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::time::Duration;

use serde_json::{Value, json};
use slint::{ComponentHandle, Model, ModelRc, Timer, TimerMode, VecModel};
use tauri::AppHandle;

use super::stage::{backend_name, fmt, note, plural, sentence, show_error};
use super::{
    AppWindow, DeviceRow, DevicesPage, FoundRow, HsCard, HsControl, StagePage, View, ZoneRow, edit_settings, fill,
    jobs, refresh, report, s, shared, tr, with_view, with_win,
};
use crate::canvas::Dev;
use crate::commands;

const MIN_SIZE: f32 = 0.02;
const PLACE: [&str; 4] = ["X", "Y", "Width", "Height"];
const UNSET: i32 = -1_000_000;
const HS_LABELS: [(&str, &str); 9] = [
    ("sidetone", "Sidetone"),
    ("mic_volume", "Mic volume"),
    ("volume_limiter", "Volume limiter"),
    ("auto_off", "Auto-off"),
    ("mic_led", "Mute light"),
    ("bt_power_on", "Bluetooth at power-on"),
    ("bt_call", "Bluetooth call"),
    ("anc", "Noise cancelling"),
    ("eq_preset", "EQ preset"),
];

struct Zone {
    key: String,
    name: String,
    min: usize,
    max: usize,
}

thread_local! {
    static ZONES: RefCell<Vec<Zone>> = const { RefCell::new(Vec::new()) };
    static DETECT: RefCell<Option<Timer>> = const { RefCell::new(None) };
    static HS_TIMER: RefCell<Option<Timer>> = const { RefCell::new(None) };
    static HS_BUSY: Cell<u32> = const { Cell::new(0) };
    static HS_VIEWS: RefCell<Value> = const { RefCell::new(Value::Null) };
    static SNAP_ID: Cell<usize> = const { Cell::new(0) };
}

fn words(s: &str) -> String {
    sentence(&s.replace('_', " "))
}

fn kind_name(kind: &str) -> String {
    match kind {
        "gpu" => "Graphics card".into(),
        "strip" => "Light strip".into(),
        k => words(k),
    }
}

fn problem(text: &str) {
    with_win(|w| w.global::<DevicesPage>().set_problem(text.into()));
}

/// Runs a backend command off the UI thread and hands its result back to it.
fn off_thread<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
    done: impl FnOnce(&AppWindow, T) + Send + 'static,
) {
    std::thread::spawn(move || {
        let out = work();
        let _ = slint::invoke_from_event_loop(move || with_win(|w| done(w, out)));
    });
}

pub fn bind(w: &AppWindow, app: &AppHandle) {
    let g = w.global::<DevicesPage>();
    let a = app.clone();
    g.on_opened(move || refresh_headsets(&a));
    let a = app.clone();
    g.on_select_device(move |id| {
        with_view(|v| {
            v.canvas.selected = (v.canvas.selected.as_deref() != Some(id.as_str())).then(|| id.to_string());
        });
        problem("");
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
    g.on_brightness_typed(move |id, text| {
        let v = text.trim().parse::<f32>().ok().filter(|v| (0.0..=100.0).contains(v));
        let Some(v) = v else {
            problem(&fmt(
                &a,
                "{0} must be from {1} to {2}{3}.",
                &[&tr(&a, "Brightness"), "0", "100", "%"],
            ));
            return false;
        };
        problem("");
        if let Some(d) = with_view(|view| view.canvas.find(&id).cloned()) {
            send_device(&a, &d, d.enabled, v.round() / 100.0, d.rect);
        }
        true
    });
    let a = app.clone();
    g.on_place_typed(move |n, text| place_value(&a, n, &text).is_some());
    let a = app.clone();
    g.on_place_set(move |n, text| {
        let Some(v) = place_value(&a, n, &text) else {
            return;
        };
        let Some(d) = with_view(|view| {
            let id = view.canvas.selected.clone()?;
            view.canvas.find(&id).cloned()
        }) else {
            return;
        };
        let mut r = d.rect;
        if (r[n as usize] - v).abs() < 1e-4 {
            return;
        }
        r[n as usize] = v;
        let w = r[2].clamp(MIN_SIZE, 1.0);
        let h = r[3].clamp(MIN_SIZE, 1.0);
        let rect = [r[0].clamp(0.0, 1.0 - w), r[1].clamp(0.0, 1.0 - h), w, h].map(|v| (v * 1e4).round() / 1e4);
        send_device(&a, &d, d.enabled, d.brightness, rect);
    });
    let a = app.clone();
    g.on_zone_typed(move |key, text| zone_value(&a, &key, &text).is_some());
    let a = app.clone();
    g.on_zone_set(move |key, text| {
        let Some(n) = zone_value(&a, &key, &text) else {
            return;
        };
        let now = with_view(|v| v.snap["settings"]["zone_leds"][key.as_str()].as_u64());
        if now == Some(n as u64) {
            return;
        }
        edit_settings(&a, |s| {
            if !s["zone_leds"].is_object() {
                s["zone_leds"] = json!({});
            }
            s["zone_leds"][key.as_str()] = json!(n);
        });
    });
    g.on_clear_problem(|| problem(""));
    let a = app.clone();
    g.on_reset_position(move |id| {
        let others: Vec<Dev> = with_view(|v| v.canvas.devices.iter().filter(|d| d.id != id.as_str()).cloned().collect());
        let r = commands::reset_layout(a.clone(), shared(&a));
        if r.is_err() {
            return report(r);
        }
        for d in others {
            send_device(&a, &d, d.enabled, d.brightness, d.rect);
        }
        let text = tr(&a, "Position reset");
        with_win(|w| note(w, &text));
    });
    let a = app.clone();
    g.on_detect_now(move || detect_now(&a));
    let a = app.clone();
    g.on_rescan(move || report(commands::rescan(jobs(&a))));
    let a = app.clone();
    g.on_set_auto_detect(move |on| edit_settings(&a, |s| s["auto_detect"] = json!(on)));
    let weak = w.as_weak();
    g.on_open_setting(move || {
        if let Some(w) = weak.upgrade() {
            w.set_tab(5);
        }
    });
    g.on_device_info(move |id, name| {
        with_win(|w| w.global::<DevicesPage>().set_reading(id.clone()));
        off_thread(
            move || tauri::async_runtime::block_on(crate::device_info::device_info(id.to_string(), name.to_string())),
            |w, out| {
                let g = w.global::<DevicesPage>();
                g.set_reading("".into());
                match out {
                    Ok(text) => {
                        g.set_info_text(text.into());
                        g.set_info_status("".into());
                        g.set_info_sending(false);
                        g.set_info_open(true);
                    }
                    Err(e) => show_error(w, &e),
                }
            },
        );
    });
    let a = app.clone();
    g.on_info_send(move || {
        let text = with_win_get(|w| w.global::<DevicesPage>().get_info_text().to_string());
        with_win(|w| {
            let g = w.global::<DevicesPage>();
            g.set_info_sending(true);
            g.set_info_status(tr(&a, "Sending…").into());
        });
        let a = a.clone();
        off_thread(
            move || tauri::async_runtime::block_on(crate::device_info::send_device_info(text)),
            move |w, out| {
                let g = w.global::<DevicesPage>();
                match out {
                    Ok(()) => g.set_info_status(tr(&a, "Sent. Thank you.").into()),
                    Err(e) => {
                        g.set_info_sending(false);
                        g.set_info_status(e.into());
                    }
                }
            },
        );
    });
    let a = app.clone();
    g.on_info_copied(move || {
        let text = tr(&a, "Copied. Paste it into a device issue on GitHub.");
        with_win(|w| w.global::<DevicesPage>().set_info_status(text.into()));
    });
    let a = app.clone();
    g.on_hs_set(move |key, control, value| {
        let what = label_of(&a, &control);
        let a2 = a.clone();
        let Ok(control) = serde_json::from_value(json!(control.as_str())) else {
            return;
        };
        headset_send(&a, what, move || {
            tauri::async_runtime::block_on(crate::headset::headset_set(shared(&a2), key.to_string(), control, value))
        });
    });
    g.on_hs_band(|key, i, v| {
        with_win(|w| {
            let cards = w.global::<DevicesPage>().get_headsets();
            for card in cards.iter().filter(|c| c.key == key) {
                if let Some(m) = card.bands.as_any().downcast_ref::<VecModel<i32>>()
                    && (i as usize) < m.row_count()
                {
                    m.set_row_data(i as usize, v);
                }
            }
        });
    });
    let a = app.clone();
    g.on_hs_eq(move |key| {
        let bands: Vec<i32> = with_win_get(|w| {
            w.global::<DevicesPage>()
                .get_headsets()
                .iter()
                .find(|c| c.key == key)
                .map(|c| c.bands.iter().collect())
                .unwrap_or_default()
        });
        let what = tr(&a, "The EQ");
        let a2 = a.clone();
        headset_send(&a, what, move || {
            tauri::async_runtime::block_on(crate::headset::headset_eq(shared(&a2), key.to_string(), bands))
        });
    });

    let weak = w.as_weak();
    let a = app.clone();
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_secs(20), move || {
        if weak.upgrade().is_some_and(|w| w.get_tab() == 1 && w.window().is_visible()) {
            refresh_headsets(&a);
        }
    });
    HS_TIMER.with_borrow_mut(|t| *t = Some(timer));
}

fn with_win_get<T: Default>(f: impl FnOnce(&AppWindow) -> T) -> T {
    let mut out = T::default();
    with_win(|w| out = f(w));
    out
}

fn place_value(app: &AppHandle, n: i32, text: &str) -> Option<f32> {
    let v = text.trim().parse::<f32>().ok().map(|v| v / 100.0).filter(|v| (0.0..=1.0).contains(v));
    let label = tr(app, PLACE[n.clamp(0, 3) as usize]);
    problem(&match v {
        Some(_) => String::new(),
        None => fmt(app, "{0} must be from 0 to 100.", &[&label]),
    });
    v
}

fn zone_value(app: &AppHandle, key: &str, text: &str) -> Option<usize> {
    ZONES.with_borrow(|zones| {
        let z = zones.iter().find(|z| z.key == key)?;
        let n = text.trim().parse::<usize>().ok().filter(|n| (z.min..=z.max).contains(n));
        problem(&match n {
            Some(_) => String::new(),
            None => fmt(
                app,
                "{0} takes {1} to {2} LEDs.",
                &[&z.name, &z.min.to_string(), &z.max.to_string()],
            ),
        });
        n
    })
}

fn detect_now(app: &AppHandle) {
    let r = commands::detect(jobs(app));
    if r.is_err() {
        return report(r);
    }
    let text = tr(app, "Detecting hardware");
    with_win(|w| {
        w.global::<DevicesPage>().set_detecting(true);
        note(w, &text);
    });
    let a = app.clone();
    let timer = Timer::default();
    timer.start(TimerMode::SingleShot, Duration::from_secs(5), move || {
        with_win(|w| {
            let g = w.global::<DevicesPage>();
            if g.get_detecting() {
                g.set_detecting(false);
                if w.global::<StagePage>().get_note() == tr(&a, "Detecting hardware").as_str() {
                    note(w, "");
                }
            }
        });
    });
    DETECT.with_borrow_mut(|t| *t = Some(timer));
}

/// Ends a detection pass once a new snapshot has come.
pub fn settle_detect(w: &AppWindow, app: &AppHandle) {
    let g = w.global::<DevicesPage>();
    if !g.get_detecting() {
        return;
    }
    g.set_detecting(false);
    DETECT.with_borrow_mut(|t| *t = None);
    if w.global::<StagePage>().get_note() == tr(app, "Detecting hardware").as_str() {
        note(w, &tr(app, "Nothing has changed"));
    }
}

fn label_of(app: &AppHandle, id: &str) -> String {
    let en = HS_LABELS.iter().find(|(k, _)| *k == id).map(|(_, v)| *v).unwrap_or(id);
    tr(app, en)
}

fn headset_send(app: &AppHandle, what: String, work: impl FnOnce() -> Result<(), String> + Send + 'static) {
    HS_BUSY.with(|b| b.set(b.get() + 1));
    with_win(|w| w.global::<DevicesPage>().set_hs_status("".into()));
    let a = app.clone();
    off_thread(work, move |w, out| {
        HS_BUSY.with(|b| b.set(b.get().saturating_sub(1)));
        if let Err(e) = out {
            let text = fmt(&a, "{0}. {1} was not changed.", &[&sentence(&e), &what]);
            w.global::<DevicesPage>().set_hs_status(text.into());
        }
        refresh_headsets(&a);
    });
}

fn refresh_headsets(app: &AppHandle) {
    if HS_BUSY.get() > 0 {
        return;
    }
    let a = app.clone();
    let a2 = app.clone();
    off_thread(
        move || tauri::async_runtime::block_on(crate::headset::headset_list(shared(&a2))),
        move |w, out| {
            if HS_BUSY.get() > 0 {
                return;
            }
            let views = out.ok().and_then(|v| serde_json::to_value(v).ok()).unwrap_or(json!([]));
            if HS_VIEWS.with_borrow(|last| *last == views) {
                return;
            }
            HS_VIEWS.with_borrow_mut(|last| *last = views.clone());
            let cards: Vec<HsCard> = views
                .as_array()
                .map(|l| l.iter().map(|h| card(&a, h)).collect())
                .unwrap_or_default();
            w.global::<DevicesPage>().set_headsets(ModelRc::new(VecModel::from(cards)));
        },
    );
}

fn facts(app: &AppHandle, h: &Value) -> String {
    let mut out = Vec::new();
    if let Some(b) = h["battery"].as_u64() {
        let charging = if h["charging"].as_bool().unwrap_or(false) { tr(app, ", charging") } else { String::new() };
        out.push(fmt(app, "Battery {0}%{1}", &[&b.to_string(), &charging]));
    }
    if let Some([g, c]) = h["chatmix"].as_array().map(|a| [a.first(), a.get(1)]) {
        let n = |v: Option<&Value>| v.and_then(|v| v.as_u64()).unwrap_or(0).to_string();
        out.push(fmt(app, "ChatMix game {0} / chat {1}", &[&n(g), &n(c)]));
    }
    out.join(" · ")
}

fn minutes(app: &AppHandle, m: i64) -> String {
    if m == 0 { tr(app, "Never") } else { fmt(app, "{0} min", &[&m.to_string()]) }
}

fn card(app: &AppHandle, h: &Value) -> HsCard {
    let answered = h["answered"].as_bool().unwrap_or(false);
    let connected = h["connected"].as_bool().unwrap_or(false);
    let (mark, state) = if !answered {
        ("warn", "Not answering")
    } else if !connected {
        ("off", "Headset off")
    } else {
        ("on", "Connected")
    };
    let controls: Vec<HsControl> = h["controls"]
        .as_array()
        .map(|l| l.iter().map(|c| control(app, c)).collect())
        .unwrap_or_default();
    let eq = &h["eq"];
    let bands: Vec<i32> = eq["bands"]
        .as_array()
        .map(|b| b.iter().map(|v| v.as_i64().unwrap_or(0) as i32).collect())
        .unwrap_or_else(|| vec![0; 10]);
    HsCard {
        key: s(&h["key"]),
        name: s(&h["name"]),
        mark: mark.into(),
        state: tr(app, state).into(),
        facts: facts(app, h).into(),
        off: !connected,
        controls: ModelRc::new(VecModel::from(controls)),
        eq: eq.is_object(),
        bands: ModelRc::new(VecModel::from(bands)),
        eq_min: eq["min"].as_i64().unwrap_or(-12) as i32,
        eq_max: eq["max"].as_i64().unwrap_or(12) as i32,
        eq_note: tr(
            app,
            if eq["bands"].is_array() { "Last set by FoxDen" } else { "Not set yet" },
        )
        .into(),
    }
}

fn control(app: &AppHandle, c: &Value) -> HsControl {
    let id = c["id"].as_str().unwrap_or_default();
    let shape = &c["shape"];
    let value = c["value"].as_i64();
    let mut out = HsControl {
        id: id.into(),
        label: label_of(app, id).into(),
        value: value.unwrap_or(0) as i32,
        set: value.is_some(),
        remembered: value.is_some() && !c["read"].as_bool().unwrap_or(false),
        ..Default::default()
    };
    let mut options: Vec<(i64, String)>;
    match shape["type"].as_str().unwrap_or_default() {
        "steps" => {
            out.kind = "steps".into();
            out.min = shape["min"].as_i64().unwrap_or(0) as i32;
            out.max = shape["max"].as_i64().unwrap_or(0) as i32;
            return out;
        }
        "switch" => {
            out.kind = "switch".into();
            out.readout = if value.is_none() { tr(app, "Not set").into() } else { "".into() };
            return out;
        }
        "minutes" => {
            out.kind = "select".into();
            options = shape["presets"]
                .as_array()
                .map(|l| l.iter().filter_map(|m| m.as_i64()).map(|m| (m, minutes(app, m))).collect())
                .unwrap_or_default();
            if let Some(v) = value
                && !options.iter().any(|(o, _)| *o == v)
            {
                options.push((v, minutes(app, v)));
                options.sort_by_key(|o| o.0);
            }
        }
        _ => {
            options = shape["options"]
                .as_array()
                .map(|l| {
                    l.iter()
                        .filter_map(|o| Some((o[0].as_i64()?, tr(app, o[1].as_str()?))))
                        .collect()
                })
                .unwrap_or_default();
            if options.len() <= 4 {
                out.kind = "seg".into();
                out.readout = match value {
                    None => "—".into(),
                    Some(4) if id == "eq_preset" => tr(app, "Custom").into(),
                    _ => "".into(),
                };
            } else {
                out.kind = "select".into();
            }
        }
    }
    if out.kind == "select" && value.is_none() {
        options.insert(0, (UNSET as i64, tr(app, "Not set")));
    }
    out.index = options
        .iter()
        .position(|(o, _)| value.map_or(*o == UNSET as i64, |v| *o == v))
        .unwrap_or(0) as i32;
    out.options = ModelRc::new(VecModel::from(
        options.iter().map(|(_, n)| n.as_str().into()).collect::<Vec<slint::SharedString>>(),
    ));
    out.values = ModelRc::new(VecModel::from(options.iter().map(|(v, _)| *v as i32).collect::<Vec<i32>>()));
    out
}

pub fn event(w: &AppWindow, app: &AppHandle, view: &mut View, name: &str) {
    if name != "chatmix" {
        return;
    }
    let d = &view.events["chatmix"];
    let (Some(game), Some(chat)) = (d["game"].as_u64(), d["chat"].as_u64()) else {
        return;
    };
    let cards = w.global::<DevicesPage>().get_headsets();
    HS_VIEWS.with_borrow_mut(|views| {
        for h in views.as_array_mut().into_iter().flatten() {
            if h["chatmix"].is_null() {
                continue;
            }
            h["chatmix"] = json!([game, chat]);
            for i in 0..cards.row_count() {
                if let Some(mut c) = cards.row_data(i)
                    && c.key.as_str() == h["key"].as_str().unwrap_or_default()
                {
                    c.facts = facts(app, h).into();
                    cards.set_row_data(i, c);
                }
            }
        }
    });
}

pub fn send_device(app: &AppHandle, d: &Dev, enabled: bool, brightness: f32, rect: [f32; 4]) {
    report(commands::set_device(
        app.clone(),
        shared(app),
        d.id.clone(),
        enabled,
        brightness,
        rect,
    ));
}

/// The selected device's place in the exact fields, following a drag.
pub fn show_place(w: &AppWindow, view: &View) {
    let g = w.global::<DevicesPage>();
    let rect = view
        .canvas
        .selected
        .as_deref()
        .and_then(|id| view.canvas.find(id))
        .map(|d| view.canvas.rect_of(d));
    let place: Vec<slint::SharedString> = match rect {
        Some(r) => r.iter().map(|v| format!("{:.1}", v * 100.0).into()).collect(),
        None => vec!["".into(); 4],
    };
    fill(g.get_place(), place, |m| g.set_place(m));
}

/// True once for each new snapshot, so local refreshes do not count as one.
pub fn fresh(snap: &Value) -> bool {
    let id = snap["catalog"].as_array().map_or(0, |a| a.as_ptr() as usize);
    SNAP_ID.with(|s| s.replace(id) != id)
}

pub fn apply(w: &AppWindow, app: &AppHandle, view: &mut View) {
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
    let raw: HashMap<&str, &Value> = snap["devices"]
        .as_array()
        .map(|l| l.iter().filter_map(|d| Some((d["id"].as_str()?, d))).collect())
        .unwrap_or_default();
    fill(
        g.get_devices(),
        view.canvas
            .devices
            .iter()
            .map(|d| {
                let backend = raw.get(d.id.as_str()).and_then(|r| r["backend"].as_str()).unwrap_or_default();
                let meta = [
                    tr(app, &backend_name(backend)),
                    tr(app, &kind_name(&d.kind)),
                    tr(app, &plural(d.leds.len(), "LED")),
                ]
                .join(" · ");
                DeviceRow {
                    id: d.id.as_str().into(),
                    name: d.name.as_str().into(),
                    status: tr(app, if d.enabled { "On" } else { "Off" }).into(),
                    meta: meta.into(),
                    switch: fmt(app, "Enable {0}", &[&d.name]).into(),
                    enabled: d.enabled,
                    brightness: d.brightness,
                    selected: view.canvas.selected.as_ref() == Some(&d.id),
                }
            })
            .collect(),
        |m| g.set_devices(m),
    );
    let zones: Vec<Zone> = view
        .canvas
        .selected
        .as_deref()
        .and_then(|id| raw.get(id))
        .and_then(|d| d["zones"].as_array())
        .map(|l| {
            l.iter()
                .map(|z| Zone {
                    key: z["key"].as_str().unwrap_or_default().into(),
                    name: z["name"].as_str().unwrap_or_default().into(),
                    min: z["min"].as_u64().unwrap_or(0) as usize,
                    max: z["max"].as_u64().unwrap_or(0) as usize,
                })
                .collect()
        })
        .unwrap_or_default();
    let leds = |key: &str| {
        view.canvas
            .selected
            .as_deref()
            .and_then(|id| raw.get(id))
            .and_then(|d| d["zones"].as_array())
            .and_then(|l| l.iter().find(|z| z["key"] == key))
            .and_then(|z| z["leds"].as_u64())
            .unwrap_or(0)
    };
    fill(
        g.get_zones(),
        zones
            .iter()
            .map(|z| ZoneRow {
                key: z.key.as_str().into(),
                name: z.name.as_str().into(),
                leds: leds(&z.key).to_string().into(),
            })
            .collect(),
        |m| g.set_zones(m),
    );
    ZONES.with_borrow_mut(|zs| *zs = zones);
    show_place(w, view);
    g.set_auto_detect(snap["settings"]["auto_detect"].as_bool().unwrap_or(false));
    fill(
        g.get_detected(),
        snap["detected"]
            .as_array()
            .map(|l| l.iter().map(|d| found(app, d)).collect())
            .unwrap_or_default(),
        |m| g.set_detected(m),
    );
}

fn found(app: &AppHandle, d: &Value) -> FoundRow {
    let state = d["state"].as_str().unwrap_or_default();
    let (mark, label) = match state {
        "active" => ("on", tr(app, "Active")),
        "available" => ("off", tr(app, "Available")),
        "unsupported" => ("none", tr(app, "No lighting control")),
        "battery" => ("on", tr(app, "Battery and settings")),
        s => ("none", tr(app, &words(s))),
    };
    let route = d["route"].as_str().unwrap_or_default();
    let mut about = vec![tr(app, &kind_name(d["kind"].as_str().unwrap_or_default()))];
    match d["bus"].as_str() {
        Some("usb") => about.push(tr(app, "USB")),
        Some("network") => about.push(tr(app, "Network")),
        _ => {}
    }
    match route {
        "native" => about.push(tr(app, "FoxDen")),
        "none" | "" => {}
        r => about.push(backend_name(r)),
    }
    FoundRow {
        id: s(&d["id"]),
        name: s(&d["name"]),
        mark: mark.into(),
        state: label.into(),
        meta: about.join(" · ").into(),
        note: s(&d["note"]),
        setting: if state == "available" && route == "native" {
            fmt(app, "{0} setting", &[&tr(app, "Native driver")]).into()
        } else {
            "".into()
        },
    }
}
