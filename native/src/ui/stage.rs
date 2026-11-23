use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::time::Duration;

use serde_json::Value;
use slint::{ComponentHandle, Image, Timer, TimerMode};
use tauri::AppHandle;

use super::devices::{self, send_device};
use super::{
    AppWindow, CanvasDot, CanvasLabel, SettingsPage, StagePage, View, fill, refresh, report, shared, tr, with_view,
    with_win,
};
use crate::canvas::ViewOpts;
use crate::commands;

const GRIDS: [f32; 3] = [0.025, 0.05, 0.1];
const NAMES: [(&str, &str); 5] = [
    ("hid", "Native HID"),
    ("board", "Boards, memory and cards"),
    ("net", "Network lights"),
    ("audio", "Audio capture"),
    ("screen", "Screen capture"),
];

thread_local! {
    static FRAMES: Cell<u32> = const { Cell::new(0) };
    static FPS: RefCell<Option<Timer>> = const { RefCell::new(None) };
    static KNOWN: RefCell<Option<HashMap<String, String>>> = const { RefCell::new(None) };
    static LISTED: Cell<bool> = const { Cell::new(false) };
    static LAST_PROFILE: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// A backend's name as the Settings tab shows it.
pub fn backend_name(id: &str) -> String {
    NAMES
        .iter()
        .find(|(k, _)| *k == id)
        .map(|(_, v)| v.to_string())
        .unwrap_or_else(|| id.to_string())
}

/// Translates a template such as "Profile {0}" and fills it in.
pub fn fmt(app: &AppHandle, template: &str, args: &[&str]) -> String {
    let mut out = tr(app, template);
    for (i, a) in args.iter().enumerate() {
        out = out.replace(&format!("{{{i}}}"), a);
    }
    out
}

/// A count with its noun, as the web UI writes it: "1 LED", "1,204 LEDs".
pub fn plural(n: usize, word: &str) -> String {
    let digits = n.to_string();
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(c);
    }
    format!("{grouped} {word}{}", if n == 1 { "" } else { "s" })
}

pub fn sentence(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

pub fn note(w: &AppWindow, text: &str) {
    w.global::<StagePage>().set_note(text.into());
}

pub fn show_error(w: &AppWindow, text: &str) {
    w.global::<StagePage>().set_error(sentence(text).into());
}

fn view_file() -> std::path::PathBuf {
    crate::config::dir().join("view.json")
}

fn load_view() -> ViewOpts {
    let mut v: ViewOpts = std::fs::read(view_file())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    if !GRIDS.contains(&v.grid) {
        v.grid = 0.05;
    }
    v
}

fn set_view(app: &AppHandle, change: impl FnOnce(&mut ViewOpts)) {
    let opts = with_view(|v| {
        change(&mut v.canvas.view);
        v.canvas.view.clone()
    });
    if let Ok(text) = serde_json::to_vec_pretty(&opts) {
        let _ = std::fs::create_dir_all(crate::config::dir());
        if let Err(e) = std::fs::write(view_file(), text) {
            crate::log::warn("ui", format!("the view options could not be saved: {e}"));
        }
    }
    refresh(app);
}

fn percent(app: &AppHandle, what: &str, text: &str) -> Result<f32, String> {
    let v = text.trim().parse::<f32>().ok().filter(|v| (0.0..=100.0).contains(v));
    v.map(|v| (v.round() / 100.0).clamp(0.0, 1.0)).ok_or_else(|| {
        fmt(
            app,
            "{0} must be from {1} to {2}{3}.",
            &[&tr(app, what), "0", "100", "%"],
        )
    })
}

pub fn bind(w: &AppWindow, app: &AppHandle) {
    let g = w.global::<StagePage>();
    with_view(|v| {
        v.canvas.view = load_view();
        v.canvas.invalidate();
    });
    let a = app.clone();
    g.on_set_brightness(move |v| report(commands::set_brightness(a.clone(), shared(&a), v)));
    let a = app.clone();
    g.on_brightness_typed(move |text| {
        let out = percent(&a, "Brightness", &text);
        with_win(|w| {
            w.global::<StagePage>()
                .set_problem(out.as_ref().err().cloned().unwrap_or_default().into())
        });
        match out {
            Ok(v) => {
                report(commands::set_brightness(a.clone(), shared(&a), v));
                true
            }
            Err(_) => false,
        }
    });
    g.on_clear_problem(|| with_win(|w| w.global::<StagePage>().set_problem("".into())));
    let a = app.clone();
    g.on_toggle_pause(move || {
        let paused = with_view(|v| v.snap["paused"].as_bool().unwrap_or(false));
        report(commands::set_paused(a.clone(), shared(&a), !paused));
    });
    let a = app.clone();
    g.on_reset_layout(move || {
        let r = commands::reset_layout(a.clone(), shared(&a));
        if r.is_ok() {
            let text = tr(&a, "Layout reset");
            with_win(|w| note(w, &text));
        }
        report(r);
    });
    let a = app.clone();
    g.on_canvas_down(move |x, y| {
        with_view(|v| v.canvas.down(x, y));
        refresh(&a);
    });
    g.on_canvas_move(move |x, y, pressed| {
        with_view(|v| {
            if v.canvas.moved(x, y, pressed, false) {
                with_win(|w| redraw(w, v));
            }
            let cursor = v.canvas.cursor;
            with_win(|w| w.global::<StagePage>().set_cursor(cursor));
        });
    });
    g.on_canvas_drag(move |x, y, alt| {
        with_view(|v| {
            if v.canvas.moved(x, y, true, alt) {
                with_win(|w| redraw(w, v));
            }
        });
    });
    g.on_canvas_leave(|| {
        with_view(|v| {
            if v.canvas.leave() {
                with_win(|w| redraw(w, v));
            }
            with_win(|w| w.global::<StagePage>().set_cursor(0));
        });
    });
    let a = app.clone();
    g.on_canvas_up(move || {
        if let Some((d, rect)) = with_view(|v| v.canvas.up()) {
            send_device(&a, &d, d.enabled, d.brightness, rect);
        }
    });
    let a = app.clone();
    g.on_canvas_key(move |key, ctrl, shift| canvas_key(&a, &key, ctrl, shift));
    let a = app.clone();
    g.on_set_snap(move |on| set_view(&a, |v| v.snap = on));
    let a = app.clone();
    g.on_set_grid(move |i| {
        set_view(&a, |v| {
            v.grid = GRIDS[i.clamp(0, 2) as usize];
            v.snap = true;
        })
    });
    let a = app.clone();
    g.on_set_labels(move |on| set_view(&a, |v| v.labels = on));
    let a = app.clone();
    g.on_set_shapes(move |on| set_view(&a, |v| v.shapes = on));
    let weak = w.as_weak();
    g.on_show_backends(move || {
        if let Some(w) = weak.upgrade() {
            w.set_tab(5);
        }
    });

    let weak = w.as_weak();
    let a = app.clone();
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_secs(1), move || {
        let n = FRAMES.with(|f| f.replace(0));
        if let Some(w) = weak.upgrade() {
            w.global::<StagePage>()
                .set_fps(fmt(&a, "{0} fps", &[&n.to_string()]).into());
        }
    });
    FPS.with_borrow_mut(|t| *t = Some(timer));
}

/// Stops the fps timer once the window is gone; `bind` starts it again.
pub fn stop() {
    let _ = FPS.try_with(|t| t.take());
}

fn canvas_key(app: &AppHandle, key: &str, ctrl: bool, shift: bool) -> bool {
    if let Some((d, rect)) = with_view(|v| v.canvas.key(key, ctrl, shift)) {
        send_device(app, &d, d.enabled, d.brightness, rect);
        return true;
    }
    if ctrl {
        return false;
    }
    let keys = with_view(|v| v.canvas.view.keys);
    match key {
        "\u{1b}" => {
            with_view(|v| v.canvas.selected = None);
            refresh(app);
        }
        "[" | "]" if keys => {
            if let Some(name) = with_view(|v| v.canvas.select_next(if key == "]" { 1 } else { -1 })) {
                refresh(app);
                let text = fmt(app, "Selected {0}", &[&name]);
                with_win(|w| note(w, &text));
            }
        }
        "g" | "G" if keys => {
            let on = !with_view(|v| v.canvas.view.snap);
            set_view(app, |v| v.snap = on);
            let text = tr(app, if on { "Snap to grid on" } else { "Snap to grid off" });
            with_win(|w| note(w, &text));
        }
        "l" | "L" if keys => {
            let on = !with_view(|v| v.canvas.view.labels);
            set_view(app, |v| v.labels = on);
            let text = tr(app, if on { "Labels shown" } else { "Labels hidden" });
            with_win(|w| note(w, &text));
        }
        " " if keys => {
            let paused = with_view(|v| v.snap["paused"].as_bool().unwrap_or(false));
            report(commands::set_paused(app.clone(), shared(app), !paused));
        }
        _ => return false,
    }
    true
}

pub fn event(w: &AppWindow, app: &AppHandle, _view: &mut View, name: &str) {
    if name == "update" {
        update_label(w, app);
    }
}

fn update_label(w: &AppWindow, app: &AppHandle) {
    let version = w.global::<SettingsPage>().get_update_version();
    w.global::<StagePage>()
        .set_update_label(fmt(app, "Update to {0}", &[&version]).into());
}

/// Says which devices came or went since the last snapshot.
fn note_changes(w: &AppWindow, app: &AppHandle, snap: &Value) {
    let empty = Vec::new();
    let detected = snap["detected"].as_array().unwrap_or(&empty);
    let devices = snap["devices"].as_array().unwrap_or(&empty);
    let now: HashMap<String, String> = detected
        .iter()
        .chain(devices)
        .filter_map(|d| Some((d["id"].as_str()?.to_string(), d["name"].as_str()?.to_string())))
        .collect();
    KNOWN.with_borrow_mut(|known| {
        let listed = LISTED.get();
        if let Some(k) = known.as_mut()
            && !listed
        {
            for d in detected {
                if let (Some(id), Some(name)) = (d["id"].as_str(), d["name"].as_str()) {
                    k.insert(id.into(), name.into());
                }
            }
        }
        LISTED.set(listed || !detected.is_empty());
        if let Some(k) = known.as_ref() {
            let names = |from: &HashMap<String, String>, to: &HashMap<String, String>| {
                let mut out: Vec<String> = Vec::new();
                for (id, name) in from {
                    if !to.contains_key(id) && !out.contains(name) {
                        out.push(name.clone());
                    }
                }
                out.sort();
                out
            };
            let list = |found: &[String]| {
                if found.len() > 3 {
                    plural(found.len(), "device")
                } else {
                    found.join(", ")
                }
            };
            let added = names(&now, k);
            let gone = names(k, &now);
            if !added.is_empty() {
                note(w, &fmt(app, "Added: {0}", &[&list(&added)]));
            } else if !gone.is_empty() {
                note(w, &fmt(app, "Removed: {0}", &[&list(&gone)]));
            }
        }
        *known = Some(now);
    });
}

pub fn apply(w: &AppWindow, app: &AppHandle, view: &mut View) {
    let g = w.global::<StagePage>();
    let snap = &view.snap;
    g.set_brightness(snap["brightness"].as_f64().unwrap_or(1.0) as f32);
    let paused = snap["paused"].as_bool().unwrap_or(false);
    g.set_paused(paused);
    let lighting = snap["settings"]["lighting_module"].as_bool().unwrap_or(true);
    let (mark, state) = if !lighting {
        ("off", "Lighting off")
    } else if paused {
        ("off", "Paused")
    } else {
        ("on", "Running")
    };
    g.set_state_mark(mark.into());
    g.set_status(tr(app, state).into());
    let leds: usize = view.canvas.devices.iter().map(|d| d.leds.len()).sum();
    g.set_count(
        tr(
            app,
            &format!("{}, {}", plural(view.canvas.devices.len(), "device"), plural(leds, "LED")),
        )
        .into(),
    );
    let profiles: Vec<&str> = snap["profiles"]
        .as_array()
        .map(|l| l.iter().filter_map(|p| p.as_str()).collect())
        .unwrap_or_default();
    let profile = LAST_PROFILE.with_borrow_mut(|last| {
        if let Some(p) = snap["active_profile"].as_str() {
            *last = Some(p.to_string());
            return Some(p.to_string());
        }
        if last.as_deref().is_some_and(|l| !profiles.contains(&l)) {
            *last = None;
        }
        last.as_ref().map(|l| format!("{l}, edited"))
    });
    g.set_profile(
        match profile {
            Some(p) => fmt(app, "Profile {0}", &[&p]),
            None => tr(app, "No profile"),
        }
        .into(),
    );
    let broken: Vec<&str> = snap["backends"]
        .as_array()
        .map(|l| {
            l.iter()
                .filter(|b| b["error"].as_str().is_some())
                .filter_map(|b| b["name"].as_str())
                .collect()
        })
        .unwrap_or_default();
    g.set_backend_problem(
        match broken.len() {
            0 => String::new(),
            1 => fmt(app, "{0} error", &[&tr(app, &backend_name(broken[0]))]),
            n => fmt(app, "{0} backend errors", &[&n.to_string()]),
        }
        .into(),
    );
    if devices::fresh(snap) {
        note_changes(w, app, snap);
        devices::settle_detect(w, app);
    }
    update_label(w, app);
    let opts = &view.canvas.view;
    g.set_snap(opts.snap);
    g.set_grid(GRIDS.iter().position(|v| *v == opts.grid).unwrap_or(1) as i32);
    g.set_show_labels(opts.labels);
    g.set_shapes(opts.shapes);
    redraw(w, view);
}

pub fn frame(w: &AppWindow, view: &mut View, frame: &Value) {
    let Some(frames) = frame.as_array() else {
        return;
    };
    FRAMES.with(|f| f.set(f.get() + 1));
    let colors: HashMap<String, Vec<[u8; 3]>> = frames
        .iter()
        .filter_map(|f| {
            let id = f["device"].as_str()?.to_string();
            Some((id, serde_json::from_value(f["colors"].clone()).ok()?))
        })
        .collect();
    if colors != view.canvas.colors {
        view.canvas.colors = colors;
        redraw(w, view);
    }
}

pub fn redraw(w: &AppWindow, view: &mut View) {
    let g = w.global::<StagePage>();
    g.set_readout(view.canvas.readout().into());
    devices::show_place(w, view);
    let (cw, ch) = (w.get_canvas_width(), w.get_canvas_height());
    if cw < 50.0 || ch < 50.0 {
        return;
    }
    let scale = w.window().scale_factor();
    let Some(out) = view.canvas.render(cw, ch, scale) else {
        return;
    };
    if let Some(buf) = out.image {
        g.set_canvas(Image::from_rgba8_premultiplied(buf));
    }
    fill(
        g.get_dots(),
        out.dots
            .iter()
            .map(|d| CanvasDot {
                x: d.x,
                y: d.y,
                w: d.w,
                h: d.h,
                round: d.round,
                rim: d.rim,
                color: slint::Color::from_rgb_u8(d.color[0], d.color[1], d.color[2]),
            })
            .collect(),
        |m| g.set_dots(m),
    );
    fill(
        g.get_labels(),
        out.labels
            .into_iter()
            .map(|l| CanvasLabel {
                x: l.x,
                y: l.y,
                width: l.width,
                text: l.text.into(),
                strong: l.strong,
            })
            .collect(),
        |m| g.set_labels(m),
    );
}
