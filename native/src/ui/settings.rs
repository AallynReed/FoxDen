use serde_json::json;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use tauri::AppHandle;

use super::{AppWindow, I18n, Line, SettingsPage, View, edit_settings, fill, s, tr};

pub const LANGS: [(&str, &str); 16] = [
    ("en", "English"),
    ("pt-PT", "Português (Portugal)"),
    ("es", "Español"),
    ("de", "Deutsch"),
    ("fr", "Français"),
    ("ru", "Русский"),
    ("zh-Hans", "简体中文"),
    ("pl", "Polski"),
    ("tr", "Türkçe"),
    ("ja", "日本語"),
    ("ko", "한국어"),
    ("it", "Italiano"),
    ("uk", "Українська"),
    ("zh-Hant", "繁體中文"),
    ("nl", "Nederlands"),
    ("cs", "Čeština"),
];

pub fn bind(w: &AppWindow, app: &AppHandle) {
    let g = w.global::<SettingsPage>();
    let a = app.clone();
    g.on_set_close_to_tray(move |on| edit_settings(&a, |s| s["close_to_tray"] = json!(on)));
    let a = app.clone();
    g.on_set_start_minimized(move |on| edit_settings(&a, |s| s["start_minimized"] = json!(on)));
    let a = app.clone();
    g.on_set_language(move |i| {
        let code = match i {
            0 => "auto",
            i => LANGS.get(i as usize - 1).map_or("auto", |l| l.0),
        };
        edit_settings(&a, |s| s["language"] = json!(code));
    });
}

pub fn event(_w: &AppWindow, _app: &AppHandle, _view: &mut View, _name: &str) {}

pub fn apply(w: &AppWindow, app: &AppHandle, view: &mut View) {
    let g = w.global::<SettingsPage>();
    let snap = &view.snap;
    let settings = &snap["settings"];
    let choice = settings["language"].as_str().unwrap_or("auto");
    w.global::<I18n>().set_lang(crate::i18n::language(choice).into());
    let mut langs: Vec<SharedString> = vec![tr(app, "Automatic").into()];
    langs.extend(LANGS.iter().map(|l| SharedString::from(l.1)));
    g.set_languages(ModelRc::new(VecModel::from(langs)));
    g.set_language_index(
        LANGS
            .iter()
            .position(|l| l.0 == choice)
            .map_or(0, |i| i as i32 + 1),
    );
    g.set_close_to_tray(settings["close_to_tray"].as_bool().unwrap_or(true));
    g.set_start_minimized(settings["start_minimized"].as_bool().unwrap_or(false));
    fill(
        g.get_backends(),
        snap["backends"]
            .as_array()
            .map(|l| {
                l.iter()
                    .map(|b| {
                        let active = b["active"].as_bool().unwrap_or(false);
                        let detail = match b["error"].as_str() {
                            Some(e) => e.to_string(),
                            None if active => format!("{} devices", b["devices"]),
                            None => "Off".into(),
                        };
                        Line {
                            title: s(&b["name"]),
                            detail: detail.into(),
                            ok: b["error"].is_null(),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default(),
        |m| g.set_backends(m),
    );
}
