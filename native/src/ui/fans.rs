use slint::ComponentHandle;
use tauri::AppHandle;

use super::{AppWindow, FansPage, Line, View, fill};

pub fn bind(_w: &AppWindow, _app: &AppHandle) {}

pub fn event(w: &AppWindow, app: &AppHandle, view: &mut View, name: &str) {
    if name == "fans" {
        apply(w, app, view);
    }
}

pub fn apply(w: &AppWindow, _app: &AppHandle, view: &mut View) {
    let g = w.global::<FansPage>();
    let f = view.events.get("fans").cloned().unwrap_or_default();
    let status = match f["service"].as_str() {
        Some("running") => format!(
            "Fan service running{}",
            f["version"].as_str().map(|v| format!(", version {v}")).unwrap_or_default()
        ),
        Some("error") => format!("Fan service error: {}", f["detail"].as_str().unwrap_or("")),
        Some(_) => "Fan service stopped".into(),
        None => "Asking the fan service…".into(),
    };
    g.set_fan_status(status.into());
    let mut rows: Vec<Line> = Vec::new();
    for s in f["sensors"].as_array().into_iter().flatten() {
        rows.push(Line {
            title: s["name"].as_str().unwrap_or_default().into(),
            detail: s["value"]
                .as_f64()
                .map_or("–".into(), |v| format!("{v:.1} °C"))
                .into(),
            ok: true,
        });
    }
    for fan in f["fans"].as_array().into_iter().flatten() {
        let rpm = fan["rpm"].as_u64().map_or("–".into(), |r| format!("{r} rpm"));
        let duty = fan["duty"]
            .as_f64()
            .map_or("auto".into(), |d| format!("{:.0}%", d * 100.0));
        rows.push(Line {
            title: fan["name"].as_str().unwrap_or_default().into(),
            detail: format!("{rpm}  {duty}").into(),
            ok: fan["error"].is_null(),
        });
    }
    fill(g.get_fans(), rows, |m| g.set_fans(m));
}
