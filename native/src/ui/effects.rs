use serde_json::{Value, json};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use tauri::AppHandle;

use super::{AppWindow, EffectItem, EffectsPage, Param, View, fill, report, s, shared, with_view};
use crate::commands;

pub fn bind(w: &AppWindow, app: &AppHandle) {
    let g = w.global::<EffectsPage>();
    let a = app.clone();
    g.on_pick_effect(move |kind| {
        let default = with_view(|v| {
            v.snap["catalog"]
                .as_array()
                .and_then(|c| c.iter().find(|e| e["default"]["kind"] == kind.as_str()))
                .map(|e| e["default"].clone())
        });
        if let Some(effect) = default {
            send_effect(&a, effect);
        }
    });
    let a = app.clone();
    g.on_set_param(move |name, v| edit_effect(&a, &name, json!(v)));
    let a = app.clone();
    g.on_set_param_bool(move |name, v| edit_effect(&a, &name, json!(v)));
    let a = app.clone();
    g.on_set_param_hex(move |name, text| match parse_hex(&text) {
        Some(rgb) => edit_effect(&a, &name, json!(rgb)),
        None => report(Err(format!("{text} is not a colour like #FF6A00"))),
    });
    let a = app.clone();
    g.on_set_param_choice(move |name, i| {
        let option = with_view(|v| {
            catalog_entry(v).and_then(|e| {
                e["params"]
                    .as_array()?
                    .iter()
                    .find(|p| p["name"] == name.as_str())?["control"]["options"]
                    .get(i as usize)
                    .cloned()
            })
        });
        if let Some(option) = option {
            edit_effect(&a, &name, option);
        }
    });
    let a = app.clone();
    g.on_reset_effect(move || {
        if let Some(e) = with_view(|v| catalog_entry(v)) {
            send_effect(&a, e["default"].clone());
        }
    });
    let a = app.clone();
    g.on_randomise_effect(move || {
        if let Some(effect) = with_view(randomised) {
            send_effect(&a, effect);
        }
    });
}

pub fn event(_w: &AppWindow, _app: &AppHandle, _view: &mut View, _name: &str) {}

pub fn apply(w: &AppWindow, _app: &AppHandle, view: &mut View) {
    let g = w.global::<EffectsPage>();
    let catalog = view.snap["catalog"].as_array().cloned().unwrap_or_default();
    fill(
        g.get_effects(),
        catalog
            .iter()
            .map(|e| EffectItem {
                kind: s(&e["default"]["kind"]),
                label: s(&e["label"]),
            })
            .collect(),
        |m| g.set_effects(m),
    );
    g.set_effect_kind(s(&view.effect["kind"]));
    let params = catalog_entry(view)
        .and_then(|e| e["params"].as_array().cloned())
        .unwrap_or_default();
    fill(
        g.get_params(),
        params
            .iter()
            .map(|p| param(p, &view.effect[p["name"].as_str().unwrap_or_default()]))
            .collect(),
        |m| g.set_params(m),
    );
}

pub fn parse_hex(text: &str) -> Option<[u8; 3]> {
    let t = text.trim().trim_start_matches('#');
    let v = u32::from_str_radix(t, 16).ok().filter(|_| t.len() == 6)?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

pub fn send_effect(app: &AppHandle, effect: Value) {
    match serde_json::from_value(effect) {
        Ok(effect) => report(commands::set_effect(app.clone(), shared(app), effect)),
        Err(e) => report(Err(e.to_string())),
    }
}

fn edit_effect(app: &AppHandle, name: &str, value: Value) {
    let effect = with_view(|view| {
        view.effect[name] = value;
        view.effect.clone()
    });
    send_effect(app, effect);
}

fn catalog_entry(view: &View) -> Option<Value> {
    let kind = view.effect["kind"].as_str()?;
    view.snap["catalog"]
        .as_array()?
        .iter()
        .find(|e| e["default"]["kind"] == kind)
        .cloned()
}

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

fn hue(h: f32) -> [u8; 3] {
    let f = |n: f32| {
        let k = (n + h * 6.0) % 6.0;
        (255.0 * (1.0 - k.min(4.0 - k).clamp(0.0, 1.0))) as u8
    };
    [f(5.0), f(3.0), f(1.0)]
}

fn randomised(v: &mut View) -> Option<Value> {
    let entry = catalog_entry(v)?;
    let mut effect = v.effect.clone();
    for p in entry["params"].as_array()? {
        let name = p["name"].as_str()?;
        let c = &p["control"];
        let r = random(&mut v.seed);
        effect[name] = match c["type"].as_str()? {
            "range" => {
                let (lo, hi, step) = (
                    c["min"].as_f64()? as f32,
                    c["max"].as_f64()? as f32,
                    c["step"].as_f64()? as f32,
                );
                json!((lo + (r * (hi - lo) / step).round() * step).min(hi))
            }
            "color" => json!(hue(r)),
            "toggle" => json!(r > 0.5),
            "choice" => {
                let o = c["options"].as_array()?;
                o[((r * o.len() as f32) as usize).min(o.len() - 1)].clone()
            }
            _ => continue,
        };
    }
    Some(effect)
}

pub fn param(p: &Value, value: &Value) -> Param {
    let c = &p["control"];
    let mut out = Param {
        name: s(&p["name"]),
        label: s(&p["label"]),
        control: "other".into(),
        ..Default::default()
    };
    match c["type"].as_str().unwrap_or_default() {
        "range" => {
            out.control = "range".into();
            out.min = c["min"].as_f64().unwrap_or(0.0) as f32;
            out.max = c["max"].as_f64().unwrap_or(1.0) as f32;
            out.step = c["step"].as_f64().unwrap_or(0.01) as f32;
            out.value = value.as_f64().unwrap_or(0.0) as f32;
        }
        "color" => {
            let [r, g, b]: [u8; 3] = serde_json::from_value(value.clone()).unwrap_or_default();
            out.control = "color".into();
            out.color = slint::Color::from_rgb_u8(r, g, b);
            out.hex = format!("#{r:02X}{g:02X}{b:02X}").into();
        }
        "toggle" => {
            out.control = "toggle".into();
            out.on = value.as_bool().unwrap_or(false);
        }
        "choice" => {
            let options: Vec<SharedString> = c["options"]
                .as_array()
                .map(|o| o.iter().map(s).collect())
                .unwrap_or_default();
            out.control = "choice".into();
            out.index = options
                .iter()
                .position(|o| Some(o.as_str()) == value.as_str())
                .unwrap_or(0) as i32;
            out.options = ModelRc::new(VecModel::from(options));
        }
        _ => {}
    }
    out
}
