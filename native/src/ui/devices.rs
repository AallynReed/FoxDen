use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use slint::{ComponentHandle, Model, ModelRc, Timer, TimerMode, VecModel};
use tauri::AppHandle;

use super::stage::{backend_name, fmt, note, plural, sentence, show_error};
use super::{
    AppWindow, DeviceRow, DevicesPage, FoundRow, HsCard, HsControl, HwRange, StagePage, View, ZoneRow, edit_settings, fill,
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

const HARDWARE_MODES: [(&str, &str); 18] = [
    ("breathing", "Breathing"),
    ("color_cycle", "Colour cycle"),
    ("star", "Star"),
    ("follow_board", "Follow motherboard"),
    ("static", "Static"),
    ("flashing", "Flashing"),
    ("double_flashing", "Double flashing"),
    ("rainbow", "Rainbow"),
    ("random", "Random"),
    ("lightning", "Lightning"),
    ("meteor", "Meteor"),
    ("fire", "Fire"),
    ("ripple", "Ripple"),
    ("wave", "Wave"),
    ("reload", "Reload"),
    ("recoil", "Recoil"),
    ("refill", "Refill"),
    ("demo", "Demo"),
];

struct Zone {
    key: String,
    name: String,
    min: usize,
    max: usize,
    /// LEDs per fan for a zone set in whole fans, 1 for a free count.
    per: usize,
}

/// The selected device's own lighting as the panel shows it.
#[derive(Default)]
struct Hw {
    id: String,
    modes: Vec<Value>,
    now: Option<Value>,
    last: Option<Value>,
    at: Option<Instant>,
    pending: Option<(String, Option<Value>)>,
}

thread_local! {
    static HW: RefCell<Hw> = RefCell::new(Hw::default());
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

/// Whether the device runs one of its own effects.
fn own_effect(d: &Value) -> bool {
    let Some(mode) = d["hardware_setting"]["mode"].as_str() else {
        return false;
    };
    d["hardware"]["modes"]
        .as_array()
        .is_some_and(|l| l.iter().any(|m| m["id"] == mode))
}

/// The values a hardware mode runs with when the setting leaves them out, as the app fills them in.
fn hardware_values(mode: &Value, setting: Option<&Value>) -> Value {
    let id = mode["id"].as_str().unwrap_or_default();
    let blank = json!({ "mode": id });
    let s = setting.filter(|s| s["mode"] == id).unwrap_or(&blank);
    let fit = |key: &str, default: fn(i64, i64) -> i64| -> Value {
        let Some(r) = mode[key].as_array() else {
            return Value::Null;
        };
        let lo = r.first().and_then(|v| v.as_i64()).unwrap_or(0);
        let hi = r.get(1).and_then(|v| v.as_i64()).unwrap_or(lo).max(lo);
        json!(s[key].as_i64().unwrap_or_else(|| default(lo, hi)).clamp(lo, hi))
    };
    let n = mode["colors"].as_u64().unwrap_or(0) as usize;
    let colors: Vec<Value> = (0..n)
        .map(|i| match &s["colors"][i] {
            c if c.is_array() => c.clone(),
            _ => json!([255, 255, 255]),
        })
        .collect();
    let dirs = mode["directions"].as_array().cloned().unwrap_or_default();
    let direction = if dirs.contains(&s["direction"]) {
        s["direction"].clone()
    } else {
        dirs.first().cloned().unwrap_or(Value::Null)
    };
    let mut out = json!({
        "mode": id,
        "colors": colors,
        "random": mode["random"].as_bool().unwrap_or(false) && s["random"].as_bool().unwrap_or(false),
    });
    for (key, v) in [
        ("speed", fit("speed", |lo, hi| lo + (hi - lo) / 2)),
        ("brightness", fit("brightness", |_, hi| hi)),
        ("direction", direction),
    ] {
        if !v.is_null() {
            out[key] = v;
        }
    }
    out
}

fn rgb(v: &Value) -> [u8; 3] {
    [0, 1, 2].map(|i| v[i].as_u64().unwrap_or(0).min(255) as u8)
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2])
}

fn from_hex(text: &str) -> Option<[u8; 3]> {
    let t = text.trim();
    let t = t.strip_prefix('#').unwrap_or(t);
    if !t.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let six: String = match t.len() {
        3 => t.chars().flat_map(|c| [c, c]).collect(),
        6 => t.to_string(),
        _ => return None,
    };
    let byte = |i: usize| u8::from_str_radix(&six[i..i + 2], 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

/// Shows the lighting choice and the fields of the mode in `values`.
fn render_hw(g: &DevicesPage, app: &AppHandle, modes: &[Value], values: Option<&Value>) {
    let names: Vec<slint::SharedString> = if modes.is_empty() {
        Vec::new()
    } else {
        std::iter::once(tr(app, "FoxDen effects"))
            .chain(modes.iter().map(|m| {
                let id = m["id"].as_str().unwrap_or_default();
                let name = HARDWARE_MODES
                    .iter()
                    .find(|(k, _)| *k == id)
                    .map(|(_, v)| v.to_string())
                    .unwrap_or_else(|| m["name"].as_str().unwrap_or(id).to_string());
                tr(app, &name)
            }))
            .map(Into::into)
            .collect()
    };
    fill(g.get_hw_modes(), names, |m| g.set_hw_modes(m));
    let mode = values.and_then(|v| modes.iter().find(|m| m["id"] == v["mode"]));
    let index = values
        .and_then(|v| modes.iter().position(|m| m["id"] == v["mode"]))
        .map_or(0, |i| i + 1);
    g.set_hw_index(index as i32);
    let (Some(mode), Some(v)) = (mode, values) else {
        g.set_hw_speed(Default::default());
        g.set_hw_bright(Default::default());
        fill(g.get_hw_colors(), Vec::new(), |m| g.set_hw_colors(m));
        fill(g.get_hw_dirs(), Vec::new(), |m| g.set_hw_dirs(m));
        g.set_hw_random_on(false);
        return;
    };
    let range = |key: &str| match mode[key].as_array() {
        Some(r) => HwRange {
            on: true,
            min: r.first().and_then(|x| x.as_i64()).unwrap_or(0) as i32,
            max: r.get(1).and_then(|x| x.as_i64()).unwrap_or(0) as i32,
            value: v[key].as_i64().unwrap_or(0) as i32,
        },
        None => HwRange::default(),
    };
    g.set_hw_speed(range("speed"));
    g.set_hw_bright(range("brightness"));
    let colors: Vec<[u8; 3]> = v["colors"].as_array().map(|l| l.iter().map(rgb).collect()).unwrap_or_default();
    let many = colors.len() > 1;
    fill(
        g.get_hw_colors(),
        colors.iter().map(|c| slint::Color::from_rgb_u8(c[0], c[1], c[2])).collect(),
        |m| g.set_hw_colors(m),
    );
    fill(g.get_hw_hexes(), colors.iter().map(|c| hex(*c).into()).collect(), |m| g.set_hw_hexes(m));
    fill(
        g.get_hw_color_labels(),
        (0..colors.len())
            .map(|i| {
                if many {
                    fmt(app, "Colour {0}", &[&(i + 1).to_string()]).into()
                } else {
                    tr(app, "Colour").into()
                }
            })
            .collect(),
        |m| g.set_hw_color_labels(m),
    );
    let dirs: Vec<&str> = mode["directions"]
        .as_array()
        .map(|l| l.iter().filter_map(|d| d.as_str()).collect())
        .unwrap_or_default();
    fill(g.get_hw_dirs(), dirs.iter().map(|d| tr(app, &words(d)).into()).collect(), |m| g.set_hw_dirs(m));
    g.set_hw_dir(dirs.iter().position(|d| v["direction"] == *d).unwrap_or(0) as i32);
    g.set_hw_random_on(mode["random"].as_bool().unwrap_or(false));
    g.set_hw_random(v["random"].as_bool().unwrap_or(false));
}

/// Sends a lighting choice made here, at most once a frame, and keeps its echo from overriding it.
fn set_hw(app: &AppHandle, setting: Option<Value>) {
    let idle = HW.with_borrow_mut(|hw| {
        hw.at = Some(Instant::now());
        if setting.is_some() {
            hw.last = setting.clone();
        }
        hw.now = setting.clone();
        let modes = hw.modes.clone();
        with_win(|w| render_hw(&w.global::<DevicesPage>(), app, &modes, setting.as_ref()));
        hw.pending.replace((hw.id.clone(), setting)).is_none()
    });
    if !idle {
        return;
    }
    let a = app.clone();
    Timer::single_shot(Duration::from_millis(16), move || {
        let Some((id, setting)) = HW.with_borrow_mut(|hw| hw.pending.take()) else {
            return;
        };
        let parsed = match setting.map(serde_json::from_value).transpose() {
            Ok(p) => p,
            Err(e) => return report(Err(e.to_string())),
        };
        report(commands::set_hardware(a.clone(), shared(&a), id, parsed));
    });
}

/// Changes one value of the shown mode and sends it.
fn edit_hw(app: &AppHandle, change: impl FnOnce(&mut Value)) {
    let Some(mut now) = HW.with_borrow(|hw| hw.now.clone()) else {
        return;
    };
    change(&mut now);
    set_hw(app, Some(now));
}

fn bind_hw(g: &DevicesPage, app: &AppHandle) {
    let a = app.clone();
    g.on_hw_mode(move |i| {
        let setting = HW.with_borrow(|hw| {
            let mode = hw.modes.get((i as usize).checked_sub(1)?)?;
            let mut base = hw.last.clone().unwrap_or_else(|| json!({}));
            base["mode"] = mode["id"].clone();
            Some(hardware_values(mode, Some(&base)))
        });
        set_hw(&a, setting);
    });
    let a = app.clone();
    g.on_hw_range(move |n, v| {
        let key = if n == 0 { "speed" } else { "brightness" };
        edit_hw(&a, |s| s[key] = json!(v));
    });
    let a = app.clone();
    g.on_hw_range_typed(move |n, text| {
        let (key, label) = if n == 0 { ("speed", "Speed") } else { ("brightness", "Brightness") };
        let range = HW.with_borrow(|hw| {
            let now = hw.now.as_ref()?;
            let mode = hw.modes.iter().find(|m| m["id"] == now["mode"])?;
            let r = mode[key].as_array()?;
            Some((r.first()?.as_i64()?, r.get(1)?.as_i64()?))
        });
        let Some((lo, hi)) = range else {
            return false;
        };
        match text.trim().parse::<i64>().ok().filter(|v| (lo..=hi).contains(v)) {
            Some(v) => {
                problem("");
                edit_hw(&a, |s| s[key] = json!(v));
                true
            }
            None => {
                problem(&fmt(
                    &a,
                    "{0} must be from {1} to {2}{3}.",
                    &[&tr(&a, label), &lo.to_string(), &hi.to_string(), ""],
                ));
                false
            }
        }
    });
    let a = app.clone();
    g.on_hw_color(move |i, c| {
        edit_hw(&a, |s| {
            if let Some(slot) = s["colors"].get_mut(i as usize) {
                *slot = json!([c.red(), c.green(), c.blue()]);
            }
        })
    });
    let a = app.clone();
    g.on_hw_direction(move |i| {
        let dir = HW.with_borrow(|hw| {
            let now = hw.now.as_ref()?;
            let mode = hw.modes.iter().find(|m| m["id"] == now["mode"])?;
            mode["directions"].get(i as usize).cloned()
        });
        if let Some(dir) = dir {
            edit_hw(&a, |s| s["direction"] = dir);
        }
    });
    let a = app.clone();
    g.on_hw_set_random(move |on| edit_hw(&a, |s| s["random"] = json!(on)));
    g.on_hex_valid(|t| from_hex(&t).is_some());
    g.on_hex_color(|t| {
        let c = from_hex(&t).unwrap_or([0, 0, 0]);
        slint::Color::from_rgb_u8(c[0], c[1], c[2])
    });
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
    bind_hw(&g, app);
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
        let (lo, hi) = (z.min.div_ceil(z.per), z.max / z.per);
        let n = text.trim().parse::<usize>().ok().filter(|n| (lo..=hi).contains(n));
        let why = if z.per > 1 { "{0} takes {1} to {2} fans." } else { "{0} takes {1} to {2} LEDs." };
        problem(&match n {
            Some(_) => String::new(),
            None => fmt(app, why, &[&z.name, &lo.to_string(), &hi.to_string()]),
        });
        n.map(|n| n * z.per)
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
                        own: d["enabled"].as_bool().unwrap_or(true) && own_effect(d),
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
                    resizable: raw
                        .get(d.id.as_str())
                        .and_then(|r| r["zones"].as_array())
                        .is_some_and(|z| !z.is_empty()),
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
                    per: z["step"].as_u64().unwrap_or(1).max(1) as usize,
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
                leds: (leds(&z.key) / z.per as u64).to_string().into(),
                suffix: if z.per > 1 {
                    fmt(app, "fans × {0} LEDs per fan", &[&z.per.to_string()]).into()
                } else {
                    "".into()
                },
            })
            .collect(),
        |m| g.set_zones(m),
    );
    g.set_zones_tall(zones.iter().any(|z| z.per > 1));
    ZONES.with_borrow_mut(|zs| *zs = zones);
    view.canvas.own_text = tr(app, "Runs its own effect");
    let selected = view.canvas.selected.as_deref().and_then(|id| raw.get(id).copied());
    HW.with_borrow_mut(|hw| {
        let id = selected.and_then(|d| d["id"].as_str()).unwrap_or_default();
        if hw.id != id {
            *hw = Hw { id: id.into(), ..Default::default() };
        }
        hw.modes = selected
            .and_then(|d| d["hardware"]["modes"].as_array().cloned())
            .unwrap_or_default();
        if hw.at.is_some_and(|t| t.elapsed() < Duration::from_millis(400)) {
            return;
        }
        let setting = selected.filter(|d| own_effect(d)).map(|d| &d["hardware_setting"]);
        if let Some(s) = setting {
            hw.last = Some(s.clone());
        }
        let values = setting.and_then(|s| {
            let mode = hw.modes.iter().find(|m| m["id"] == s["mode"])?;
            Some(hardware_values(mode, Some(s)))
        });
        hw.now = values.clone();
        render_hw(&g, app, &hw.modes, values.as_ref());
    });
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
