use slint::ComponentHandle;
use tauri::AppHandle;

use super::{AppWindow, ProfilesPage, View, fill, report, s, shared};
use crate::commands;

pub fn bind(w: &AppWindow, app: &AppHandle) {
    let g = w.global::<ProfilesPage>();
    let a = app.clone();
    g.on_load_profile(move |name| {
        report(commands::load_profile(a.clone(), shared(&a), name.to_string()))
    });
    let a = app.clone();
    g.on_save_profile(move |name| {
        report(commands::save_profile(a.clone(), shared(&a), name.to_string()))
    });
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
