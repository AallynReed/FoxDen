use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, VecModel};
use tauri::AppHandle;

use super::effects_keys::{KEY_GROUPS, key_name, key_short, on_pad, vk_from_text};
use super::{
    AppWindow, CheckRow, EParam, EffectPick, EffectsPage, GroupOption, KeyCap, LayerRow, StagePage,
    StopRow, Tile, View, fill, report, s, shared, tr, with_view, with_win,
};
use crate::{commands, key_maps};

const PRESETS: [(&str, &str); 10] = [
    ("Red", "#FF0000"),
    ("Orange", "#FF6A00"),
    ("Yellow", "#FFD000"),
    ("Green", "#00C850"),
    ("Cyan", "#00C8FF"),
    ("Blue", "#0050FF"),
    ("Violet", "#963CFF"),
    ("Pink", "#FF3CA0"),
    ("White", "#FFFFFF"),
    ("Black", "#000000"),
];
const BLENDS: [&str; 5] = ["normal", "add", "screen", "multiply", "tint"];
const SCOPES: [&str; 3] = ["all", "keyboards", "chosen"];
const WHENS: [&str; 3] = ["always", "running", "front"];
const MAX_STAGES: usize = 8;

#[derive(Default)]
struct Hist {
    steps: Vec<Step>,
    at: i32,
    quiet: u32,
    sent: Option<Value>,
    stamp: Option<Instant>,
}

#[derive(Clone)]
struct Step {
    state: Value,
    label: String,
    key: String,
}

struct Ed {
    device: String,
    device_ids: Vec<String>,
    sel: i32,
    search: String,
    shown: Vec<String>,
    last_used: HashMap<String, Value>,
    last_layer: HashMap<String, Value>,
    add_kinds: Vec<Option<(bool, String)>>,
    bad: String,
    problem: (String, String),
    picker: String,
    picker_for: String,
    hsv: [f32; 3],
    rgb: [u8; 3],
    before: [u8; 3],
    picker_bad: i32,
    picker_problem: String,
    recent: Vec<[u8; 3]>,
    stops_at: Option<(String, String)>,
    stops_bad: i32,
    stops_problem: String,
    sensor_ids: Vec<String>,
    show_pad: bool,
    brush: [u8; 3],
    erase: bool,
    painting: BTreeMap<u32, [u8; 3]>,
    paint_layer: i32,
    paint_name: String,
    map_name: String,
    sent_at: Option<Instant>,
    stroke: Option<bool>,
    cursor: u32,
    note: String,
    code: String,
    copy_tick: i32,
    code_reset: i32,
    focus_tick: i32,
    app_typed: String,
    app_list: Vec<String>,
    app_asked: Option<Instant>,
    hist: Hist,
}

impl Default for Ed {
    fn default() -> Self {
        Ed {
            device: String::new(),
            device_ids: Vec::new(),
            sel: -1,
            search: String::new(),
            shown: Vec::new(),
            last_used: HashMap::new(),
            last_layer: HashMap::new(),
            add_kinds: Vec::new(),
            bad: String::new(),
            problem: Default::default(),
            picker: String::new(),
            picker_for: String::new(),
            hsv: [0.0; 3],
            rgb: [0; 3],
            before: [0; 3],
            picker_bad: -1,
            picker_problem: String::new(),
            recent: Vec::new(),
            stops_at: None,
            stops_bad: -1,
            stops_problem: String::new(),
            sensor_ids: Vec::new(),
            show_pad: false,
            brush: [255, 106, 0],
            erase: false,
            painting: BTreeMap::new(),
            paint_layer: -1,
            paint_name: String::new(),
            map_name: String::new(),
            sent_at: None,
            stroke: None,
            cursor: 0x41,
            note: String::new(),
            code: String::new(),
            copy_tick: 0,
            code_reset: 0,
            focus_tick: 0,
            app_typed: String::new(),
            app_list: Vec::new(),
            app_asked: None,
            hist: Hist { at: -1, ..Default::default() },
        }
    }
}

thread_local! {
    static ED: RefCell<Ed> = RefCell::new(Ed::default());
}

fn ed<R>(f: impl FnOnce(&mut Ed) -> R) -> R {
    ED.with_borrow_mut(f)
}

fn redraw(app: &AppHandle) {
    with_view(|v| with_win(|w| apply(w, app, v)));
}

pub fn bind(w: &AppWindow, app: &AppHandle) {
    let g = w.global::<EffectsPage>();

    let a = app.clone();
    g.on_select_layer(move |i| select_layer(&a, i));
    let a = app.clone();
    g.on_toggle_layer(move |i| {
        edit_layers(&a, |l| {
            if let Some(layer) = l.get_mut(i as usize) {
                layer["enabled"] = json!(!layer["enabled"].as_bool().unwrap_or(true));
            }
        })
    });
    let a = app.clone();
    g.on_move_layer(move |from, to| move_layer(&a, from, to));
    let a = app.clone();
    g.on_add_layer(move |i| add_layer(&a, i));

    let a = app.clone();
    g.on_edit_device(move |i| {
        ed(|e| e.device = (i as usize).checked_sub(1).and_then(|i| e.device_ids.get(i)).cloned().unwrap_or_default());
        redraw(&a);
    });
    let a = app.clone();
    g.on_follow_main(move || {
        let id = ed(|e| e.device.clone());
        with_view(|v| {
            if let Some(o) = v.snap["overrides"].as_object_mut() {
                o.remove(&id);
            }
        });
        report(commands::set_override(a.clone(), shared(&a), id, None));
        redraw(&a);
    });
    let a = app.clone();
    g.on_search_changed(move |t| {
        ed(|e| e.search = t.to_string());
        redraw(&a);
    });
    let a = app.clone();
    g.on_search_key(move |key| search_key(&a, &key).into());
    let a = app.clone();
    g.on_pick_effect(move |kind| choose_effect(&a, &kind));
    let a = app.clone();
    g.on_pick_step(move |by| {
        let (shown, kind) = (ed(|e| e.shown.clone()), with_view(|v| edited(v)["kind"].clone()));
        let at = shown.iter().position(|k| kind == k.as_str());
        let next = match at {
            Some(i) => (i as i32 + by).clamp(0, shown.len() as i32 - 1) as usize,
            None => 0,
        };
        if let Some(k) = shown.get(next) {
            choose_effect(&a, k);
        }
    });
    let a = app.clone();
    g.on_reset_effect(move || {
        if let Some(e) = with_view(|v| entry(v, "effect")) {
            set_edited(&a, e["default"].clone());
        }
    });
    let a = app.clone();
    g.on_randomise_effect(move || {
        if let Some(effect) = with_view(randomised) {
            set_edited(&a, effect);
        }
    });

    let a = app.clone();
    g.on_set_range(move |target, name, v| {
        clear_bad_of(&format!("{target}:{name}"));
        set_param(&a, &target, &name, json!(v));
    });
    let a = app.clone();
    g.on_range_text(move |target, name, text| range_text(&a, &target, &name, &text));
    let a = app.clone();
    g.on_set_toggle(move |target, name, v| set_param(&a, &target, &name, json!(v)));
    let a = app.clone();
    g.on_set_choice(move |target, name, i| {
        let value = with_view(|v| {
            let e = entry(v, &target)?;
            let p = e["params"].as_array()?.iter().find(|p| p["name"] == name.as_str())?.clone();
            match p["control"]["type"].as_str()? {
                "sensor" => ed(|e| e.sensor_ids.get(i as usize).map(|id| json!(id))),
                _ => p["control"]["options"].get(i as usize).cloned(),
            }
        });
        if let Some(value) = value {
            set_param(&a, &target, &name, value);
        }
    });
    let a = app.clone();
    g.on_clear_bad(move || {
        ed(|e| {
            e.bad.clear();
            e.problem = Default::default();
            e.stops_bad = -1;
            e.stops_problem.clear();
        });
        redraw(&a);
    });

    let a = app.clone();
    g.on_set_layer_kind(move |i| {
        let Some(Some((post, kind))) = with_view(|v| layer_kinds(v).1.get(i as usize).cloned()) else {
            return;
        };
        let effect = ed(|e| e.last_layer.get(&kind).cloned())
            .or_else(|| with_view(|v| find_entry(v, post, &kind).map(|e| e["default"].clone())));
        if let Some(effect) = effect {
            edit_layer(&a, |l| l["effect"] = effect);
        }
    });
    let a = app.clone();
    g.on_set_blend(move |i| edit_layer(&a, |l| l["blend"] = json!(BLENDS[i as usize % 5])));
    let a = app.clone();
    g.on_set_opacity(move |v| {
        clear_bad_of("layer:opacity");
        edit_layer(&a, |l| l["opacity"] = json!(v / 100.0));
    });
    let a = app.clone();
    g.on_opacity_typed(move |text| {
        let label = tr(&a, "Opacity");
        if let Some(v) = check_number(&a, "layer", "layer:opacity", &label, &text, 0.0, 100.0, 0) {
            edit_layer(&a, |l| l["opacity"] = json!(v / 100.0));
        }
        redraw(&a);
    });
    let a = app.clone();
    g.on_set_scope(move |i| edit_layer(&a, |l| l["scope"] = json!(SCOPES[i as usize % 3])));
    let a = app.clone();
    g.on_set_check(move |id, on| {
        edit_layer(&a, |l| {
            let mut devices: Vec<Value> = l["devices"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|d| d != id.as_str())
                .collect();
            if on {
                devices.push(json!(id.as_str()));
            }
            l["devices"] = json!(devices);
        })
    });
    let a = app.clone();
    g.on_set_when(move |i| edit_layer(&a, |l| l["when"] = json!(WHENS[i as usize % 3])));
    let a = app.clone();
    g.on_app_typed(move |t| {
        ed(|e| e.app_typed = t.to_string());
        let stale = ed(|e| e.app_asked.is_none_or(|t| t.elapsed() > Duration::from_secs(3)));
        if stale {
            ed(|e| e.app_asked = Some(Instant::now()));
            let a2 = a.clone();
            std::thread::spawn(move || {
                let names = crate::apps::list_apps();
                let _ = slint::invoke_from_event_loop(move || {
                    ed(|e| e.app_list = names);
                    redraw(&a2);
                });
            });
        }
        redraw(&a);
    });
    let a = app.clone();
    g.on_add_app(move |name| {
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        edit_layer(&a, |l| {
            let apps = l["apps"].as_array().cloned().unwrap_or_default();
            if !apps.iter().any(|x| app_name(x.as_str().unwrap_or("")) == app_name(&name)) {
                let mut apps = apps;
                apps.push(json!(name));
                l["apps"] = json!(apps);
            }
        });
    });
    let a = app.clone();
    g.on_remove_app(move |n| {
        edit_layer(&a, |l| {
            if let Some(apps) = l["apps"].as_array_mut()
                && (n as usize) < apps.len()
            {
                apps.remove(n as usize);
            }
        })
    });
    let a = app.clone();
    g.on_reset_layer(move || {
        let default = with_view(|v| {
            let sel = ed(|e| e.sel);
            layer_info(v, &v.snap["layers"][sel as usize]["effect"]).0.map(|i| i["default"].clone())
        });
        if let Some(d) = default {
            edit_layer(&a, |l| l["effect"] = d);
        }
    });
    let a = app.clone();
    g.on_copy_layer(move || {
        let i = ed(|e| e.sel);
        if i < 0 {
            return;
        }
        edit_layers(&a, |l| {
            if let Some(layer) = l.get(i as usize).cloned() {
                l.insert(i as usize + 1, layer);
            }
        });
        focus_layer(&a, i + 1);
    });
    let a = app.clone();
    g.on_remove_layer(move || {
        let i = ed(|e| e.sel);
        if i < 0 {
            return;
        }
        edit_layers(&a, |l| {
            if (i as usize) < l.len() {
                l.remove(i as usize);
            }
        });
        let len = with_view(|v| layers_len(v));
        focus_layer(&a, i.min(len - 1));
    });

    let a = app.clone();
    g.on_open_picker(move |pid| open_picker(&a, &pid));
    let a = app.clone();
    g.on_picker_set(move |n, v| {
        ed(|e| {
            e.hsv[n as usize % 3] = v;
            e.picker_bad = -1;
            e.picker_problem.clear();
        });
        let rgb = ed(|e| from_hsv(e.hsv));
        choose(&a, rgb, true);
    });
    let a = app.clone();
    g.on_picker_typed(move |n, text| {
        let (name, max) = [("Hue", 360.0), ("Saturation", 100.0), ("Brightness", 100.0)][n as usize % 3];
        match text.trim().parse::<f32>().ok().filter(|v| (0.0..=max).contains(&v.round())) {
            Some(v) => {
                ed(|e| {
                    e.hsv[n as usize % 3] = v.round();
                    e.picker_bad = -1;
                    e.picker_problem.clear();
                });
                let rgb = ed(|e| from_hsv(e.hsv));
                choose(&a, rgb, true);
            }
            None => {
                let what = tr(&a, name);
                let line = tr(&a, &format!("{what} must be from 0 to {max}."));
                ed(|e| {
                    e.picker_bad = n;
                    e.picker_problem = line;
                });
                redraw(&a);
            }
        }
    });
    let a = app.clone();
    g.on_picker_hex(move |text| match from_hex(&text) {
        Some(rgb) => {
            ed(|e| {
                e.picker_bad = -1;
                e.picker_problem.clear();
            });
            choose(&a, rgb, false);
        }
        None => {
            let line = tr(&a, "Enter a colour as six hex digits, such as #FF6A00.");
            ed(|e| {
                e.picker_bad = 3;
                e.picker_problem = line;
            });
            redraw(&a);
        }
    });
    let a = app.clone();
    g.on_picker_tile(move |hex| {
        if let Some(rgb) = from_hex(&hex) {
            choose(&a, rgb, false);
        }
    });
    let a = app.clone();
    g.on_picker_escape(move || {
        ed(|e| {
            e.picker_bad = -1;
            e.picker_problem.clear();
        });
        redraw(&a);
    });

    let a = app.clone();
    g.on_stop_temp(move |i, text| {
        let v = text.trim().parse::<f64>().ok().filter(|v| v.is_finite() && (0.0..=150.0).contains(v));
        match v {
            Some(v) => {
                ed(|e| {
                    e.stops_bad = -1;
                    e.stops_problem.clear();
                });
                edit_stops(&a, |stops| {
                    if let Some(s) = stops.get_mut(i as usize) {
                        s["temp"] = json!(v);
                    }
                });
            }
            None => {
                let line = tr(&a, "Enter a temperature from 0 to 150 °C.");
                ed(|e| {
                    e.stops_bad = i;
                    e.stops_problem = line;
                });
                redraw(&a);
            }
        }
    });
    let a = app.clone();
    g.on_add_stage(move || {
        close_picker();
        edit_stops(&a, |stops| {
            if stops.len() >= MAX_STAGES {
                return;
            }
            let last = stops
                .iter()
                .max_by(|x, y| x["temp"].as_f64().unwrap_or(0.0).total_cmp(&y["temp"].as_f64().unwrap_or(0.0)))
                .cloned()
                .unwrap_or_else(|| json!({ "temp": 50.0, "color": [255, 0, 0] }));
            let temp = (last["temp"].as_f64().unwrap_or(50.0) + 10.0).min(150.0);
            stops.push(json!({ "temp": temp, "color": last["color"] }));
        });
    });
    let a = app.clone();
    g.on_remove_stage(move |i| {
        close_picker();
        edit_stops(&a, |stops| {
            if stops.len() > 1 && (i as usize) < stops.len() {
                stops.remove(i as usize);
            }
        });
    });

    let a = app.clone();
    g.on_set_show_pad(move |on| {
        ed(|e| {
            e.show_pad = on;
            if !on && is_pad_key(e.cursor) {
                e.cursor = 0x41;
            }
        });
        redraw(&a);
    });
    let a = app.clone();
    g.on_key_toggle(move |vk| toggle_key(&a, vk as u32));
    let a = app.clone();
    g.on_board_key(move |text, repeat| {
        if repeat {
            return true;
        }
        let Some(vk) = vk_from_text(&text) else {
            return false;
        };
        if !with_view(|v| layout(v).iter().any(|k| k.0 == vk)) {
            return false;
        }
        toggle_key(&a, vk);
        true
    });
    let a = app.clone();
    g.on_clear_keys(move || edit_layer(&a, |l| l["effect"]["keys"] = json!([])));
    let a = app.clone();
    g.on_board_hit(move |x, row, erase, start| board_hit(&a, x, row, erase, start));
    let a = app.clone();
    g.on_paint_key(move |text| paint_key(&a, &text));
    let a = app.clone();
    g.on_paint_tool(move |erase| {
        ed(|e| e.erase = erase);
        redraw(&a);
    });
    let a = app.clone();
    g.on_paint_recent_pick(move |hex| {
        if let Some(rgb) = from_hex(&hex) {
            ed(|e| {
                e.brush = rgb;
                e.erase = false;
            });
            redraw(&a);
        }
    });
    let a = app.clone();
    g.on_paint_group(move |i| {
        let vks: Vec<u32> = match KEY_GROUPS.get(i as usize) {
            Some(Some(group)) => group.to_vec(),
            _ => with_view(|v| layout(v).iter().map(|k| k.0).collect()),
        };
        paint(&a, &vks, false);
        remember_brush();
        redraw(&a);
    });
    let a = app.clone();
    g.on_fill_empty(move || {
        let vks: Vec<u32> = with_view(|v| layout(v).iter().map(|k| k.0).collect());
        let empty: Vec<u32> = ed(|e| vks.into_iter().filter(|vk| !e.painting.contains_key(vk)).collect());
        let brush = ed(|e| e.brush);
        set_keys(&a, &empty, Some(brush));
    });
    let a = app.clone();
    g.on_clear_paint(move || {
        let vks: Vec<u32> = with_view(|v| layout(v).iter().map(|k| k.0).collect());
        set_keys(&a, &vks, None);
    });
    let a = app.clone();
    g.on_use_map(move |i| {
        let m = with_view(|v| v.snap["key_maps"].get(i as usize).cloned());
        if let Some(m) = m {
            use_map(&a, &m);
            let line = tr(&a, &format!("Using {}.", s(&m["name"])));
            say(&a, line);
        }
    });
    let a = app.clone();
    g.on_map_named(move |t| {
        ed(|e| e.map_name = t.to_string());
        redraw(&a);
    });
    let a = app.clone();
    g.on_save_map(move |name| {
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        let m = current_map(&name);
        match serde_json::from_value(m.clone()) {
            Ok(km) => match key_maps::save_key_map(a.clone(), shared(&a), km) {
                Ok(()) => {
                    ed(|e| e.paint_name = name.clone());
                    let line = tr(&a, &format!("Saved {name}."));
                    say(&a, line);
                }
                Err(err) => say(&a, sentence(&err)),
            },
            Err(err) => say(&a, sentence(&err.to_string())),
        }
    });
    let a = app.clone();
    g.on_delete_map(move |name| {
        let name = ed(|e| if name.trim().is_empty() { e.paint_name.clone() } else { name.trim().to_string() });
        let known = with_view(|v| saved_names(v).contains(&name));
        if !known {
            let line = tr(&a, "Pick a saved key map first.");
            return say(&a, line);
        }
        match key_maps::delete_key_map(a.clone(), shared(&a), name.clone()) {
            Ok(()) => {
                let line = tr(&a, &format!("Deleted {name}."));
                say(&a, line);
            }
            Err(err) => say(&a, sentence(&err)),
        }
    });
    let a = app.clone();
    g.on_copy_code(move |name| {
        let m = current_map(name.trim());
        match serde_json::from_value(m) {
            Ok(km) => {
                let code = key_maps::key_map_code(km);
                ed(|e| {
                    e.code = code;
                    e.copy_tick += 1;
                });
                let line = tr(&a, "Share code copied.");
                say(&a, line);
            }
            Err(err) => say(&a, sentence(&err.to_string())),
        }
    });
    let a = app.clone();
    g.on_add_code(move |code| {
        let result = key_maps::key_map_decode(code.trim().to_string()).and_then(|km| {
            let m = serde_json::to_value(&km).map_err(|e| e.to_string())?;
            key_maps::save_key_map(a.clone(), shared(&a), km)?;
            Ok(m)
        });
        match result {
            Ok(m) => {
                ed(|e| {
                    e.code.clear();
                    e.code_reset += 1;
                });
                use_map(&a, &m);
                let n = m["map"].as_array().map_or(0, Vec::len);
                let line = tr(&a, &format!("Added {}, {}.", s(&m["name"]), plural(n, "key")));
                say(&a, line);
            }
            Err(err) => say(&a, sentence(&err)),
        }
    });

    let a = app.clone();
    g.on_undo(move || step(&a, -1));
    let a = app.clone();
    g.on_redo(move || step(&a, 1));
}

pub fn event(w: &AppWindow, app: &AppHandle, view: &mut View, name: &str) {
    if name == "apps" || name == "fans" {
        apply(w, app, view);
    }
}

pub fn apply(w: &AppWindow, app: &AppHandle, view: &mut View) {
    let g = w.global::<EffectsPage>();
    ed(|e| {
        if let Some(kind) = view.snap["effect"]["kind"].as_str() {
            e.last_used.insert(kind.to_string(), view.snap["effect"].clone());
        }
        track(e, &view.snap);
        let devices = view.snap["devices"].as_array().cloned().unwrap_or_default();
        if !e.device.is_empty() && !devices.iter().any(|d| d["id"] == e.device.as_str()) {
            e.device.clear();
        }
        let len = layers_len(view);
        if e.sel >= len {
            e.sel = len - 1;
        }
        render_layers(&g, app, view, e);
        g.set_layer_sel(e.sel);
        g.set_bad(e.bad.clone().into());
        if e.sel < 0 {
            render_base(&g, app, view, e);
        } else {
            render_layer(&g, app, view, e);
        }
        render_picker(&g, app, view, e);
        fill(g.get_recent(), recent_tiles(e, e.rgb), |m| g.set_recent(m));
        g.set_can_undo(e.hist.at > 0 && (e.hist.at as usize) < e.hist.steps.len());
        g.set_can_redo(((e.hist.at + 1) as usize) < e.hist.steps.len());
        g.set_focus_tick(e.focus_tick);
    });
}

// ---- Reading the snapshot

fn layers_len(v: &View) -> i32 {
    v.snap["layers"].as_array().map_or(0, Vec::len) as i32
}

/// The background being edited: the main one, or the chosen device's own.
fn edited(v: &View) -> Value {
    let device = ed(|e| e.device.clone());
    match v.snap["overrides"].get(&device) {
        Some(o) if !device.is_empty() && !o.is_null() => o.clone(),
        _ => v.effect.clone(),
    }
}

fn edited_in(v: &View, e: &Ed) -> Value {
    match v.snap["overrides"].get(&e.device) {
        Some(o) if !e.device.is_empty() && !o.is_null() => o.clone(),
        _ => v.effect.clone(),
    }
}

fn find_entry(v: &View, post: bool, kind: &str) -> Option<Value> {
    let list = if post { "post_catalog" } else { "catalog" };
    v.snap[list].as_array()?.iter().find(|c| c["default"]["kind"] == kind).cloned()
}

/// A layer effect's catalogue entry, and whether it is a key or overlay effect.
fn layer_info(v: &View, effect: &Value) -> (Option<Value>, bool) {
    let kind = effect["kind"].as_str().unwrap_or_default();
    match find_entry(v, true, kind) {
        Some(post) => (Some(post), true),
        None => (find_entry(v, false, kind), false),
    }
}

/// The catalogue entry behind "effect" (the background being edited) or "layer" (the selected layer).
fn entry(v: &View, target: &str) -> Option<Value> {
    if target == "layer" {
        let sel = ed(|e| e.sel);
        return layer_info(v, &v.snap["layers"][sel.max(0) as usize]["effect"]).0;
    }
    let effect = edited(v);
    find_entry(v, false, effect["kind"].as_str()?)
}

fn layer_kinds(v: &View) -> (Vec<GroupOption>, Vec<Option<(bool, String)>>) {
    let mut options = Vec::new();
    let mut kinds = Vec::new();
    for (title, list, post) in [("Key and overlay effects", "post_catalog", true), ("Effects", "catalog", false)] {
        options.push((title.to_string(), true));
        kinds.push(None);
        for c in v.snap[list].as_array().into_iter().flatten() {
            let kind = c["default"]["kind"].as_str().unwrap_or_default();
            if kind == "none" {
                continue;
            }
            options.push((c["label"].as_str().unwrap_or_default().to_string(), false));
            kinds.push(Some((post, kind.to_string())));
        }
    }
    let options = options
        .into_iter()
        .map(|(label, header)| GroupOption { label: label.into(), header })
        .collect();
    (options, kinds)
}

fn layout(v: &View) -> Vec<(u32, f32, i32, f32)> {
    v.snap["key_layout"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|k| {
            (
                k["vk"].as_u64().unwrap_or(0) as u32,
                k["x"].as_f64().unwrap_or(0.0) as f32,
                k["row"].as_i64().unwrap_or(0) as i32,
                k["w"].as_f64().unwrap_or(1.0) as f32,
            )
        })
        .collect()
}

fn is_pad_key(vk: u32) -> bool {
    with_view(|v| layout(v).iter().any(|k| k.0 == vk && on_pad(k.1)))
}

fn saved_names(v: &View) -> Vec<String> {
    v.snap["key_maps"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|m| m["name"].as_str().unwrap_or_default().to_string())
        .collect()
}

fn apps_seen(v: &View) -> (Vec<String>, Option<String>) {
    let apps = v.events.get("apps").cloned().unwrap_or(Value::Null);
    let running = apps["running"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| a.as_str().map(str::to_string))
        .collect();
    (running, apps["front"].as_str().map(str::to_string))
}

fn app_name(name: &str) -> String {
    let n = name.trim().to_lowercase();
    n.strip_suffix(".exe").map(str::to_string).unwrap_or(n)
}

/// Whether a layer tied to apps shows right now.
fn live(v: &View, layer: &Value) -> bool {
    let (running, front) = apps_seen(v);
    let named = |n: &str| layer["apps"].as_array().into_iter().flatten().any(|a| app_name(a.as_str().unwrap_or("")) == n);
    match layer["when"].as_str() {
        Some("running") => running.iter().any(|r| named(r)),
        Some("front") => front.is_some_and(|f| named(&f)),
        _ => true,
    }
}

// ---- Writing

/// Sends the background being edited: the device's own when one is chosen.
fn set_edited(app: &AppHandle, effect: Value) {
    let device = ed(|e| e.device.clone());
    with_view(|v| {
        if device.is_empty() {
            v.effect = effect.clone();
            v.snap["effect"] = effect.clone();
        } else if let Some(o) = v.snap["overrides"].as_object_mut() {
            o.insert(device.clone(), effect.clone());
        } else {
            v.snap["overrides"] = json!({ device.clone(): effect.clone() });
        }
    });
    match serde_json::from_value(effect) {
        Ok(effect) if device.is_empty() => report(commands::set_effect(app.clone(), shared(app), effect)),
        Ok(effect) => report(commands::set_override(app.clone(), shared(app), device, Some(effect))),
        Err(e) => report(Err(e.to_string())),
    }
    redraw(app);
}

pub fn send_effect(app: &AppHandle, effect: Value) {
    match serde_json::from_value(effect) {
        Ok(effect) => report(commands::set_effect(app.clone(), shared(app), effect)),
        Err(e) => report(Err(e.to_string())),
    }
}

fn edit_layers(app: &AppHandle, change: impl FnOnce(&mut Vec<Value>)) {
    let layers = with_view(|v| {
        let mut layers = v.snap["layers"].as_array().cloned().unwrap_or_default();
        change(&mut layers);
        v.snap["layers"] = json!(layers);
        v.snap["layers"].clone()
    });
    match serde_json::from_value(layers) {
        Ok(layers) => report(commands::set_layers(app.clone(), shared(app), layers)),
        Err(e) => report(Err(e.to_string())),
    }
    redraw(app);
}

fn edit_layer(app: &AppHandle, change: impl FnOnce(&mut Value)) {
    let sel = ed(|e| e.sel);
    if sel < 0 {
        return;
    }
    edit_layers(app, |l| {
        if let Some(layer) = l.get_mut(sel as usize) {
            change(layer);
        }
    });
}

fn set_param(app: &AppHandle, target: &str, name: &str, value: Value) {
    if target == "layer" {
        edit_layer(app, |l| l["effect"][name] = value);
    } else {
        let mut effect = with_view(|v| edited(v));
        effect[name] = value;
        set_edited(app, effect);
    }
}

fn choose_effect(app: &AppHandle, kind: &str) {
    let effect = ed(|e| e.last_used.get(kind).cloned())
        .or_else(|| with_view(|v| find_entry(v, false, kind).map(|e| e["default"].clone())));
    if let Some(effect) = effect {
        set_edited(app, effect);
    }
}

fn search_key(app: &AppHandle, key: &str) -> String {
    let (rows, found) = with_view(|v| {
        let q = ed(|e| e.search.trim().to_lowercase());
        let rows = ed(|e| e.shown.clone());
        let found: Vec<String> = rows
            .iter()
            .filter(|k| {
                find_entry(v, false, k).is_some_and(|c| tr(app, c["label"].as_str().unwrap_or("")).to_lowercase().contains(&q))
            })
            .cloned()
            .collect();
        (rows, found)
    });
    let kind = with_view(|v| edited(v)["kind"].as_str().unwrap_or_default().to_string());
    let used = (key == "down").then(|| found.iter().find(|k| **k == kind).cloned()).flatten();
    let Some(row) = used.or_else(|| found.first().cloned()).or_else(|| rows.first().cloned()) else {
        return String::new();
    };
    choose_effect(app, &row);
    row
}

fn select_layer(app: &AppHandle, i: i32) {
    let len = with_view(|v| layers_len(v));
    if i < -1 || i >= len {
        return;
    }
    focus_layer(app, i);
}

/// Selects a layer, or the background with -1, and moves the focus to its row.
fn focus_layer(app: &AppHandle, i: i32) {
    let len = with_view(|v| layers_len(v));
    ed(|e| {
        if e.sel != i {
            e.picker.clear();
            e.bad.clear();
            e.problem = Default::default();
        }
        e.sel = i;
        e.focus_tick += 1;
    });
    with_win(|w| w.global::<EffectsPage>().set_focus_row(if i < 0 { -1 } else { len - 1 - i }));
    redraw(app);
}

fn move_layer(app: &AppHandle, from: i32, to: i32) {
    let n = with_view(|v| layers_len(v));
    let to = to.clamp(0, n - 1);
    if from == to || from < 0 || from >= n {
        return;
    }
    edit_layers(app, |l| {
        let layer = l.remove(from as usize);
        l.insert(to as usize, layer);
    });
    focus_layer(app, to);
}

fn add_layer(app: &AppHandle, i: i32) {
    let Some(Some((post, kind))) = ed(|e| e.add_kinds.get(i as usize).cloned()) else {
        return;
    };
    let Some(effect) = with_view(|v| find_entry(v, post, &kind).map(|e| e["default"].clone())) else {
        return;
    };
    let keyboards = kind.starts_with("press_") || kind == "key_lights" || kind == "key_colours";
    let layer = json!({
        "enabled": true, "opacity": 1.0, "blend": "normal",
        "scope": if keyboards { "keyboards" } else { "all" },
        "devices": [], "when": "always", "apps": [], "effect": effect,
    });
    edit_layers(app, |l| l.push(layer));
    let len = with_view(|v| layers_len(v));
    focus_layer(app, len - 1);
}

// ---- Exact numbers and the problem line

fn clear_bad_of(id: &str) {
    ed(|e| {
        if e.bad == id {
            e.bad.clear();
            e.problem = Default::default();
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn check_number(app: &AppHandle, target: &str, id: &str, what: &str, text: &str, min: f64, max: f64, digits: usize) -> Option<f64> {
    let value = text.trim().parse::<f64>().ok().map(|v| round_to(v, digits));
    match value.filter(|v| *v >= min && *v <= max) {
        Some(v) => {
            clear_bad_of(id);
            Some(v)
        }
        None => {
            let line = tr(app, &format!("{what} must be from {} to {}.", num(min), num(max)));
            ed(|e| {
                e.bad = id.to_string();
                e.problem = (target.to_string(), line);
            });
            None
        }
    }
}

fn range_text(app: &AppHandle, target: &str, name: &str, text: &str) {
    let Some(p) = with_view(|v| {
        entry(v, target)?["params"].as_array()?.iter().find(|p| p["name"] == name).cloned()
    }) else {
        return;
    };
    let c = &p["control"];
    let (min, max, step) = (
        c["min"].as_f64().unwrap_or(0.0),
        c["max"].as_f64().unwrap_or(1.0),
        c["step"].as_f64().unwrap_or(0.01),
    );
    let what = tr(app, p["label"].as_str().unwrap_or(name));
    if let Some(v) = check_number(app, target, &format!("{target}:{name}"), &what, text, min, max, digits(step)) {
        set_param(app, target, name, json!(v));
    }
    redraw(app);
}

fn round_to(v: f64, digits: usize) -> f64 {
    let f = 10f64.powi(digits as i32);
    (v * f).round() / f
}

fn digits(step: f64) -> usize {
    let text = format!("{step}");
    text.split('.').nth(1).map_or(0, str::len)
}

fn num(v: f64) -> String {
    let t = format!("{v}");
    t.strip_suffix(".0").map(str::to_string).unwrap_or(t)
}

// ---- Rendering

fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

fn sentence(t: &str) -> String {
    let mut c = t.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

fn words(t: &str) -> String {
    sentence(t).replace('_', " ")
}

fn scope_words(app: &AppHandle, v: &View, layer: &Value) -> String {
    match layer["scope"].as_str() {
        Some("chosen") => {
            let chosen = layer["devices"].as_array().cloned().unwrap_or_default();
            let devices = v.snap["devices"].as_array().cloned().unwrap_or_default();
            let there: Vec<&Value> = devices.iter().filter(|d| chosen.contains(&d["id"])).collect();
            if there.len() == 1 {
                there[0]["name"].as_str().unwrap_or_default().to_string()
            } else {
                tr(app, &plural(there.len(), "device"))
            }
        }
        Some("keyboards") => tr(app, "Keyboards"),
        _ => tr(app, "All devices"),
    }
}

fn when_words(app: &AppHandle, layer: &Value) -> String {
    let apps: Vec<&str> = layer["apps"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    match layer["when"].as_str() {
        Some("always") | None => String::new(),
        _ if apps.is_empty() => tr(app, "no app yet"),
        Some(when) => {
            let more = if apps.len() > 1 { format!(" +{}", apps.len() - 1) } else { String::new() };
            let names = format!("{}{more}", apps[0]);
            if when == "front" {
                tr(app, &format!("{names} in front"))
            } else {
                tr(app, &format!("while {names} is open"))
            }
        }
    }
}

fn render_layers(g: &EffectsPage, app: &AppHandle, v: &View, e: &mut Ed) {
    let layers = v.snap["layers"].as_array().cloned().unwrap_or_default();
    let rows: Vec<LayerRow> = layers
        .iter()
        .rev()
        .map(|l| {
            let (info, _) = layer_info(v, &l["effect"]);
            let name = info
                .map(|i| tr(app, i["label"].as_str().unwrap_or("")))
                .unwrap_or_else(|| words(l["effect"]["kind"].as_str().unwrap_or("")));
            let opacity = l["opacity"].as_f64().unwrap_or(1.0);
            let blend = l["blend"].as_str().unwrap_or("normal");
            let parts = [
                scope_words(app, v, l),
                when_words(app, l),
                if opacity < 1.0 { format!("{}%", (opacity * 100.0).round()) } else { String::new() },
                if blend != "normal" && blend != "over" { tr(app, &words(blend)) } else { String::new() },
            ];
            let enabled = l["enabled"].as_bool().unwrap_or(true);
            LayerRow {
                name: name.into(),
                tag: parts.into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" · ").into(),
                enabled,
                waiting: enabled && !live(v, l),
            }
        })
        .collect();
    fill(g.get_layers(), rows, |m| g.set_layers(m));
    let own = v.snap["overrides"].as_object().map_or(0, |o| o.values().filter(|x| !x.is_null()).count());
    let base = find_entry(v, false, v.snap["effect"]["kind"].as_str().unwrap_or(""))
        .map(|c| tr(app, c["label"].as_str().unwrap_or("")))
        .unwrap_or_else(|| words(v.snap["effect"]["kind"].as_str().unwrap_or("")));
    let tag = if own > 0 { format!("{base} · {}", tr(app, &format!("{own} own"))) } else { base };
    g.set_base_tag(tag.into());

    let (options, kinds) = layer_kinds(v);
    let mut add = vec![GroupOption { label: tr(app, "Add a layer").into(), header: false }];
    add.extend(options.into_iter().map(|o| GroupOption { label: tr(app, &o.label).into(), header: o.header }));
    let mut add_kinds = vec![None];
    add_kinds.extend(kinds);
    e.add_kinds = add_kinds;
    fill(g.get_add_options(), add, |m| g.set_add_options(m));
}

fn render_base(g: &EffectsPage, app: &AppHandle, v: &View, e: &mut Ed) {
    let effect = edited_in(v, e);
    let kind = effect["kind"].as_str().unwrap_or_default().to_string();
    let overrides = &v.snap["overrides"];
    let has_own = |id: &str| overrides.get(id).is_some_and(|o| !o.is_null());
    let devices: Vec<Value> = v.snap["devices"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|d| d["enabled"].as_bool().unwrap_or(true))
        .cloned()
        .collect();
    let mut options = vec![GroupOption { label: tr(app, "All devices").into(), header: false }];
    for d in &devices {
        let name = d["name"].as_str().unwrap_or_default();
        let label = if has_own(d["id"].as_str().unwrap_or("")) { tr(app, &format!("{name} (own)")) } else { name.to_string() };
        options.push(GroupOption { label: label.into(), header: false });
    }
    e.device_ids = devices.iter().map(|d| d["id"].as_str().unwrap_or_default().to_string()).collect();
    fill(g.get_edit_options(), options, |m| g.set_edit_options(m));
    let index = e.device_ids.iter().position(|id| *id == e.device).map_or(0, |i| i + 1);
    g.set_edit_index(index as i32);
    let own = !e.device.is_empty() && has_own(&e.device);
    g.set_follow(!e.device.is_empty() && !own);
    g.set_own(own);

    let q = e.search.trim().to_lowercase();
    let mut shown = Vec::new();
    let mut matches = 0;
    let picks: Vec<EffectPick> = v.snap["catalog"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let k = c["default"]["kind"].as_str().unwrap_or_default();
            let label = tr(app, c["label"].as_str().unwrap_or(""));
            let hit = label.to_lowercase().contains(&q);
            if hit {
                matches += 1;
            }
            if !hit && k != kind {
                return None;
            }
            shown.push(k.to_string());
            Some(EffectPick { kind: k.into(), label: label.into(), tag: !hit })
        })
        .collect();
    g.set_effect_at(shown.iter().position(|k| *k == kind).map_or(-1, |i| i as i32));
    e.shown = shown;
    fill(g.get_effects(), picks, |m| g.set_effects(m));
    g.set_no_match(matches == 0);
    g.set_effect_kind(kind.clone().into());
    let label = find_entry(v, false, &kind).map(|c| tr(app, c["label"].as_str().unwrap_or(""))).unwrap_or_default();
    g.set_effect_label(label.into());
    if g.get_search().as_str() != e.search {
        g.set_search(e.search.clone().into());
    }

    let info = find_entry(v, false, &kind);
    let rows = param_rows(app, v, e, info.as_ref(), &effect, "effect");
    let old = g.get_params();
    fill(old.clone(), keep_options(&old, rows), |m| g.set_params(m));
    g.set_params_problem(if e.problem.0 == "effect" { e.problem.1.clone().into() } else { SharedString::new() });
}

fn render_layer(g: &EffectsPage, app: &AppHandle, v: &View, e: &mut Ed) {
    let layer = v.snap["layers"][e.sel as usize].clone();
    let (info, _) = layer_info(v, &layer["effect"]);
    let kind = layer["effect"]["kind"].as_str().unwrap_or_default().to_string();
    e.last_layer.insert(kind.clone(), layer["effect"].clone());
    let title = info.as_ref().map_or_else(|| tr(app, "Layer"), |i| tr(app, i["label"].as_str().unwrap_or("")));
    g.set_layer_title(title.into());

    let (options, kinds) = layer_kinds(v);
    let options: Vec<GroupOption> =
        options.into_iter().map(|o| GroupOption { label: tr(app, &o.label).into(), header: o.header }).collect();
    let post = find_entry(v, true, &kind).is_some();
    let index = kinds.iter().position(|k| k.as_ref().is_some_and(|(p, k)| *p == post && *k == kind));
    fill(g.get_kind_options(), options, |m| g.set_kind_options(m));
    g.set_kind_index(index.unwrap_or(0) as i32);
    let blend = layer["blend"].as_str().unwrap_or("normal");
    g.set_blend_index(BLENDS.iter().position(|b| *b == blend).unwrap_or(0) as i32);
    let opacity = (layer["opacity"].as_f64().unwrap_or(1.0) * 100.0).round();
    g.set_opacity(opacity as f32);
    g.set_opacity_text(num(opacity).into());
    let scope = layer["scope"].as_str().unwrap_or("all");
    g.set_scope_index(SCOPES.iter().position(|x| *x == scope).unwrap_or(0) as i32);
    let chosen = layer["devices"].as_array().cloned().unwrap_or_default();
    let checks: Vec<CheckRow> = v.snap["devices"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|d| CheckRow { id: s(&d["id"]), name: s(&d["name"]), on: chosen.contains(&d["id"]) })
        .collect();
    fill(g.get_checks(), checks, |m| g.set_checks(m));
    let when = layer["when"].as_str().unwrap_or("always");
    g.set_when_index(WHENS.iter().position(|x| *x == when).unwrap_or(0) as i32);
    let apps: Vec<SharedString> = layer["apps"].as_array().into_iter().flatten().map(s).collect();
    let typed = e.app_typed.trim().to_lowercase();
    let choices: Vec<SharedString> = e
        .app_list
        .iter()
        .filter(|n| n.to_lowercase().contains(&typed) && !apps.iter().any(|a| app_name(a) == app_name(n)))
        .take(8)
        .map(|n| n.as_str().into())
        .collect();
    fill(g.get_app_choices(), choices, |m| g.set_app_choices(m));
    let hint = if !apps.is_empty() {
        tr(app, "Pick an app with a window open, or type its .exe name.")
    } else {
        let until = if when == "front" { "in front." } else { "open." };
        tr(app, "Add an app. The layer stays hidden until one is {0}").replace("{0}", &tr(app, until))
    };
    g.set_app_hint(hint.into());
    fill(g.get_apps(), apps, |m| g.set_apps(m));

    let mut rows = param_rows(app, v, e, info.as_ref(), &layer["effect"], "layer");
    // The second colour only matters when the colours fade to it.
    let colors = layer["effect"]["colors"].as_str();
    rows.retain(|p| match p.name.as_str() {
        "second" => colors == Some("fade_to_second"),
        "color" => colors.is_none_or(|c| c == "single" || c == "fade_to_second"),
        _ => true,
    });
    let old = g.get_layer_params();
    fill(old.clone(), keep_options(&old, rows), |m| g.set_layer_params(m));
    g.set_layer_problem(if e.problem.0 == "layer" { e.problem.1.clone().into() } else { SharedString::new() });
    render_keys(g, app, v, e, &layer);
}

/// Reuses the option lists of rows that did not change, so that open lists stay open.
fn keep_options(old: &ModelRc<EParam>, mut rows: Vec<EParam>) -> Vec<EParam> {
    for (i, row) in rows.iter_mut().enumerate() {
        if let Some(prev) = old.row_data(i)
            && prev.options.row_count() == row.options.row_count()
            && prev.options.iter().zip(row.options.iter()).all(|(a, b)| a == b)
        {
            row.options = prev.options.clone();
        }
    }
    rows
}

fn param_rows(app: &AppHandle, v: &View, e: &mut Ed, info: Option<&Value>, effect: &Value, target: &str) -> Vec<EParam> {
    let mut rows = Vec::new();
    for p in info.and_then(|i| i["params"].as_array()).into_iter().flatten() {
        let name = p["name"].as_str().unwrap_or_default();
        let value = &effect[name];
        let c = &p["control"];
        let mut row = EParam {
            name: name.into(),
            label: tr(app, p["label"].as_str().unwrap_or("")).into(),
            control: s(&c["type"]),
            ..Default::default()
        };
        match c["type"].as_str().unwrap_or_default() {
            "range" => {
                let step = c["step"].as_f64().unwrap_or(0.01);
                row.min = c["min"].as_f64().unwrap_or(0.0) as f32;
                row.max = c["max"].as_f64().unwrap_or(1.0) as f32;
                row.step = step as f32;
                let v = value.as_f64().unwrap_or(0.0);
                row.value = v as f32;
                row.text = format!("{:.*}", digits(step), v).into();
            }
            "color" => {
                let rgb = rgb_of(value);
                row.color = slint_color(rgb);
                row.hex = to_hex(rgb).into();
            }
            "toggle" => row.on = value.as_bool().unwrap_or(false),
            "choice" => {
                let options: Vec<GroupOption> = c["options"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|o| GroupOption { label: tr(app, &words(o.as_str().unwrap_or(""))).into(), header: false })
                    .collect();
                row.index = c["options"]
                    .as_array()
                    .and_then(|o| o.iter().position(|o| o == value))
                    .unwrap_or(0) as i32;
                row.options = ModelRc::new(VecModel::from(options));
            }
            "sensor" => {
                let chosen = value.as_str().unwrap_or_default().to_string();
                let fans = v.events.get("fans").cloned().unwrap_or(Value::Null);
                let service = fans["service"].as_str().unwrap_or_default();
                let sensors: Vec<Value> = fans["sensors"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|s| s["kind"] != "power")
                    .cloned()
                    .collect();
                let mut options: Vec<(String, String)> =
                    sensors.iter().map(|s| (s["id"].as_str().unwrap_or("").into(), s["name"].as_str().unwrap_or("").into())).collect();
                if !options.iter().any(|(id, _)| *id == chosen) {
                    let name = if chosen.is_empty() { tr(app, "None") } else { chosen.clone() };
                    options.push((chosen.clone(), name));
                }
                row.index = options.iter().position(|(id, _)| *id == chosen).unwrap_or(0) as i32;
                e.sensor_ids = options.iter().map(|(id, _)| id.clone()).collect();
                row.options = ModelRc::new(VecModel::from(
                    options.into_iter().map(|(_, n)| GroupOption { label: n.into(), header: false }).collect::<Vec<_>>(),
                ));
                let sensor = sensors.iter().find(|s| s["id"] == chosen.as_str());
                let status = if !v.snap["settings"]["fans_module"].as_bool().unwrap_or(true) {
                    tr(app, "Turn on the Fans module to read temperatures.")
                } else if !service.is_empty() && service != "running" {
                    tr(app, "Start the service in the Fans tab to read temperatures.")
                } else if !service.is_empty() && sensor.is_none_or(|s| s["value"].is_null()) {
                    tr(app, "No reading from this sensor. The coldest stage is shown.")
                } else if let Some(s) = sensor {
                    tr(app, &format!("Now {:.1} °C", s["value"].as_f64().unwrap_or(0.0)))
                } else {
                    String::new()
                };
                row.status = status.into();
            }
            "stops" => {
                e.stops_at = Some((target.to_string(), name.to_string()));
                let stops: Vec<StopRow> = value
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|st| {
                        let rgb = rgb_of(&st["color"]);
                        StopRow { temp: num(st["temp"].as_f64().unwrap_or(0.0)).into(), color: slint_color(rgb), hex: to_hex(rgb).into() }
                    })
                    .collect();
                with_win(|w| {
                    let g = w.global::<EffectsPage>();
                    fill(g.get_stops(), stops, |m| g.set_stops(m));
                    g.set_stops_problem(e.stops_problem.clone().into());
                    g.set_stops_bad(e.stops_bad);
                });
            }
            _ => {}
        }
        rows.push(row);
    }
    rows
}

fn render_keys(g: &EffectsPage, app: &AppHandle, v: &View, e: &mut Ed, layer: &Value) {
    let effect = &layer["effect"];
    let keys_mode = effect.get("keys").is_some();
    let paint_mode = effect.get("map").is_some() && effect["kind"] == "key_colours";
    if !keys_mode && !paint_mode {
        return;
    }
    let lay = layout(v);
    let pad_of = |vk: u32| lay.iter().any(|k| k.0 == vk && on_pad(k.1));
    let mut caps = Vec::new();
    if keys_mode {
        let on: Vec<u32> = effect["keys"].as_array().into_iter().flatten().filter_map(|k| k.as_u64()).map(|k| k as u32).collect();
        if on.iter().any(|vk| pad_of(*vk)) {
            e.show_pad = true;
        }
        let rgb = rgb_of(&effect["color"]);
        for k in &lay {
            let pressed = on.contains(&k.0);
            let title = format!("{}{}", if on_pad(k.1) { tr(app, "Number pad") + " " } else { String::new() }, key_name(k.0));
            caps.push(cap(k, title, pressed, rgb, false));
        }
        let count = if on.is_empty() {
            tr(app, "No keys chosen")
        } else {
            format!("{}: {}", tr(app, &plural(on.len(), "key")), on.iter().map(|vk| key_name(*vk)).collect::<Vec<_>>().join(" "))
        };
        g.set_key_count(count.into());
    } else {
        let echo = e.stroke.is_none() && e.sent_at.is_none_or(|t| t.elapsed() > Duration::from_millis(400));
        if e.paint_layer != e.sel || echo {
            e.paint_layer = e.sel;
            e.painting = effect["map"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|p| Some((p[0].as_u64()? as u32, rgb_of(&p[1]))))
                .collect();
        }
        if e.painting.keys().any(|vk| pad_of(*vk)) {
            e.show_pad = true;
        }
        for k in &lay {
            let c = e.painting.get(&k.0).copied();
            let title = format!("{}, {}", key_name(k.0), c.map_or_else(|| tr(app, "not painted"), to_hex));
            caps.push(cap(k, title, c.is_some(), c.unwrap_or_default(), k.0 == e.cursor));
        }
        let colours: std::collections::BTreeSet<[u8; 3]> = e.painting.values().copied().collect();
        let count = if e.painting.is_empty() {
            tr(app, "No keys painted")
        } else {
            tr(app, &format!("{} in {}", plural(e.painting.len(), "key"), plural(colours.len(), "colour")))
        };
        g.set_key_count(count.into());
        g.set_brush(slint_color(e.brush));
        g.set_brush_hex(to_hex(e.brush).into());
        g.set_erasing(e.erase);
        fill(g.get_paint_recent(), recent_tiles(e, e.brush), |m| g.set_paint_recent(m));
        let names = saved_names(v);
        let first = if names.is_empty() { "No saved key maps" } else { "Use a saved key map" };
        let mut choices = vec![GroupOption { label: tr(app, first).into(), header: false }];
        choices.extend(names.iter().map(|n| GroupOption { label: n.as_str().into(), header: false }));
        fill(g.get_map_choices(), choices, |m| g.set_map_choices(m));
        let name = if e.map_name.trim().is_empty() { e.paint_name.clone() } else { e.map_name.trim().to_string() };
        g.set_can_delete_map(names.contains(&name));
        g.set_map_name(e.paint_name.clone().into());
        g.set_share_code(e.code.clone().into());
        g.set_copy_tick(e.copy_tick);
        g.set_code_reset(e.code_reset);
        g.set_paint_note(e.note.clone().into());
        let zoned: Vec<String> = v.snap["devices"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|d| d["kind"] == "keyboard" && d["grid"][1].as_u64().is_some_and(|h| h <= 1))
            .map(|d| d["name"].as_str().unwrap_or_default().to_string())
            .collect();
        let zones = if zoned.is_empty() {
            String::new()
        } else {
            let lights = if zoned.len() == 1 { " lights" } else { " light" };
            tr(app, &format!("{}{lights} in zones, not single keys: each zone shows the colour most of its painted keys have.", zoned.join(", ")))
        };
        g.set_zones(zones.into());
    }
    fill(g.get_keys(), caps, |m| g.set_keys(m));
    g.set_show_pad(e.show_pad);
}

fn cap(k: &(u32, f32, i32, f32), title: String, on: bool, rgb: [u8; 3], cursor: bool) -> KeyCap {
    KeyCap {
        vk: k.0 as i32,
        x: k.1,
        w: k.3,
        row: k.2,
        label: key_short(k.0).into(),
        title: title.into(),
        pad: on_pad(k.1),
        on,
        fill: slint_color(rgb),
        ink: if contrast(rgb) { slint::Color::from_rgb_u8(0, 0, 0) } else { slint::Color::from_rgb_u8(255, 255, 255) },
        cursor,
    }
}

fn contrast([r, g, b]: [u8; 3]) -> bool {
    0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32 > 140.0
}

// ---- Colours

fn rgb_of(v: &Value) -> [u8; 3] {
    serde_json::from_value(v.clone()).unwrap_or_default()
}

fn slint_color([r, g, b]: [u8; 3]) -> slint::Color {
    slint::Color::from_rgb_u8(r, g, b)
}

fn to_hex([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02X}{g:02X}{b:02X}")
}

/// Three or six hex digits as a colour.
fn from_hex(text: &str) -> Option<[u8; 3]> {
    let t = text.trim().trim_start_matches('#');
    if !t.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let six: String = match t.len() {
        3 => t.chars().flat_map(|c| [c, c]).collect(),
        6 => t.to_string(),
        _ => return None,
    };
    let v = u32::from_str_radix(&six, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

pub fn parse_hex(text: &str) -> Option<[u8; 3]> {
    from_hex(text).filter(|_| text.trim().trim_start_matches('#').len() == 6)
}

fn to_hsv([r, g, b]: [u8; 3]) -> [f32; 3] {
    let (r, g, b) = (r as f32, g as f32, b as f32);
    let max = r.max(g).max(b);
    let span = max - r.min(g).min(b);
    let mut h = 0.0;
    if span > 0.0 {
        h = if max == r {
            (g - b) / span
        } else if max == g {
            (b - r) / span + 2.0
        } else {
            (r - g) / span + 4.0
        };
    }
    [(h * 60.0 + 360.0) % 360.0, if max > 0.0 { span / max * 100.0 } else { 0.0 }, max / 2.55]
}

fn from_hsv([h, s, v]: [f32; 3]) -> [u8; 3] {
    let part = |n: f32| {
        let k = (n + h / 60.0) % 6.0;
        (v * 2.55 * (1.0 - s / 100.0 * k.min(4.0 - k).clamp(0.0, 1.0))).round() as u8
    };
    [part(5.0), part(3.0), part(1.0)]
}

fn recent_tiles(e: &Ed, current: [u8; 3]) -> Vec<Tile> {
    e.recent.iter().map(|c| Tile { name: SharedString::new(), hex: to_hex(*c).into(), color: slint_color(*c), on: *c == current }).collect()
}

fn remember(e: &mut Ed, rgb: [u8; 3]) {
    let hex = to_hex(rgb);
    if PRESETS.iter().any(|(_, p)| *p == hex) {
        return;
    }
    e.recent.retain(|c| *c != rgb);
    e.recent.insert(0, rgb);
    e.recent.truncate(8);
}

fn remember_brush() {
    ed(|e| {
        if !e.erase {
            let b = e.brush;
            remember(e, b);
        }
    });
}

/// The colour a picker edits, read from the view.
fn picker_colour(v: &View, e: &Ed, pid: &str) -> Option<[u8; 3]> {
    if pid == "brush" {
        return Some(e.brush);
    }
    let (target, rest) = pid.split_once(':')?;
    let effect = if target == "layer" {
        v.snap["layers"].get(e.sel.max(0) as usize)?["effect"].clone()
    } else {
        edited_in(v, e)
    };
    if let Some(i) = rest.strip_prefix("stop:") {
        let (_, name) = e.stops_at.as_ref()?;
        let stop = effect[name.as_str()].get(i.parse::<usize>().ok()?)?;
        return Some(rgb_of(&stop["color"]));
    }
    effect.get(rest).map(rgb_of)
}

fn picker_key(v: &View, e: &Ed) -> String {
    let kind = if e.sel < 0 { edited_in(v, e)["kind"].clone() } else { v.snap["layers"][e.sel as usize]["effect"]["kind"].clone() };
    format!("{}|{}|{}", e.sel, e.device, kind)
}

fn open_picker(app: &AppHandle, pid: &str) {
    let open = ed(|e| e.picker == pid);
    close_picker();
    if !open {
        with_view(|v| {
            ed(|e| {
                if let Some(rgb) = picker_colour(v, e, pid) {
                    e.picker = pid.to_string();
                    e.picker_for = picker_key(v, e);
                    e.rgb = rgb;
                    e.before = rgb;
                    e.hsv = to_hsv(rgb);
                }
            })
        });
    }
    redraw(app);
}

fn close_picker() {
    ed(|e| {
        if !e.picker.is_empty() && e.rgb != e.before {
            let rgb = e.rgb;
            remember(e, rgb);
        }
        e.picker.clear();
        e.picker_bad = -1;
        e.picker_problem.clear();
    });
}

/// A colour chosen in the open picker, sent to what it edits.
fn choose(app: &AppHandle, rgb: [u8; 3], keep_hsv: bool) {
    let pid = ed(|e| {
        e.rgb = rgb;
        if !keep_hsv {
            e.hsv = to_hsv(rgb);
        }
        e.picker.clone()
    });
    if pid == "brush" {
        ed(|e| {
            e.brush = rgb;
            e.erase = false;
        });
        return redraw(app);
    }
    let Some((target, rest)) = pid.split_once(':') else {
        return;
    };
    match rest.strip_prefix("stop:").and_then(|i| i.parse::<usize>().ok()) {
        Some(i) => edit_stops(app, |stops| {
            if let Some(s) = stops.get_mut(i) {
                s["color"] = json!(rgb);
            }
        }),
        None => set_param(app, target, rest, json!(rgb)),
    }
}

fn render_picker(g: &EffectsPage, app: &AppHandle, v: &View, e: &mut Ed) {
    if !e.picker.is_empty() {
        let now = picker_colour(v, e, &e.picker.clone());
        match now {
            Some(rgb) if e.picker_for == picker_key(v, e) => {
                if rgb != e.rgb {
                    e.rgb = rgb;
                }
                if from_hsv(e.hsv) != e.rgb {
                    e.hsv = to_hsv(e.rgb);
                }
            }
            _ => {
                if e.rgb != e.before {
                    let rgb = e.rgb;
                    remember(e, rgb);
                }
                e.picker.clear();
            }
        }
    }
    g.set_picker(e.picker.clone().into());
    if e.picker.is_empty() {
        return;
    }
    let [h, s, val] = e.hsv;
    g.set_pick_h(h.round());
    g.set_pick_s(s.round());
    g.set_pick_v(val.round());
    if e.picker_bad != 0 {
        g.set_pick_ht(num(h.round() as f64).into());
    }
    if e.picker_bad != 1 {
        g.set_pick_st(num(s.round() as f64).into());
    }
    if e.picker_bad != 2 {
        g.set_pick_vt(num(val.round() as f64).into());
    }
    g.set_pick_hex(to_hex(e.rgb).into());
    let steps = |count: usize, at: &dyn Fn(f32) -> [f32; 3]| -> Vec<slint::Color> {
        (0..count).map(|i| slint_color(from_hsv(at((i as f32 + 0.5) / count as f32 * 100.0)))).collect()
    };
    fill(g.get_hue_track(), steps(12, &|k| [k * 3.6, 100.0, 100.0]), |m| g.set_hue_track(m));
    fill(g.get_sat_track(), steps(8, &|k| [h, k, val]), |m| g.set_sat_track(m));
    fill(g.get_val_track(), steps(8, &|k| [h, s, k]), |m| g.set_val_track(m));
    let presets: Vec<Tile> = PRESETS
        .iter()
        .map(|(name, hex)| {
            let rgb = from_hex(hex).unwrap_or_default();
            Tile { name: tr(app, name).into(), hex: (*hex).into(), color: slint_color(rgb), on: rgb == e.rgb }
        })
        .collect();
    fill(g.get_presets(), presets, |m| g.set_presets(m));
    g.set_picker_bad(e.picker_bad);
    g.set_picker_problem(e.picker_problem.clone().into());
}

// ---- Stages

fn edit_stops(app: &AppHandle, change: impl FnOnce(&mut Vec<Value>)) {
    let Some((target, name)) = ed(|e| e.stops_at.clone()) else {
        return;
    };
    let mut stops = with_view(|v| {
        let effect = if target == "layer" {
            v.snap["layers"][ed(|e| e.sel).max(0) as usize]["effect"].clone()
        } else {
            edited(v)
        };
        effect[name.as_str()].as_array().cloned().unwrap_or_default()
    });
    change(&mut stops);
    set_param(app, &target, &name, json!(stops));
}

// ---- Keyboards

fn toggle_key(app: &AppHandle, vk: u32) {
    edit_layer(app, |l| {
        let mut keys: Vec<Value> = l["effect"]["keys"].as_array().cloned().unwrap_or_default();
        match keys.iter().position(|k| k.as_u64() == Some(vk as u64)) {
            Some(i) => {
                keys.remove(i);
            }
            None => keys.push(json!(vk)),
        }
        l["effect"]["keys"] = json!(keys);
    });
}

/// Paints `vks` with `colour`, or clears them without one.
fn set_keys(app: &AppHandle, vks: &[u32], colour: Option<[u8; 3]>) {
    let changed = ed(|e| {
        let mut changed = false;
        for vk in vks {
            match colour {
                Some(c) if e.painting.get(vk) != Some(&c) => {
                    e.painting.insert(*vk, c);
                }
                None if e.painting.contains_key(vk) => {
                    e.painting.remove(vk);
                }
                _ => continue,
            }
            changed = true;
        }
        changed
    });
    if !changed {
        return redraw(app);
    }
    let map: Vec<Value> = ed(|e| {
        e.sent_at = Some(Instant::now());
        e.painting.iter().map(|(vk, c)| json!([vk, c])).collect()
    });
    edit_layer(app, |l| l["effect"]["map"] = json!(map));
}

fn paint(app: &AppHandle, vks: &[u32], erase: bool) {
    let colour = ed(|e| if erase || e.erase { None } else { Some(e.brush) });
    set_keys(app, vks, colour);
}

fn board_hit(app: &AppHandle, x: f32, row: i32, erase: bool, start: bool) {
    let paint_mode = with_view(|v| v.snap["layers"][ed(|e| e.sel).max(0) as usize]["effect"]["kind"] == "key_colours");
    if row < 0 {
        if ed(|e| e.stroke.take()) == Some(false) {
            remember_brush();
            redraw(app);
        }
        return;
    }
    let show_pad = ed(|e| e.show_pad);
    let hit = with_view(|v| {
        layout(v)
            .into_iter()
            .find(|k| k.2 == row && (x - k.1).abs() <= k.3 / 2.0 && (show_pad || !on_pad(k.1)))
            .map(|k| k.0)
    });
    let Some(vk) = hit else {
        return;
    };
    if !paint_mode {
        if start {
            toggle_key(app, vk);
        }
        return;
    }
    if start {
        ed(|e| e.stroke = Some(erase || e.erase));
    }
    ed(|e| e.cursor = vk);
    paint(app, &[vk], erase);
}

fn paint_key(app: &AppHandle, text: &str) -> bool {
    let arrow = match text {
        "\u{F702}" => Some((-1, 0)),
        "\u{F703}" => Some((1, 0)),
        "\u{F700}" => Some((0, -1)),
        "\u{F701}" => Some((0, 1)),
        _ => None,
    };
    if let Some((dx, dy)) = arrow {
        nudge(dx, dy);
        redraw(app);
        return true;
    }
    let cursor = ed(|e| e.cursor);
    match text {
        " " | "\n" => {
            paint(app, &[cursor], false);
            remember_brush();
            redraw(app);
            true
        }
        "\u{7f}" | "\u{8}" => {
            paint(app, &[cursor], true);
            true
        }
        _ => false,
    }
}

fn nudge(dx: i32, dy: i32) {
    let lay = with_view(|v| layout(v));
    let (cursor, show_pad) = ed(|e| (e.cursor, e.show_pad));
    let keys: Vec<_> = lay.iter().filter(|k| show_pad || !on_pad(k.1)).collect();
    let Some(at) = lay.iter().find(|k| k.0 == cursor) else {
        return;
    };
    let next = if dy != 0 {
        keys.iter()
            .filter(|k| k.2 == at.2 + dy)
            .min_by(|a, b| (a.1 - at.1).abs().total_cmp(&(b.1 - at.1).abs()))
            .map(|k| k.0)
    } else {
        let mut row: Vec<_> = keys.iter().filter(|k| k.2 == at.2).collect();
        row.sort_by(|a, b| a.1.total_cmp(&b.1));
        row.iter()
            .position(|k| k.0 == cursor)
            .and_then(|i| row.get((i as i32 + dx).max(-1) as usize))
            .map(|k| k.0)
    };
    if let Some(vk) = next {
        ed(|e| e.cursor = vk);
    }
}

fn current_map(name: &str) -> Value {
    ed(|e| {
        let name = if !name.is_empty() {
            name.to_string()
        } else if !e.paint_name.is_empty() {
            e.paint_name.clone()
        } else {
            "Key map".to_string()
        };
        json!({ "name": name, "map": e.painting.iter().map(|(vk, c)| json!([vk, c])).collect::<Vec<_>>() })
    })
}

fn use_map(app: &AppHandle, m: &Value) {
    ed(|e| {
        e.painting = m["map"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| Some((p[0].as_u64()? as u32, rgb_of(&p[1]))))
            .collect();
        e.paint_name = m["name"].as_str().unwrap_or_default().to_string();
        e.map_name = e.paint_name.clone();
    });
    set_keys(app, &[], None);
    let map: Vec<Value> = ed(|e| {
        e.sent_at = Some(Instant::now());
        e.painting.iter().map(|(vk, c)| json!([vk, c])).collect()
    });
    edit_layer(app, |l| l["effect"]["map"] = json!(map));
}

fn say(app: &AppHandle, line: String) {
    ed(|e| e.note = line);
    redraw(app);
}

// ---- Randomise

pub fn random(seed: &mut u64) -> f32 {
    if *seed == 0 {
        *seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |d| d.as_nanos() as u64)
            | 1;
    }
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    (*seed >> 40) as f32 / (1u64 << 24) as f32
}

fn randomised(v: &mut View) -> Option<Value> {
    let entry = find_entry(v, false, edited(v)["kind"].as_str()?)?;
    let mut effect = edited(v);
    for p in entry["params"].as_array()? {
        let name = p["name"].as_str()?;
        let c = &p["control"];
        let r = random(&mut v.seed);
        effect[name] = match c["type"].as_str()? {
            "range" => {
                let (lo, hi, step) = (c["min"].as_f64()?, c["max"].as_f64()?, c["step"].as_f64()?);
                let steps = ((hi - lo) / step).round();
                json!(round_to(lo + (r as f64 * steps).round() * step, 4))
            }
            "color" => json!([0, 0, 0].map(|_: u8| (random(&mut v.seed) * 256.0).min(255.0) as u8)),
            "toggle" => json!(r < 0.5),
            _ => match c["options"].as_array() {
                Some(o) if !o.is_empty() => o[((r * o.len() as f32) as usize).min(o.len() - 1)].clone(),
                _ => continue,
            },
        };
    }
    Some(effect)
}

// ---- Undo history of the effect, layers, device effects, brightness, devices and layout

fn undoable(snap: &Value) -> Value {
    let mut rects = serde_json::Map::new();
    let mut levels = serde_json::Map::new();
    for d in snap["devices"].as_array().into_iter().flatten() {
        let id = d["id"].as_str().unwrap_or_default().to_string();
        rects.insert(id.clone(), d["rect"].clone());
        levels.insert(id, json!([d["enabled"], d["brightness"]]));
    }
    json!({
        "effect": snap["effect"], "layers": snap["layers"], "overrides": snap["overrides"],
        "brightness": snap["brightness"], "rects": rects, "levels": levels,
    })
}

/// What changed between two states: a label for the user, and a key that tells one kind of edit from another.
fn difference(a: &Value, b: &Value) -> Option<(String, String)> {
    let empty = serde_json::Map::new();
    let (ea, eb) = (a["effect"].as_object().unwrap_or(&empty), b["effect"].as_object().unwrap_or(&empty));
    let mut fields: Vec<&String> = ea.keys().chain(eb.keys()).filter(|k| ea.get(*k) != eb.get(*k)).collect();
    fields.sort();
    fields.dedup();
    let changed = |key: &str| -> Vec<String> {
        b[key]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(id, v)| a[key].get(id.as_str()).is_some_and(|old| old != *v))
            .map(|(id, _)| id.clone())
            .collect()
    };
    let (moved, tuned) = (changed("rects"), changed("levels"));
    let mut parts: Vec<(&str, String)> = Vec::new();
    if !fields.is_empty() {
        let what = if fields.iter().any(|f| *f == "kind") { "kind".to_string() } else { fields.iter().map(|f| f.as_str()).collect::<Vec<_>>().join(",") };
        parts.push(("effect", what));
    }
    if a["layers"] != b["layers"] {
        parts.push(("layers", String::new()));
    }
    if a["overrides"] != b["overrides"] {
        parts.push(("device effect", String::new()));
    }
    if a["brightness"] != b["brightness"] {
        parts.push(("brightness", String::new()));
    }
    if !tuned.is_empty() {
        parts.push(("device", tuned.join(",")));
    }
    if !moved.is_empty() {
        parts.push(("layout", moved.join(",")));
    }
    if parts.is_empty() {
        return None;
    }
    let label = parts.iter().map(|p| p.0).collect::<Vec<_>>().join(" and ");
    Some((label, format!("{parts:?}")))
}

fn merged(last: &Value, now: &Value) -> Value {
    let mut out = now.clone();
    for key in ["rects", "levels"] {
        let mut map = last[key].as_object().cloned().unwrap_or_default();
        map.extend(now[key].as_object().cloned().unwrap_or_default());
        out[key] = Value::Object(map);
    }
    out
}

fn track(e: &mut Ed, snap: &Value) {
    let h = &mut e.hist;
    if h.quiet > 0 || snap.is_null() {
        return;
    }
    let now = undoable(snap);
    let last = (h.at >= 0).then(|| h.steps.get(h.at as usize).cloned()).flatten();
    let change = last.as_ref().and_then(|l| difference(&l.state, &now));
    let tip = h.at > 0 && h.at as usize == h.steps.len() - 1;
    let recent = h.stamp.is_some_and(|t| t.elapsed() < Duration::from_millis(600));
    match (&change, last) {
        (None, last) => {
            h.at = h.at.max(0);
            let step = match last {
                Some(l) => Step { state: merged(&l.state, &now), ..l },
                None => Step { state: now, label: String::new(), key: String::new() },
            };
            match h.steps.get_mut(h.at as usize) {
                Some(slot) => *slot = step,
                None => h.steps.push(step),
            }
        }
        (Some((_, key)), Some(l)) if tip && l.key == *key && recent => {
            let at = h.at as usize;
            h.steps[at].state = now.clone();
            if difference(&h.steps[at - 1].state, &now).is_none() {
                h.steps.truncate(at);
                h.at -= 1;
            }
        }
        (Some((label, key)), _) => {
            h.steps.truncate((h.at + 1) as usize);
            h.steps.push(Step { state: now, label: label.clone(), key: key.clone() });
            if h.steps.len() > 100 {
                h.steps.remove(0);
            }
            h.at = h.steps.len() as i32 - 1;
        }
    }
    if change.is_some() {
        h.stamp = Some(Instant::now());
    }
}

/// Undo with -1, redo with 1.
fn step(app: &AppHandle, by: i32) {
    let plan = with_view(|v| {
        ed(|e| {
            let h = &mut e.hist;
            let to = h.steps.get((h.at + by).max(-1) as usize)?.state.clone();
            if h.at + by < 0 {
                return None;
            }
            let from = h.sent.clone().unwrap_or_else(|| undoable(&v.snap));
            let label = h.steps[if by < 0 { h.at } else { h.at + 1 } as usize].label.clone();
            h.at += by;
            h.sent = Some(to.clone());
            h.quiet += 1;
            Some((from, to, label, v.snap["devices"].clone()))
        })
    });
    let Some((from, to, label, devices)) = plan else {
        return;
    };
    let verb = if by < 0 { "Undid" } else { "Redid" };
    let note = tr(app, &format!("{verb} {label} change"));
    with_win(|w| w.global::<StagePage>().set_note(note.into()));

    if from["effect"] != to["effect"]
        && let Ok(effect) = serde_json::from_value(to["effect"].clone())
    {
        report(commands::set_effect(app.clone(), shared(app), effect));
    }
    if from["layers"] != to["layers"]
        && let Ok(layers) = serde_json::from_value(to["layers"].clone())
    {
        report(commands::set_layers(app.clone(), shared(app), layers));
    }
    let empty = serde_json::Map::new();
    let (fo, to_o) = (from["overrides"].as_object().unwrap_or(&empty), to["overrides"].as_object().unwrap_or(&empty));
    let mut ids: Vec<&String> = fo.keys().chain(to_o.keys()).collect();
    ids.sort();
    ids.dedup();
    for id in ids {
        if fo.get(id) != to_o.get(id) {
            let effect = to_o.get(id).filter(|v| !v.is_null()).and_then(|v| serde_json::from_value(v.clone()).ok());
            report(commands::set_override(app.clone(), shared(app), id.clone(), effect));
        }
    }
    if from["brightness"] != to["brightness"] {
        let value = to["brightness"].as_f64().unwrap_or(1.0) as f32;
        report(commands::set_brightness(app.clone(), shared(app), value));
    }
    for (id, rect) in to["rects"].as_object().into_iter().flatten() {
        let known = devices.as_array().into_iter().flatten().any(|d| d["id"] == id.as_str());
        let Some(old) = from["rects"].get(id) else {
            continue;
        };
        if !known || (old == rect && from["levels"].get(id) == to["levels"].get(id)) {
            continue;
        }
        let level = &to["levels"][id];
        let r: [f32; 4] = serde_json::from_value(rect.clone()).unwrap_or_default();
        report(commands::set_device(
            app.clone(),
            shared(app),
            id.clone(),
            level[0].as_bool().unwrap_or(true),
            level[1].as_f64().unwrap_or(1.0) as f32,
            r.map(|x| (x * 1e4).round() / 1e4),
        ));
    }
    with_win(|w| {
        let g = w.global::<EffectsPage>();
        ed(|e| {
            g.set_can_undo(e.hist.at > 0);
            g.set_can_redo(((e.hist.at + 1) as usize) < e.hist.steps.len());
        });
    });
    let a = app.clone();
    Timer::single_shot(Duration::from_millis(150), move || {
        let settled = ed(|e| {
            e.hist.quiet -= 1;
            e.hist.quiet == 0
        });
        if settled {
            ed(|e| e.hist.sent = None);
            redraw(&a);
        }
    });
}
