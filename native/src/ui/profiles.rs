use std::time::Duration;

use serde_json::Value;
use slint::{ComponentHandle, Timer};
use tauri::AppHandle;

use super::{AppWindow, ProfilesPage, StagePage, View, clipboard, fill, report, s, shared, tr, with_view, with_win};
use crate::commands;

const MAX_STAGES: usize = 8;

thread_local! {
    static NOTE: Timer = Timer::default();
}

fn note(text: String) {
    with_win(|w| w.global::<StagePage>().set_note(text.into()));
    NOTE.with(|t| {
        t.start(slint::TimerMode::SingleShot, Duration::from_secs(8), || {
            with_win(|w| w.global::<StagePage>().set_note("".into()))
        })
    });
}

fn ok(r: Result<(), String>) -> bool {
    let done = r.is_ok();
    report(r);
    done
}

/// Names keep to 40 characters, as the web field does.
fn clip(name: &str) -> String {
    name.chars().take(40).collect::<String>().trim().to_string()
}

fn names() -> Vec<String> {
    with_view(|v| {
        v.snap["profiles"]
            .as_array()
            .map(|l| l.iter().filter_map(|n| n.as_str().map(String::from)).collect())
            .unwrap_or_default()
    })
}

pub fn bind(w: &AppWindow, app: &AppHandle) {
    let g = w.global::<ProfilesPage>();

    let a = app.clone();
    let weak = w.as_weak();
    g.on_save_new(move |name| {
        let Some(w) = weak.upgrade() else { return };
        let g = w.global::<ProfilesPage>();
        let name = clip(&name);
        if name.is_empty() {
            g.set_new_problem(tr(&a, "Enter a name for the profile.").into());
            return;
        }
        if names().contains(&name) && g.get_replace() != name.as_str() {
            g.set_replace(name.as_str().into());
            g.set_new_problem(
                tr(&a, &format!("A profile named \"{name}\" already exists. Save again to replace it.")).into(),
            );
            return;
        }
        if ok(commands::save_profile(a.clone(), shared(&a), name.clone())) {
            g.set_new_name("".into());
            g.set_replace("".into());
            g.set_new_problem("".into());
            note(tr(&a, &format!("Saved profile {name}")));
        }
    });

    let a = app.clone();
    g.on_load(move |name| report(commands::load_profile(a.clone(), shared(&a), name.to_string())));

    let a = app.clone();
    g.on_delete(move |name| {
        if ok(commands::delete_profile(a.clone(), shared(&a), name.to_string())) {
            note(tr(&a, &format!("Deleted profile {name}")));
        }
    });

    let weak = w.as_weak();
    g.on_rename(move |name| {
        if let Some(w) = weak.upgrade() {
            ask(&w, "Rename profile", &name, &name);
        }
    });
    let weak = w.as_weak();
    g.on_duplicate(move |name| {
        if let Some(w) = weak.upgrade() {
            ask(&w, "Duplicate profile", &format!("{name} copy"), &name);
        }
    });

    let a = app.clone();
    let weak = w.as_weak();
    g.on_name_submit(move |to| {
        let Some(w) = weak.upgrade() else { return };
        let g = w.global::<ProfilesPage>();
        let to = clip(&to);
        let from = FROM.with_borrow(Clone::clone);
        let rename = g.get_name_title() == "Rename profile";
        let own = if rename { Some(from.as_str()) } else { None };
        let problem = if to.is_empty() {
            tr(&a, "Enter a name.")
        } else if own != Some(to.as_str()) && names().contains(&to) {
            tr(&a, &format!("A profile named \"{to}\" already exists."))
        } else {
            String::new()
        };
        g.set_name_problem(problem.as_str().into());
        if !problem.is_empty() {
            return;
        }
        g.set_name_open(false);
        if !(rename && to == from) {
            copy_profile(&a, &from, &to, rename);
        }
    });

    let a = app.clone();
    let weak = w.as_weak();
    g.on_copy_effect(move || {
        let text = current_json();
        match clipboard::copy(&text) {
            Ok(()) => note(tr(&a, "Effect copied as JSON")),
            Err(e) => {
                crate::log::warn("ui", e);
                if let Some(w) = weak.upgrade() {
                    open_json(&w, "Copying was blocked. Press Ctrl+C to copy the selected text.");
                }
            }
        }
    });
    let weak = w.as_weak();
    g.on_paste_effect(move || {
        if let Some(w) = weak.upgrade() {
            open_json(&w, "Paste an effect over this one, or edit it, then choose Apply.");
        }
    });

    let a = app.clone();
    let weak = w.as_weak();
    g.on_json_edited(move |text| {
        if let Some(w) = weak.upgrade() {
            let problem = read_effect(&a, &text).err().unwrap_or_default();
            w.global::<ProfilesPage>().set_json_problem(problem.into());
        }
    });
    let a = app.clone();
    let weak = w.as_weak();
    g.on_json_apply(move |text| {
        let Some(w) = weak.upgrade() else { return };
        let g = w.global::<ProfilesPage>();
        match read_effect(&a, &text) {
            Ok(effect) => {
                g.set_json_problem("".into());
                g.set_json_open(false);
                if ok(commands::set_effect(a.clone(), shared(&a), effect)) {
                    note(tr(&a, "Effect applied"));
                }
            }
            Err(problem) => g.set_json_problem(problem.into()),
        }
    });
}

thread_local! {
    static FROM: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

fn ask(w: &AppWindow, title: &str, value: &str, from: &str) {
    FROM.with_borrow_mut(|f| *f = from.to_string());
    let g = w.global::<ProfilesPage>();
    g.set_name_title(title.into());
    g.set_name_text(value.into());
    g.set_name_problem("".into());
    g.set_name_open(true);
}

/// A profile can only be saved from the live state, so the source is loaded, saved under the
/// new name, and the live state is then put back.
fn copy_profile(app: &AppHandle, from: &str, to: &str, remove: bool) {
    let (effect, brightness, active) = with_view(|v| {
        (
            v.snap["effect"].clone(),
            v.snap["brightness"].as_f64().unwrap_or(1.0) as f32,
            v.snap["active_profile"].as_str().map(String::from),
        )
    });
    let load = |name: &str| ok(commands::load_profile(app.clone(), shared(app), name.to_string()));
    let loaded = active.as_deref() == Some(from) || load(from);
    let saved = loaded && ok(commands::save_profile(app.clone(), shared(app), to.to_string()));
    let gone = saved && remove && ok(commands::delete_profile(app.clone(), shared(app), from.to_string()));
    if active.as_deref() == Some(from) {
        if saved && !gone {
            load(from);
        }
    } else if let Some(active) = &active {
        load(active);
    } else if loaded {
        if let Ok(effect) = serde_json::from_value(effect) {
            report(commands::set_effect(app.clone(), shared(app), effect));
        }
        report(commands::set_brightness(app.clone(), shared(app), brightness));
    }
    if saved {
        let text = if gone { format!("Renamed to {to}") } else { format!("Duplicated as {to}") };
        note(tr(app, &text));
    }
}

fn current_json() -> String {
    with_view(|v| serde_json::to_string_pretty(&v.effect).unwrap_or_default())
}

fn open_json(w: &AppWindow, hint: &str) {
    let g = w.global::<ProfilesPage>();
    g.set_json_hint(hint.into());
    g.set_json_text(current_json().into());
    g.set_json_problem("".into());
    g.set_json_open(true);
}

fn color_problem(v: &Value) -> bool {
    !v.as_array().is_some_and(|c| {
        c.len() == 3
            && c.iter().all(|n| n.as_f64().is_some_and(|n| n.fract() == 0.0 && (0.0..=255.0).contains(&n)))
    })
}

fn param_problem(app: &AppHandle, control: &Value, value: &Value) -> Option<String> {
    let bad = match control["type"].as_str().unwrap_or_default() {
        "range" => {
            let (min, max) = (control["min"].as_f64()?, control["max"].as_f64()?);
            let ok = value.as_f64().is_some_and(|v| v >= min && v <= max);
            return (!ok).then(|| tr(app, &format!("a number from {} to {}", min as f32, max as f32)));
        }
        "color" => color_problem(value).then_some("three whole numbers from 0 to 255"),
        "toggle" => (!value.is_boolean()).then_some("true or false"),
        "sensor" => (!value.is_string()).then_some("a sensor id in quotes"),
        "battery" => (!value.is_string()).then_some("a device key in quotes, or empty quotes for the lowest battery"),
        "stops" => {
            let stage = |s: &Value| s["temp"].as_f64().is_some_and(f64::is_finite) && !color_problem(&s["color"]);
            let ok = value
                .as_array()
                .is_some_and(|l| (1..=MAX_STAGES).contains(&l.len()) && l.iter().all(stage));
            (!ok).then_some(if control["unit"] == "%" {
                "a list of 1 to 8 stages, each with a battery level and a colour"
            } else {
                "a list of 1 to 8 stages, each with a temperature and a colour"
            })
        }
        "choice" => {
            let options = control["options"].as_array()?;
            if options.contains(value) {
                return None;
            }
            let list: Vec<&str> = options.iter().filter_map(Value::as_str).collect();
            return Some(tr(app, &format!("one of {}", list.join(", "))));
        }
        _ => None,
    };
    bad.map(|b| tr(app, b))
}

/// Checks effect JSON against the catalogue. Gives the effect, or a sentence saying what is wrong.
fn read_effect(app: &AppHandle, text: &str) -> Result<foxden_core::Effect, String> {
    let data: Value = serde_json::from_str(text).map_err(|_| tr(app, "This is not valid JSON."))?;
    let Some(kind) = data.get("kind").and_then(Value::as_str) else {
        return Err(tr(app, "The effect needs a \"kind\", such as \"rainbow\"."));
    };
    let info = with_view(|v| {
        v.snap["catalog"]
            .as_array()
            .and_then(|c| c.iter().find(|i| i["default"]["kind"] == kind).cloned())
    })
    .ok_or_else(|| tr(app, &format!("There is no effect of kind \"{kind}\".")))?;
    let params = info["params"].as_array().cloned().unwrap_or_default();
    let fields = data.as_object().expect("has a kind");
    if let Some(extra) = fields.keys().find(|k| *k != "kind" && !params.iter().any(|p| p["name"] == k.as_str())) {
        let label = info["label"].as_str().unwrap_or(kind);
        return Err(tr(app, &format!("{label} has no parameter \"{extra}\".")));
    }
    let mut effect = info["default"].clone();
    for (k, v) in fields {
        effect[k] = v.clone();
    }
    for p in &params {
        let name = p["name"].as_str().unwrap_or_default();
        if let Some(problem) = param_problem(app, &p["control"], &effect[name]) {
            return Err(tr(app, &format!("\"{name}\" must be {problem}.")));
        }
    }
    serde_json::from_value(effect).map_err(|e| e.to_string())
}

pub fn event(_w: &AppWindow, _app: &AppHandle, _view: &mut View, _name: &str) {}

pub fn apply(w: &AppWindow, _app: &AppHandle, view: &mut View) {
    let g = w.global::<ProfilesPage>();
    let snap = &view.snap;
    fill(
        g.get_profiles(),
        snap["profiles"]
            .as_array()
            .map(|l| l.iter().map(s).collect())
            .unwrap_or_default(),
        |m| g.set_profiles(m),
    );
    g.set_active_profile(s(&snap["active_profile"]));
}
