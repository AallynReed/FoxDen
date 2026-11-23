use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use tauri::{AppHandle, Manager};

use super::{
    AppWindow, ChangeRow, NetCard, NetField, NetFound, SettingsPage, StagePage, StateRow, Strip, View,
    edit_settings, fill, jobs, report, s, shared, tr, with_view, with_win,
};

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

const BACKENDS: [(&str, &str); 6] = [
    ("openrgb", "OpenRGB"),
    ("hid", "Native HID"),
    ("board", "Boards, memory and cards"),
    ("net", "Network lights"),
    ("audio", "Audio capture"),
    ("screen", "Screen capture"),
];

const PROTOCOLS: [(&str, &str); 3] = [
    ("adalight", "Adalight"),
    ("tpm2", "TPM2"),
    ("keyboard_visualizer", "Keyboard Visualizer"),
];
const STRIP_KEYS: [&str; 4] = ["name", "port", "baud", "leds"];

#[derive(Clone, Copy)]
enum Ty {
    Ip,
    OptIp,
    Text,
    Check,
    Serial,
    Num(i64, i64),
    Choice(&'static [&'static str]),
}

struct Kind {
    key: &'static str,
    name: &'static str,
    pair: Option<&'static str>,
    blank: &'static str,
    fields: &'static [(&'static str, &'static str, Ty)],
}

const IP: (&str, &str, Ty) = ("ip", "Address", Ty::Ip);

const KINDS: [Kind; 17] = [
    Kind {
        key: "hue",
        name: "Philips Hue bridge",
        pair: Some("Press the link button on the bridge. FoxDen waits 35 seconds for it."),
        blank: r#"{"ip":"","username":"","clientkey":""}"#,
        fields: &[IP],
    },
    Kind {
        key: "nanoleaf",
        name: "Nanoleaf",
        pair: Some("Hold the power button on the controller for 5 to 7 seconds, until its lights flash. FoxDen waits 30 seconds."),
        blank: r#"{"ip":"","port":16021,"auth_token":""}"#,
        fields: &[IP, ("port", "Port", Ty::Num(1, 65535))],
    },
    Kind {
        key: "wiz",
        name: "Philips Wiz",
        pair: None,
        blank: r#"{"ip":"","use_cool_white":false,"use_warm_white":false,"white_strategy":"Average"}"#,
        fields: &[
            IP,
            ("white_strategy", "White from", Ty::Choice(&["Average", "Minimum", "None"])),
            ("use_cool_white", "Cool white", Ty::Check),
            ("use_warm_white", "Warm white", Ty::Check),
        ],
    },
    Kind {
        key: "lifx",
        name: "LIFX",
        pair: None,
        blank: r#"{"ip":"","name":"","multizone":false,"extended_multizone":false}"#,
        fields: &[
            IP,
            ("name", "Name", Ty::Text),
            ("multizone", "Multizone", Ty::Check),
            ("extended_multizone", "Extended multizone", Ty::Check),
        ],
    },
    Kind { key: "govee", name: "Govee", pair: None, blank: r#"{"ip":""}"#, fields: &[IP] },
    Kind {
        key: "yeelight",
        name: "Yeelight",
        pair: None,
        blank: r#"{"ip":"","host_ip":"","music_mode":false}"#,
        fields: &[IP, ("host_ip", "This PC's address", Ty::OptIp), ("music_mode", "Music mode", Ty::Check)],
    },
    Kind {
        key: "kasa",
        name: "Kasa Smart",
        pair: None,
        blank: r#"{"ip":"","name":""}"#,
        fields: &[IP, ("name", "Name", Ty::Text)],
    },
    Kind { key: "elgato_key_light", name: "Elgato Key Light", pair: None, blank: r#"{"ip":""}"#, fields: &[IP] },
    Kind { key: "elgato_light_strip", name: "Elgato Light Strip", pair: None, blank: r#"{"ip":""}"#, fields: &[IP] },
    Kind {
        key: "e131",
        name: "E1.31 (sACN)",
        pair: None,
        blank: r#"{"name":"","ip":"","num_leds":1,"start_universe":1,"start_channel":1,"keepalive_time":0,"universe_size":512}"#,
        fields: &[
            ("name", "Name", Ty::Text),
            ("ip", "Address, empty for multicast", Ty::OptIp),
            ("num_leds", "LEDs", Ty::Num(1, 8192)),
            ("start_universe", "First universe", Ty::Num(1, 63999)),
            ("start_channel", "First channel", Ty::Num(1, 512)),
            ("universe_size", "Universe size", Ty::Num(1, 512)),
            ("keepalive_time", "Keepalive (ms)", Ty::Num(0, 60000)),
        ],
    },
    Kind {
        key: "ddp",
        name: "DDP",
        pair: None,
        blank: r#"{"name":"","ip":"","port":4048,"num_leds":1,"keepalive_time":1000}"#,
        fields: &[
            ("name", "Name", Ty::Text),
            IP,
            ("port", "Port", Ty::Num(1, 65535)),
            ("num_leds", "LEDs", Ty::Num(1, 8192)),
            ("keepalive_time", "Keepalive (ms)", Ty::Num(0, 60000)),
        ],
    },
    Kind {
        key: "espurna",
        name: "Espurna",
        pair: None,
        blank: r#"{"ip":"","port":80,"apikey":""}"#,
        fields: &[IP, ("port", "Port", Ty::Num(1, 65535)), ("apikey", "API key", Ty::Text)],
    },
    Kind { key: "wled", name: "WLED / LeetDesk / SRGBmods WLC", pair: None, blank: r#"{"ip":""}"#, fields: &[IP] },
    Kind { key: "twinkly", name: "Twinkly", pair: None, blank: r#"{"ip":""}"#, fields: &[IP] },
    Kind { key: "cololight", name: "Cololight", pair: None, blank: r#"{"ip":""}"#, fields: &[IP] },
    Kind {
        key: "dmx",
        name: "DMX (serial)",
        pair: None,
        blank: r#"{"name":"","port":"COM3","keepalive_time":0,"red_channel":1,"green_channel":2,"blue_channel":3,"brightness_channel":0}"#,
        fields: &[
            ("name", "Name", Ty::Text),
            ("port", "Serial port", Ty::Serial),
            ("red_channel", "Red channel", Ty::Num(0, 512)),
            ("green_channel", "Green channel", Ty::Num(0, 512)),
            ("blue_channel", "Blue channel", Ty::Num(0, 512)),
            ("brightness_channel", "Brightness channel", Ty::Num(0, 512)),
            ("keepalive_time", "Keepalive (ms)", Ty::Num(0, 60000)),
        ],
    },
    Kind { key: "", name: "", pair: None, blank: "{}", fields: &[] },
];

fn kinds() -> &'static [Kind] {
    &KINDS[..KINDS.len() - 1]
}

fn kind(key: &str) -> Option<&'static Kind> {
    kinds().iter().find(|k| k.key == key)
}

const FEEDBACK_HOUR: usize = 5;
const FEEDBACK_WAIT: Duration = Duration::from_secs(30);
const FEEDBACK_TYPES: [&str; 4] = ["bug", "idea", "praise", "other"];
const ATTACHMENT_TYPES: [&str; 8] = ["png", "jpg", "jpeg", "gif", "webp", "mp4", "webm", "mov"];
const ATTACHMENT_MOST: u64 = 16 * 1024 * 1024;

#[derive(Default)]
struct Local {
    net_found: Vec<Value>,
    net_status: String,
    net_searching: bool,
    pairing: String,
    net_kind: usize,
    net_key: String,
    net_cards: Vec<(&'static str, usize)>,
    net_problems: HashMap<(usize, usize), String>,
    strip_key: String,
    strip_text: HashMap<(usize, &'static str), String>,
    strip_protocol: HashMap<usize, usize>,
    strip_problems: HashMap<(usize, &'static str), String>,
    host_problem: String,
    port_problem: String,
    fps_problem: String,
    modules: Option<(bool, String)>,
    mixer: Value,
    vmic_outcome: String,
    vmic_progress: String,
    vmic_busy: bool,
    conflicts: Vec<Value>,
    conflict_exes: Vec<Option<String>>,
    conflicts_asked: u32,
    conflicts_last: String,
    update: Option<Value>,
    update_status: String,
    update_progress: String,
    update_busy: bool,
    file: Option<(String, Vec<u8>)>,
    file_problem: String,
    feedback_status: String,
    feedback_waiting: bool,
    sent: Vec<Instant>,
}

thread_local! {
    static LOCAL: RefCell<Local> = RefCell::new(Local::default());
}

fn local<R>(f: impl FnOnce(&mut Local) -> R) -> R {
    LOCAL.with_borrow_mut(f)
}

/// Translates a template such as "Paired {0} at {1}", then fills it.
fn tf(app: &AppHandle, template: &str, args: &[&str]) -> String {
    let mut out = tr(app, template);
    for (i, a) in args.iter().enumerate() {
        out = out.replace(&format!("{{{i}}}"), &tr(app, a));
    }
    out
}

fn sentence(text: &str) -> String {
    let mut c = text.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

fn file_size(bytes: u64) -> String {
    if bytes < 1024 * 1024 {
        format!("{} KB", ((bytes as f64 / 1024.0).round() as u64).max(1))
    } else {
        format!("{:.1} MB", bytes as f64 / 1024.0 / 1024.0)
    }
}

fn note(app: &AppHandle, text: &str) {
    let text = tr(app, text);
    with_win(|w| w.global::<StagePage>().set_note(text.into()));
}

/// Re-applies this tab from a callback or a result that came back to the UI thread.
fn redraw(app: &AppHandle) {
    with_view(|view| with_win(|w| apply(w, app, view)));
}

/// Runs blocking work on a thread and hands its result back to the UI thread.
fn later<T: Send + 'static>(
    app: &AppHandle,
    work: impl FnOnce(&AppHandle) -> T + Send + 'static,
    done: impl FnOnce(&AppHandle, T) + Send + 'static,
) {
    let app = app.clone();
    std::thread::Builder::new().name("foxden-settings-job".into()).spawn(move || {
        let out = work(&app);
        let _ = slint::invoke_from_event_loop(move || {
            done(&app, out);
            redraw(&app);
        });
    }).expect("thread start");
}

/// The settings as the backend holds them now, which a save changes at once.
fn snap_settings(app: &AppHandle) -> Value {
    serde_json::to_value(&crate::state::lock(&shared(app)).config.settings).unwrap_or_default()
}

fn entry<'a>(net: &'a mut Value, kind: &str, idx: usize) -> Option<&'a mut Value> {
    net.get_mut(kind)?.get_mut(idx)
}

fn local_ip(text: &str) -> bool {
    if let Ok(v4) = text.parse::<std::net::Ipv4Addr>() {
        let [a, b, ..] = v4.octets();
        return a == 10 || a == 127 || (a == 172 && (16..=31).contains(&b)) || (a == 192 && b == 168) || (a == 169 && b == 254);
    }
    let t = text.to_ascii_lowercase();
    if t == "::1" {
        return true;
    }
    let hex = |s: &str| s.chars().all(|c| c.is_ascii_hexdigit() || c == ':');
    t.len() > 4
        && hex(&t)
        && (t.starts_with("fc") || t.starts_with("fd") || ["fe8", "fe9", "fea", "feb"].iter().any(|p| t.starts_with(p)))
        && t.as_bytes().get(4) == Some(&b':')
}

fn whole(app: &AppHandle, text: &str, min: i64, max: i64) -> Result<Value, String> {
    match text.parse::<i64>() {
        Ok(n) if (min..=max).contains(&n) => Ok(json!(n)),
        _ => Err(tf(app, "Enter a whole number from {0} to {1}.", &[&min.to_string(), &max.to_string()])),
    }
}

fn net_check(app: &AppHandle, ty: Ty, text: &str) -> Result<Value, String> {
    let text = text.trim();
    match ty {
        Ty::Text | Ty::Check | Ty::Choice(_) => Ok(json!(text)),
        Ty::OptIp if text.is_empty() => Ok(json!("")),
        Ty::Ip | Ty::OptIp if local_ip(text) => Ok(json!(text)),
        Ty::Ip | Ty::OptIp => Err(tr(app, "Enter an address on this network, such as 192.168.1.20.")),
        Ty::Serial => {
            let ok = text.len() > 3
                && text[..3].eq_ignore_ascii_case("com")
                && text.len() <= 6
                && text[3..].chars().all(|c| c.is_ascii_digit());
            if ok {
                Ok(json!(text.to_uppercase()))
            } else {
                Err(tr(app, "Enter a serial port, such as COM3."))
            }
        }
        Ty::Num(min, max) => whole(app, text, min, max),
    }
}

fn strip_check(app: &AppHandle, key: &str, text: &str, protocol: &str, port: &str) -> Result<Value, String> {
    let text = text.trim();
    let kv = protocol == "keyboard_visualizer";
    let com = |t: &str| t.len() > 3 && t.len() <= 6 && t[..3].eq_ignore_ascii_case("com") && t[3..].chars().all(|c| c.is_ascii_digit());
    let udp = |t: &str| t.len() > 4 && t[..4].eq_ignore_ascii_case("udp:") && !t[4..].contains(char::is_whitespace);
    match key {
        "name" => Ok(json!(text)),
        "port" if com(text) => Ok(json!(text.to_uppercase())),
        "port" if kv && udp(text) => Ok(json!(format!("udp:{}", &text[4..]))),
        "port" if kv => Err(tr(app, "Enter a serial port such as COM3, or udp:address.")),
        "port" => Err(tr(app, "Enter a serial port, such as COM3.")),
        "leds" => whole(app, text, 1, 8192),
        _ => whole(app, text, 1, if kv && udp(port.trim()) { 65535 } else { 4_000_000 }),
    }
}

fn value_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        v => v.to_string(),
    }
}

/// The MCP client config that starts foxden-mcp.exe from beside this app.
fn ai_setup() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let command = exe.with_file_name("foxden-mcp.exe");
    let config = json!({"mcpServers": {"foxden": {"command": command.to_string_lossy()}}});
    serde_json::to_string_pretty(&config).map_err(|e| e.to_string())
}

pub fn bind(w: &AppWindow, app: &AppHandle) {
    local(|l| {
        l.net_key.clear();
        l.strip_key.clear();
        l.net_problems.clear();
        l.strip_problems.clear();
    });
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
    let a = app.clone();
    g.on_set_flag(move |key, on| {
        if key == "network.discover" {
            edit_settings(&a, |s| s["network"]["discover"] = json!(on));
        } else {
            edit_settings(&a, |s| s[key.as_str()] = json!(on));
        }
    });

    let a = app.clone();
    g.on_copy_ai_setup(move || match ai_setup().and_then(|text| super::clipboard::copy(&text)) {
        Ok(()) => {
            with_win(|w| w.global::<SettingsPage>().set_ai_copied(tr(&a, "Copied").into()));
            slint::Timer::single_shot(Duration::from_secs(3), || {
                with_win(|w| w.global::<SettingsPage>().set_ai_copied("".into()));
            });
        }
        Err(e) => report(Err(e)),
    });

    let a = app.clone();
    g.on_set_fps(move |fps| {
        local(|l| l.fps_problem.clear());
        edit_settings(&a, |s| s["fps"] = json!(fps));
    });
    let a = app.clone();
    g.on_fps_typed(move |text| {
        let n = text.trim().parse::<i64>().ok().filter(|n| (10..=60).contains(n));
        let problem = match n {
            Some(_) => String::new(),
            None => tf(&a, "{0} must be from {1} to {2}{3}.", &["Frame rate", "10", "60", ""]),
        };
        local(|l| l.fps_problem = problem);
        if let Some(n) = n {
            edit_settings(&a, |s| s["fps"] = json!(n));
        }
        redraw(&a);
    });

    let a = app.clone();
    g.on_server_edited(move |key, text| {
        let problem = server_check(&a, &key, &text).err().unwrap_or_default();
        local(|l| match key.as_str() {
            "host" => l.host_problem = problem,
            _ => l.port_problem = problem,
        });
        redraw(&a);
    });
    let a = app.clone();
    g.on_server_commit(move |key, text| {
        if let Ok(value) = server_check(&a, &key, &text) {
            let name = if key == "host" { "openrgb_host" } else { "openrgb_port" };
            if snap_settings(&a)[name] != value {
                edit_settings(&a, |s| s[name] = value);
            }
        }
    });

    bind_network(w, app);
    bind_strips(w, app);
    bind_vmic(w, app);
    bind_conflicts(w, app);
    bind_updates(w, app);
    bind_feedback(w, app);

    let a = app.clone();
    g.on_open_logs(move || super::logs::open(&a));
    let a = app.clone();
    g.on_rescan(move || {
        let r = crate::commands::rescan(jobs(&a));
        if r.is_ok() {
            note(&a, "Rescanning devices");
        }
        report(r);
    });
    let a = app.clone();
    g.on_close_sheet(move || {
        with_win(|w| w.global::<SettingsPage>().set_sheet("".into()));
        redraw(&a);
    });

    refresh_vmic(app);
    refresh_conflicts(app);
    local(|l| {
        if let Some(r) = crate::commands::last_conflicts() {
            l.conflicts_last = closed_line(app, &r.text, &r.at);
        }
    });
}

fn server_check(app: &AppHandle, key: &str, text: &str) -> Result<Value, String> {
    if key == "host" {
        let t = text.trim();
        if t.is_empty() {
            return Err(tr(app, "Enter a host name or address."));
        }
        return Ok(json!(t));
    }
    match text.trim().parse::<u16>() {
        Ok(n) if n >= 1 => Ok(json!(n)),
        _ => Err(tr(app, "Port must be a whole number from 1 to 65535.")),
    }
}

fn closed_line(app: &AppHandle, text: &str, at: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let text = tr(app, text);
    if at.is_empty() { text } else { format!("{text}{}", tf(app, " at {0}", &[at])) }
}

fn save_net(app: &AppHandle, change: impl FnOnce(&mut Value)) {
    local(|l| l.net_problems.clear());
    edit_settings(app, |s| change(&mut s["network"]));
}

fn pair(app: &AppHandle, family: &'static str, ip: String, port: u16) {
    let Some(spec) = kind(family) else { return };
    if !local(|l| l.pairing.is_empty()) || !local_ip(&ip) {
        return;
    }
    local(|l| {
        l.pairing = format!("{family}{ip}");
        l.net_status = tr(app, spec.pair.unwrap_or_default());
    });
    redraw(app);
    let ip2 = ip.clone();
    later(
        app,
        move |app| {
            let (shared, jobs) = (app.state(), app.state());
            if family == "hue" {
                crate::commands::pair_hue(app.clone(), shared, jobs, ip2)
            } else {
                crate::commands::pair_nanoleaf(app.clone(), shared, jobs, ip2, port)
            }
        },
        move |app, out| {
            let status = match &out {
                Ok(()) => tf(app, "Paired {0} at {1}", &[spec.name, &ip]),
                Err(_) => String::new(),
            };
            local(|l| {
                l.pairing.clear();
                l.net_status = status;
            });
            report(out);
        },
    );
}

fn bind_network(w: &AppWindow, app: &AppHandle) {
    let g = w.global::<SettingsPage>();
    let a = app.clone();
    g.on_net_search(move || {
        if local(|l| std::mem::replace(&mut l.net_searching, true)) {
            return;
        }
        local(|l| l.net_status = tr(&a, "Searching"));
        redraw(&a);
        later(
            &a,
            |app| crate::commands::scan_network(app.state()),
            |app, out| {
                let status = match &out {
                    Ok(found) if found.is_empty() => tr(app, "Nothing found"),
                    Ok(found) => tf(app, "Found {0}", &[&plural(found.len(), "light")]),
                    Err(_) => String::new(),
                };
                local(|l| {
                    l.net_searching = false;
                    l.net_status = status;
                    if let Ok(found) = &out {
                        l.net_found = found.iter().filter_map(|f| serde_json::to_value(f).ok()).collect();
                    }
                });
                report(out.map(|_| ()));
            },
        );
    });
    let a = app.clone();
    g.on_net_pick_kind(move |i| {
        local(|l| l.net_kind = i.max(0) as usize);
        redraw(&a);
    });
    let a = app.clone();
    g.on_net_add(move || {
        let spec = &kinds()[local(|l| l.net_kind).min(kinds().len() - 1)];
        let blank: Value = serde_json::from_str(spec.blank).unwrap_or_default();
        save_net(&a, |n| push(&mut n[spec.key], blank));
    });
    let a = app.clone();
    g.on_net_found_act(move |i| {
        let Some(found) = local(|l| l.net_found.get(i as usize).cloned()) else { return };
        let family = found["family"].as_str().unwrap_or_default().to_string();
        let Some(spec) = kind(&family) else { return };
        let ip = s(&found["ip"]).to_string();
        if spec.pair.is_some() {
            let port = found["port"].as_u64().filter(|p| *p > 0).unwrap_or(16021) as u16;
            pair(&a, spec.key, ip, port);
            return;
        }
        let mut blank: Value = serde_json::from_str(spec.blank).unwrap_or_default();
        blank["ip"] = json!(ip);
        if blank.get("name").is_some() {
            blank["name"] = found["name"].clone();
        }
        save_net(&a, |n| push(&mut n[spec.key], blank));
    });
    let a = app.clone();
    g.on_net_edit(move |ci, fi, text| {
        net_field(&a, ci, fi, &text, false);
    });
    let a = app.clone();
    g.on_net_commit(move |ci, fi, text| {
        net_field(&a, ci, fi, &text, true);
    });
    let a = app.clone();
    g.on_net_check(move |ci, fi, on| {
        if let Some((spec, idx)) = card(ci) {
            let key = spec.fields[fi as usize].0;
            save_net(&a, |n| {
                if let Some(e) = entry(n, spec.key, idx) {
                    e[key] = json!(on);
                }
            });
        }
    });
    let a = app.clone();
    g.on_net_choice(move |ci, fi, o| {
        if let Some((spec, idx)) = card(ci)
            && let (key, _, Ty::Choice(options)) = spec.fields[fi as usize]
        {
            let value = options.get(o as usize).copied().unwrap_or_default();
            save_net(&a, |n| {
                if let Some(e) = entry(n, spec.key, idx) {
                    e[key] = json!(value);
                }
            });
        }
    });
    let a = app.clone();
    g.on_net_remove(move |ci| {
        if let Some((spec, idx)) = card(ci) {
            save_net(&a, |n| {
                if let Some(list) = n[spec.key].as_array_mut()
                    && idx < list.len()
                {
                    list.remove(idx);
                }
            });
        }
    });
    let a = app.clone();
    g.on_net_pair(move |ci| {
        let Some((spec, idx)) = card(ci) else { return };
        let item = snap_settings(&a)["network"][spec.key][idx].clone();
        let port = item["port"].as_u64().unwrap_or(16021) as u16;
        pair(&a, spec.key, s(&item["ip"]).to_string(), port);
    });
}

fn push(list: &mut Value, item: Value) {
    if !list.is_array() {
        *list = json!([]);
    }
    if let Some(l) = list.as_array_mut() {
        l.push(item);
    }
}

fn card(ci: i32) -> Option<(&'static Kind, usize)> {
    let (key, idx) = local(|l| l.net_cards.get(ci as usize).copied())?;
    Some((kind(key)?, idx))
}

fn net_field(app: &AppHandle, ci: i32, fi: i32, text: &str, commit: bool) {
    let Some((spec, idx)) = card(ci) else { return };
    let Some(&(key, _, ty)) = spec.fields.get(fi as usize) else { return };
    let out = net_check(app, ty, text);
    local(|l| match &out {
        Ok(_) => l.net_problems.remove(&(ci as usize, fi as usize)),
        Err(p) => l.net_problems.insert((ci as usize, fi as usize), p.clone()),
    });
    if commit
        && let Ok(value) = out
        && snap_settings(app)["network"][spec.key][idx][key] != value
    {
        save_net(app, |n| {
            if let Some(e) = entry(n, spec.key, idx) {
                e[key] = value;
            }
        });
        return;
    }
    with_win(net_problems);
}

fn net_problems(w: &AppWindow) {
    let g = w.global::<SettingsPage>();
    let rows: Vec<SharedString> = local(|l| {
        (0..l.net_cards.len())
            .map(|ci| {
                let first = l
                    .net_problems
                    .iter()
                    .filter(|((c, _), _)| *c == ci)
                    .min_by_key(|((_, f), _)| *f)
                    .map(|(_, p)| p.clone());
                first.unwrap_or_default().into()
            })
            .collect()
    });
    fill(g.get_net_problems(), rows, |m| g.set_net_problems(m));
}

fn bind_strips(w: &AppWindow, app: &AppHandle) {
    let g = w.global::<SettingsPage>();
    let a = app.clone();
    g.on_strip_add(move || {
        later(
            &a,
            |_| crate::commands::serial_ports(),
            |app, ports| {
                let settings = snap_settings(app);
                let used: Vec<&str> = settings["led_strips"]
                    .as_array()
                    .map(|l| l.iter().filter_map(|s| s["port"].as_str()).collect())
                    .unwrap_or_default();
                let port = ports.iter().find(|p| !used.contains(&p.as_str())).cloned().unwrap_or_default();
                save_strips(app, |s| {
                    push(s, json!({"name": "LED strip", "port": port, "baud": 115200, "leds": 60, "protocol": "adalight"}))
                });
            },
        );
    });
    let a = app.clone();
    g.on_strip_edit(move |i, key, text| strip_field(&a, i as usize, &key, &text, false));
    let a = app.clone();
    g.on_strip_commit(move |i, key, text| strip_field(&a, i as usize, &key, &text, true));
    let a = app.clone();
    g.on_strip_protocol(move |i, p| {
        local(|l| l.strip_protocol.insert(i as usize, p.max(0) as usize));
        let port = strip_text(&a, i as usize, "port");
        strip_field(&a, i as usize, "port", &port, true);
    });
    let a = app.clone();
    g.on_strip_remove(move |i| {
        save_strips(&a, |s| {
            if let Some(list) = s.as_array_mut()
                && (i as usize) < list.len()
            {
                list.remove(i as usize);
            }
        });
    });
}

fn save_strips(app: &AppHandle, change: impl FnOnce(&mut Value)) {
    local(|l| {
        l.strip_text.clear();
        l.strip_protocol.clear();
        l.strip_problems.clear();
    });
    edit_settings(app, |s| change(&mut s["led_strips"]));
}

/// What a strip field holds now: typed text, or the saved value.
fn strip_text(app: &AppHandle, i: usize, key: &'static str) -> String {
    if let Some(t) = local(|l| l.strip_text.get(&(i, key)).cloned()) {
        return t;
    }
    value_text(&snap_settings(app)["led_strips"][i][key])
}

fn strip_protocol(app: &AppHandle, i: usize) -> &'static str {
    if let Some(p) = local(|l| l.strip_protocol.get(&i).copied()) {
        return PROTOCOLS.get(p).map_or("adalight", |p| p.0);
    }
    let saved = snap_settings(app)["led_strips"][i]["protocol"].as_str().unwrap_or_default().to_string();
    PROTOCOLS.iter().find(|p| p.0 == saved).map_or("adalight", |p| p.0)
}

fn strip_field(app: &AppHandle, i: usize, key: &str, text: &str, commit: bool) {
    let Some(&key) = STRIP_KEYS.iter().find(|k| **k == key) else { return };
    local(|l| l.strip_text.insert((i, key), text.to_string()));
    let protocol = strip_protocol(app, i);
    let port = strip_text(app, i, "port");
    let recheck: &[&'static str] = if key == "port" { &["port", "baud"] } else { &[key] };
    for &k in recheck {
        let out = strip_check(app, k, &strip_text(app, i, k), protocol, &port);
        local(|l| match out {
            Ok(_) => l.strip_problems.remove(&(i, k)),
            Err(p) => l.strip_problems.insert((i, k), p),
        });
    }
    if commit {
        let all: Result<serde_json::Map<String, Value>, String> = STRIP_KEYS
            .iter()
            .map(|k| strip_check(app, k, &strip_text(app, i, k), protocol, &port).map(|v| (k.to_string(), v)))
            .collect();
        if let Ok(mut all) = all {
            all.insert("protocol".into(), json!(protocol));
            let item = Value::Object(all);
            if snap_settings(app)["led_strips"][i] != item {
                save_strips(app, |s| {
                    if let Some(slot) = s.get_mut(i) {
                        *slot = item;
                    }
                });
                return;
            }
        }
    }
    with_win(strip_problems);
}

fn strip_problems(w: &AppWindow) {
    let g = w.global::<SettingsPage>();
    let n = g.get_strips().row_count();
    let rows: Vec<SharedString> = local(|l| {
        (0..n)
            .map(|i| {
                STRIP_KEYS
                    .iter()
                    .find_map(|k| l.strip_problems.get(&(i, *k)).cloned())
                    .unwrap_or_default()
                    .into()
            })
            .collect()
    });
    fill(g.get_strip_problems(), rows, |m| g.set_strip_problems(m));
}

fn mixer_change(app: &AppHandle, on: bool) {
    later(
        app,
        move |app| {
            let change = serde_json::from_value(json!({"kind": "virtual_mic", "on": on})).map_err(|e| e.to_string())?;
            crate::mixer::mixer_set(app.state(), change)
        },
        |_, out| match out {
            Ok(view) => local(|l| l.mixer = serde_json::to_value(view).unwrap_or_default()),
            Err(e) => report(Err(e)),
        },
    );
}

fn refresh_vmic(app: &AppHandle) {
    if app.try_state::<crate::mixer::Mixer>().is_none() {
        return;
    }
    later(
        app,
        |app| crate::mixer::get_mixer(app.state(), false),
        |_, out| {
            if let Ok(view) = out {
                local(|l| l.mixer = serde_json::to_value(view).unwrap_or_default());
            }
        },
    );
}

fn bind_vmic(w: &AppWindow, app: &AppHandle) {
    let g = w.global::<SettingsPage>();
    let a = app.clone();
    g.on_set_vmic(move |on| {
        let installed = local(|l| l.mixer["vmic"]["installed"].as_bool().unwrap_or(false));
        if !on || installed {
            local(|l| l.mixer["config"]["virtual_mic"] = json!(on));
            mixer_change(&a, on);
        } else {
            local(|l| l.vmic_progress.clear());
            with_win(|w| w.global::<SettingsPage>().set_sheet("vmic".into()));
        }
        redraw(&a);
    });
    g.on_vmic_donate(move || report(crate::links::open_link("vbaudio".into())));
    let a2 = app.clone();
    g.on_vmic_check(move || {
        local(|l| {
            l.vmic_busy = true;
            l.vmic_progress = tr(&a2, "Looking for VB-CABLE…");
        });
        redraw(&a2);
        later(
            &a2,
            |app| crate::mixer::virtual_mic_install(app.state()),
            |app, out| {
                local(|l| l.vmic_busy = false);
                match out {
                    Ok(view) => {
                        local(|l| l.mixer = serde_json::to_value(view).unwrap_or_default());
                        with_win(|w| w.global::<SettingsPage>().set_sheet("".into()));
                    }
                    Err(e) => local(|l| l.vmic_progress = tr(app, &e)),
                }
            },
        );
    });
    let a = app.clone();
    g.on_vmic_remove(move || {
        local(|l| {
            l.vmic_busy = true;
            l.vmic_outcome = tr(&a, "Removing VB-CABLE. Windows asks for administrator rights.");
        });
        redraw(&a);
        later(
            &a,
            |app| crate::mixer::virtual_mic_remove(app.state()),
            |app, out| {
                local(|l| {
                    l.vmic_busy = false;
                    match out {
                        Ok(view) => {
                            l.mixer = serde_json::to_value(view).unwrap_or_default();
                            l.vmic_outcome = tr(app, "Removed. Restart Windows to finish.");
                        }
                        Err(e) => l.vmic_outcome = tr(app, &e),
                    }
                });
            },
        );
    });
}

fn refresh_conflicts(app: &AppHandle) {
    let asked = local(|l| {
        l.conflicts_asked += 1;
        l.conflicts_asked
    });
    later(
        app,
        |app| crate::commands::get_conflicts(app.clone(), app.state()),
        move |_, found| {
            local(|l| {
                if l.conflicts_asked == asked {
                    l.conflicts = found.iter().filter_map(|f| serde_json::to_value(f).ok()).collect();
                }
            });
        },
    );
}

fn bind_conflicts(w: &AppWindow, app: &AppHandle) {
    let g = w.global::<SettingsPage>();
    let a = app.clone();
    g.on_refresh_conflicts(move || refresh_conflicts(&a));
    let a = app.clone();
    g.on_close_conflicts_now(move || {
        later(
            &a,
            |app| crate::commands::close_conflicts_now(app.clone(), app.state()),
            |app, r| {
                note(app, &r.text);
                if !r.closed.is_empty() || !r.left.is_empty() {
                    let line = closed_line(app, &r.text, &r.at);
                    local(|l| l.conflicts_last = line);
                }
                refresh_conflicts(app);
            },
        );
    });
    let a = app.clone();
    g.on_set_exe_closes(move |i, closes| {
        let Some(Some(exe)) = local(|l| l.conflict_exes.get(i as usize).cloned()) else { return };
        edit_settings(&a, move |s| {
            let mut keep: Vec<Value> = s["keep_running"]
                .as_array()
                .map(|k| {
                    k.iter()
                        .filter(|n| !n.as_str().unwrap_or_default().eq_ignore_ascii_case(&exe))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            if !closes {
                keep.push(json!(exe));
            }
            s["keep_running"] = Value::Array(keep);
        });
        refresh_conflicts(&a);
    });
}

const REMIND: u64 = 60 * 60;

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn show_update(app: &AppHandle, info: &Value, announce: bool) {
    let version = s(&info["version"]).to_string();
    local(|l| {
        l.update = Some(info.clone());
        l.update_status = tf(app, "Version {0} available", &[&version]);
    });
    if announce {
        prompt_update(app);
    }
}

/// The first time the window opens on a newer version, shows what changed since the last one opened.
pub fn show_whats_new(app: &AppHandle) {
    let app = app.clone();
    slint::Timer::single_shot(Duration::ZERO, move || {
        let current = env!("CARGO_PKG_VERSION");
        let seen = s(&snap_settings(&app)["seen_version"]).to_string();
        if seen == current || !with_win_visible() {
            return;
        }
        edit_settings(&app, |s| s["seen_version"] = json!(current));
        if !seen.is_empty() && !crate::updater::newer(current, &seen) {
            return;
        }
        let all = crate::changelog::built_in();
        let entries: Vec<Value> = if seen.is_empty() {
            all.as_array().into_iter().flatten().filter(|e| e["version"] == current).cloned().collect()
        } else {
            crate::changelog::between(&all, &seen, current)
        };
        if entries.is_empty() {
            return;
        }
        let rows = log_rows(&app, &entries);
        with_win(|w| {
            let g = w.global::<SettingsPage>();
            if g.get_sheet().is_empty() {
                g.set_changelog(slint::ModelRc::new(VecModel::from(rows)));
                g.set_changelog_title(tf(&app, "FoxDen {0}", &[current]).into());
                g.set_sheet("changelog".into());
            }
        });
        redraw(&app);
    });
}

fn with_win_visible() -> bool {
    let mut visible = false;
    with_win(|w| visible = w.window().is_visible());
    visible
}

/// Opens the update popup on a visible window, unless this release was put off or ignored.
pub fn prompt_update(app: &AppHandle) {
    let app = app.clone();
    slint::Timer::single_shot(Duration::ZERO, move || {
        let Some(version) = local(|l| l.update.as_ref().map(|u| s(&u["version"]).to_string())) else {
            return;
        };
        let settings = snap_settings(&app);
        let ignored = settings["update_ignored"].as_str() == Some(version.as_str());
        if ignored || unix_now() < settings["update_remind_at"].as_u64().unwrap_or(0) {
            return;
        }
        with_win(|w| {
            let g = w.global::<SettingsPage>();
            if w.window().is_visible() && g.get_sheet().is_empty() {
                g.set_sheet("update".into());
            }
        });
        redraw(&app);
    });
}

/// Changelog entries as rows: a version line, then each section's heading and items.
fn log_rows(app: &AppHandle, entries: &[Value]) -> Vec<ChangeRow> {
    let row = |kind: &str, text: String| ChangeRow { kind: kind.into(), text: text.into() };
    let mut rows = Vec::new();
    for e in entries {
        rows.push(row("version", format!("{}  ·  {}", s(&e["version"]), s(&e["date"]))));
        for (key, label) in crate::changelog::SECTIONS {
            let items = e[key].as_array().into_iter().flatten().filter_map(Value::as_str);
            let mut items = items.peekable();
            if items.peek().is_some() {
                rows.push(row("heading", tr(app, label).to_uppercase()));
                rows.extend(items.map(|i| row("item", i.to_string())));
            }
        }
    }
    rows
}

fn bind_updates(w: &AppWindow, app: &AppHandle) {
    let g = w.global::<SettingsPage>();
    let a = app.clone();
    g.on_open_changelog(move || {
        let all = crate::changelog::built_in();
        let rows = log_rows(&a, all.as_array().map_or(&[][..], Vec::as_slice));
        with_win(|w| {
            let g = w.global::<SettingsPage>();
            g.set_changelog(slint::ModelRc::new(VecModel::from(rows)));
            g.set_changelog_title(tr(&a, "Changelog").into());
            g.set_sheet("changelog".into());
        });
    });
    if let Some(info) = crate::updater::get_update() {
        show_update(app, &serde_json::to_value(info).unwrap_or_default(), false);
    }
    g.on_open_update(move || {
        if local(|l| l.update.is_some()) {
            with_win(|w| w.global::<SettingsPage>().set_sheet("update".into()));
        }
    });
    let a2 = app.clone();
    g.on_later_update(move || {
        crate::updater::later_update();
        edit_settings(&a2, |s| s["update_remind_at"] = json!(unix_now() + REMIND));
        with_win(|w| w.global::<SettingsPage>().set_sheet("".into()));
    });
    let a = app.clone();
    g.on_ignore_update(move || {
        crate::updater::later_update();
        if let Some(version) = local(|l| l.update.as_ref().map(|u| s(&u["version"]).to_string())) {
            edit_settings(&a, |s| s["update_ignored"] = json!(version));
        }
        with_win(|w| w.global::<SettingsPage>().set_sheet("".into()));
    });
    let a = app.clone();
    g.on_install_update(move || {
        if local(|l| std::mem::replace(&mut l.update_busy, true)) {
            return;
        }
        local(|l| l.update_progress = tr(&a, "Downloading"));
        redraw(&a);
        let app = a.clone();
        tauri::async_runtime::spawn(async move {
            let out = crate::updater::install_update(app.clone()).await;
            let _ = slint::invoke_from_event_loop(move || {
                if let Err(e) = out {
                    local(|l| {
                        l.update_busy = false;
                        l.update_progress = sentence(&tr(&app, &e));
                    });
                }
                redraw(&app);
            });
        });
    });
    let a = app.clone();
    g.on_check_update(move || {
        local(|l| l.update_status = tr(&a, "Checking"));
        redraw(&a);
        let app = a.clone();
        let shared = shared(&a).inner().clone();
        tauri::async_runtime::spawn(async move {
            let out = crate::updater::check_update(app.clone(), shared).await;
            let _ = slint::invoke_from_event_loop(move || {
                match out {
                    Ok(Some(info)) => show_update(&app, &serde_json::to_value(info).unwrap_or_default(), true),
                    Ok(None) => local(|l| l.update_status = tr(&app, "Up to date")),
                    Err(e) => local(|l| l.update_status = sentence(&tr(&app, &e))),
                }
                redraw(&app);
            });
        });
    });
}

fn feedback_ready(w: &AppWindow) {
    let g = w.global::<SettingsPage>();
    let message = g.get_feedback_message();
    let count = message.chars().count();
    if count > 2000 {
        g.set_feedback_message(message.chars().take(2000).collect::<String>().into());
    }
    let contact = g.get_feedback_contact();
    if contact.chars().count() > 100 {
        g.set_feedback_contact(contact.chars().take(100).collect::<String>().into());
    }
    g.set_feedback_count(format!("{} / 2000", count.min(2000)).into());
    let waiting = local(|l| l.feedback_waiting);
    g.set_feedback_can_send(!waiting && !g.get_feedback_message().trim().is_empty());
}

fn pick_file() -> Option<std::path::PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::UI::Controls::Dialogs::{
        GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST, OPENFILENAMEW,
    };
    let filter: Vec<u16> = "Images and videos\0*.png;*.jpg;*.jpeg;*.gif;*.webp;*.mp4;*.webm;*.mov\0\0"
        .encode_utf16()
        .collect();
    let mut name = vec![0u16; 1024];
    let mut ofn: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    ofn.lStructSize = size_of::<OPENFILENAMEW>() as u32;
    ofn.lpstrFilter = filter.as_ptr();
    ofn.lpstrFile = name.as_mut_ptr();
    ofn.nMaxFile = name.len() as u32;
    ofn.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
    unsafe { windows_sys::Win32::System::Ole::OleInitialize(std::ptr::null_mut()) };
    if unsafe { GetOpenFileNameW(&mut ofn) } == 0 {
        return None;
    }
    let len = name.iter().position(|c| *c == 0).unwrap_or(name.len());
    Some(std::ffi::OsString::from_wide(&name[..len]).into())
}

fn bind_feedback(w: &AppWindow, app: &AppHandle) {
    let g = w.global::<SettingsPage>();
    g.on_open_feedback(move || {
        local(|l| l.feedback_status.clear());
        with_win(|w| {
            w.global::<SettingsPage>().set_sheet("feedback".into());
            feedback_ready(w);
        });
    });
    g.on_feedback_edited(move || with_win(feedback_ready));
    let a = app.clone();
    g.on_feedback_remove(move || {
        local(|l| {
            l.file = None;
            l.file_problem.clear();
        });
        redraw(&a);
    });
    let a = app.clone();
    g.on_feedback_pick(move || {
        later(
            &a,
            |_| {
                let path = pick_file()?;
                let name = path.file_name()?.to_string_lossy().to_string();
                let size = std::fs::metadata(&path).ok()?.len();
                let ext = name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
                if !ATTACHMENT_TYPES.contains(&ext.as_str()) {
                    return Some(Err((name, None)));
                }
                if size > ATTACHMENT_MOST {
                    return Some(Err((name, Some(size))));
                }
                Some(std::fs::read(&path).map(|data| (name.clone(), data)).map_err(|_| (name, None)))
            },
            |app, out| {
                let Some(out) = out else { return };
                local(|l| match out {
                    Ok(file) => {
                        l.file = Some(file);
                        l.file_problem.clear();
                    }
                    Err((name, Some(size))) => {
                        l.file = None;
                        l.file_problem = tf(app, "{0} is {1}. Files can be up to 16 MB.", &[&name, &file_size(size)]);
                    }
                    Err((name, None)) => {
                        l.file = None;
                        l.file_problem = tf(
                            app,
                            "{0} is not an image or video FoxDen can send. Use PNG, JPG, GIF, WebP, MP4, WebM or MOV.",
                            &[&name],
                        );
                    }
                });
            },
        );
    });
    let a = app.clone();
    g.on_feedback_send(move || {
        let hour = Duration::from_secs(3600);
        let busy = local(|l| {
            l.sent.retain(|t| t.elapsed() < hour);
            l.feedback_waiting || l.sent.len() >= FEEDBACK_HOUR
        });
        if busy {
            if !local(|l| l.feedback_waiting) {
                local(|l| l.feedback_status = tr(&a, "That is a lot of feedback for one hour. Try again later."));
                redraw(&a);
            }
            return;
        }
        let mut fields = None;
        with_win(|w| {
            let g = w.global::<SettingsPage>();
            let kind = FEEDBACK_TYPES[g.get_feedback_type().clamp(0, 3) as usize];
            fields = Some((kind, g.get_feedback_message().to_string(), g.get_feedback_contact().to_string()));
        });
        let Some((kind, message, contact)) = fields else { return };
        let file = local(|l| l.file.clone());
        local(|l| {
            l.feedback_waiting = true;
            l.feedback_status = match &file {
                Some((_, data)) => tf(&a, "Sending with {0} attached…", &[&file_size(data.len() as u64)]),
                None => tr(&a, "Sending…"),
            };
        });
        redraw(&a);
        let meta = json!({"type": kind, "message": message, "contact": contact, "file": file.as_ref().map(|f| f.0.clone())});
        let meta = serde_json::to_vec(&meta).unwrap_or_default();
        let mut body = (meta.len() as u32).to_le_bytes().to_vec();
        body.extend_from_slice(&meta);
        if let Some((_, data)) = &file {
            body.extend_from_slice(data);
        }
        let app = a.clone();
        tauri::async_runtime::spawn(async move {
            let request = tauri::ipc::Request::new(tauri::ipc::InvokeBody::Raw(body));
            let out = crate::report::send_feedback(request).await;
            let _ = slint::invoke_from_event_loop(move || {
                match out {
                    Ok(()) => {
                        local(|l| {
                            l.sent.push(Instant::now());
                            l.file = None;
                            l.feedback_status = tr(&app, "Sent. Thank you.");
                        });
                        with_win(|w| w.global::<SettingsPage>().set_feedback_message("".into()));
                        let a = app.clone();
                        slint::Timer::single_shot(FEEDBACK_WAIT, move || {
                            local(|l| l.feedback_waiting = false);
                            redraw(&a);
                        });
                    }
                    Err(e) => local(|l| {
                        l.feedback_waiting = false;
                        l.feedback_status = tr(&app, &e);
                    }),
                }
                redraw(&app);
            });
        });
    });
}

pub fn event(w: &AppWindow, app: &AppHandle, view: &mut View, name: &str) {
    let payload = view.events.get(name).cloned().unwrap_or_default();
    match name {
        "modules" => {
            let text = tr(app, payload["text"].as_str().unwrap_or_default());
            note(app, &text);
            local(|l| l.modules = Some((payload["ok"].as_bool().unwrap_or(false), text)));
        }
        "conflicts" => {
            let text = payload["text"].as_str().unwrap_or_default();
            if !text.is_empty() {
                note(app, text);
            }
            let line = closed_line(app, text, payload["at"].as_str().unwrap_or_default());
            if !line.is_empty() {
                local(|l| l.conflicts_last = line);
            }
            refresh_conflicts(app);
        }
        "update" => show_update(app, &payload, true),
        "update-progress" => {
            let mb = |n: f64| tf(app, "{0} MB", &[&format!("{:.1}", n / 1048576.0)]);
            let done = payload["done"].as_f64().unwrap_or(0.0);
            let text = match payload["total"].as_f64().filter(|t| *t > 0.0) {
                Some(total) => tf(app, "{0}% of {1}", &[&format!("{}", (done / total * 100.0).round()), &mb(total)]),
                None => mb(done),
            };
            local(|l| l.update_progress = text);
        }
        _ => return,
    }
    apply(w, app, view);
}

pub fn apply(w: &AppWindow, app: &AppHandle, view: &mut View) {
    let g = w.global::<SettingsPage>();
    let snap = &view.snap;
    let settings = &snap["settings"];
    let choice = settings["language"].as_str().unwrap_or("auto");
    let lang = crate::i18n::language(choice);
    super::logs::set_lang(app, &lang);
    w.global::<super::I18n>().set_lang(lang.into());
    let mut langs: Vec<SharedString> = vec![tr(app, "Automatic").into()];
    langs.extend(LANGS.iter().map(|l| SharedString::from(l.1)));
    fill(g.get_languages(), langs, |m| g.set_languages(m));
    g.set_language_index(LANGS.iter().position(|l| l.0 == choice).map_or(0, |i| i as i32 + 1));
    let flag = |key: &str, default: bool| settings[key].as_bool().unwrap_or(default);
    g.set_close_to_tray(flag("close_to_tray", true));
    g.set_start_minimized(flag("start_minimized", false));
    g.set_lighting(flag("lighting_module", true));
    g.set_fans(flag("fans_module", true));
    g.set_battery_tray(flag("battery_tray", true));
    g.set_battery_alerts(flag("battery_alerts", true));
    g.set_native_drivers(flag("native_drivers", true));
    g.set_net_enabled(flag("network_enabled", false));
    g.set_net_discover(settings["network"]["discover"].as_bool().unwrap_or(false));
    g.set_server_enabled(flag("openrgb_enabled", false));
    g.set_autostart(flag("autostart", false));
    g.set_auto_update(flag("auto_update", false));
    g.set_close_conflicts(flag("close_conflicts", false));
    g.set_send_reports(flag("send_reports", false));
    g.set_ai_access(flag("ai_access", false));
    g.set_version(s(&snap["version"]));

    let fps = settings["fps"].as_i64().unwrap_or(30);
    g.set_fps(fps as i32);
    g.set_host(s(&settings["openrgb_host"]));
    g.set_port(value_text(&settings["openrgb_port"]).into());

    if !local(|l| l.update.is_some())
        && let Some(info) = view.events.get("update").cloned()
    {
        show_update(app, &info, false);
    }

    local(|l| {
        g.set_fps_problem(l.fps_problem.as_str().into());
        if l.fps_problem.is_empty() {
            g.set_fps_text(fps.to_string().into());
        }
        g.set_host_problem(l.host_problem.as_str().into());
        g.set_port_problem(l.port_problem.as_str().into());
        let (ok, text) = l.modules.clone().unwrap_or((true, String::new()));
        g.set_modules_ok(ok);
        g.set_modules_outcome(text.into());

        let v = &l.mixer["vmic"];
        g.set_vmic_on(l.mixer["config"]["virtual_mic"].as_bool().unwrap_or(false));
        let state = if !v["installed"].as_bool().unwrap_or(false) {
            "Not installed"
        } else if !v["ready"].as_bool().unwrap_or(false) {
            "Restart Windows to finish"
        } else if v["ours"].as_bool().unwrap_or(false) {
            "Installed by FoxDen"
        } else {
            "Installed"
        };
        g.set_vmic_state(if l.mixer.is_null() { SharedString::new() } else { tr(app, state).into() });
        g.set_vmic_ours(v["ours"].as_bool().unwrap_or(false));
        g.set_vmic_busy(l.vmic_busy);
        g.set_vmic_outcome(l.vmic_outcome.as_str().into());
        g.set_vmic_progress(l.vmic_progress.as_str().into());

        g.set_update_status(l.update_status.as_str().into());
        if let Some(info) = &l.update {
            let version = s(&info["version"]).to_string();
            g.set_update_available(true);
            g.set_update_version(version.as_str().into());
            g.set_update_label(tf(app, "Update to {0}", &[&version]).into());
            g.set_update_title(tf(app, "FoxDen {0}", &[&version]).into());
            let notes = info["notes"].as_str().filter(|n| !n.is_empty()).unwrap_or("No release notes.");
            g.set_update_notes(tr(app, notes).into());
            let mut log = log_rows(app, info["changes"].as_array().map_or(&[][..], Vec::as_slice));
            if log.is_empty() {
                log.push(ChangeRow { kind: "item".into(), text: tr(app, notes).into() });
            }
            fill(g.get_update_log(), log, |m| g.set_update_log(m));
        }
        g.set_update_progress(l.update_progress.as_str().into());
        g.set_update_busy(l.update_busy);

        let file = match &l.file {
            Some((name, data)) => format!("{name}, {}", file_size(data.len() as u64)),
            None => tr(app, "Optional. PNG, JPG, GIF, WebP, MP4, WebM or MOV, up to 16 MB."),
        };
        g.set_feedback_file(file.into());
        g.set_feedback_has_file(l.file.is_some());
        g.set_feedback_file_problem(l.file_problem.as_str().into());
        g.set_feedback_status(l.feedback_status.as_str().into());
    });
    feedback_ready(w);

    conflicts(w, app);
    network(w, app, settings);
    strips(w, app, settings);
    backends(w, app, snap);
}

fn conflicts(w: &AppWindow, app: &AppHandle) {
    let g = w.global::<SettingsPage>();
    let (rows, last, empty) = local(|l| {
        let mut rows = Vec::new();
        let mut exes = Vec::new();
        for c in &l.conflicts {
            let name = s(&c["name"]).to_string();
            let kept = c["kept_for"].as_str();
            let (mark, state) = if let Some(k) = kept {
                ("on", tf(app, "Kept running for {0}", &[k]))
            } else if c["running"] == json!(false) {
                ("off", tr(app, "Not running"))
            } else if c["closes"].as_bool().unwrap_or(false) {
                ("warn", tr(app, "Running"))
            } else {
                ("off", tr(app, "Left alone"))
            };
            rows.push(StateRow {
                name: name.into(),
                mark: mark.into(),
                state: state.into(),
                ..Default::default()
            });
            exes.push(None);
            for e in c["exes"].as_array().into_iter().flatten() {
                let exe = s(&e["name"]).to_string();
                rows.push(StateRow {
                    name: tf(app, "Close {0}", &[&exe]).into(),
                    exe: true,
                    closes: e["closes"].as_bool().unwrap_or(false),
                    kept: kept.is_some(),
                    ..Default::default()
                });
                exes.push(Some(exe));
            }
        }
        l.conflict_exes = exes;
        (rows, l.conflicts_last.clone(), l.conflicts.is_empty() && l.conflicts_asked > 0)
    });
    fill(g.get_conflicts(), rows, |m| g.set_conflicts(m));
    g.set_conflicts_last(last.into());
    g.set_conflicts_empty(empty);
}

fn network(w: &AppWindow, app: &AppHandle, settings: &Value) {
    let g = w.global::<SettingsPage>();
    let net = &settings["network"];
    let names: Vec<SharedString> = kinds().iter().map(|k| tr(app, k.name).into()).collect();
    fill(g.get_net_kinds(), names, |m| g.set_net_kinds(m));
    let (status, searching, pairing, kind_index, found) =
        local(|l| (l.net_status.clone(), l.net_searching, l.pairing.clone(), l.net_kind, l.net_found.clone()));
    g.set_net_status(status.into());
    g.set_net_searching(searching);
    g.set_net_pairing(!pairing.is_empty());
    g.set_net_kind(kind_index as i32);

    let paired = |item: &Value| {
        [&item["username"], &item["auth_token"]]
            .iter()
            .any(|v| v.as_str().is_some_and(|s| !s.is_empty()))
    };
    let pair_label = |family: &str, ip: &str, is_paired: bool| {
        tr(
            app,
            if pairing == format!("{family}{ip}") {
                "Waiting"
            } else if is_paired {
                "Pair again"
            } else {
                "Pair"
            },
        )
    };
    let found_rows: Vec<NetFound> = found
        .iter()
        .filter_map(|f| {
            let family = f["family"].as_str()?;
            let spec = kind(family)?;
            let ip = f["ip"].as_str().unwrap_or_default();
            let added = net[family].as_array().and_then(|l| l.iter().find(|d| d["ip"] == f["ip"]));
            let (action, label) = if spec.pair.is_some() {
                ("pair", pair_label(family, ip, added.is_some_and(paired)))
            } else if added.is_some() {
                ("added", String::new())
            } else {
                ("add", tr(app, "Add"))
            };
            let title = f["name"].as_str().filter(|n| !n.is_empty()).map_or_else(|| tr(app, spec.name), str::to_string);
            Some(NetFound {
                title: title.into(),
                ip: ip.into(),
                action: action.into(),
                label: label.into(),
            })
        })
        .collect();
    fill(g.get_net_found(), found_rows, |m| g.set_net_found(m));

    let key = serde_json::to_string(&(net, &pairing, &found, crate::i18n::language(settings["language"].as_str().unwrap_or("auto")))).unwrap_or_default();
    if local(|l| l.net_key == key) {
        return;
    }
    let mut map = Vec::new();
    let mut cards = Vec::new();
    for spec in kinds() {
        for (idx, item) in net[spec.key].as_array().into_iter().flatten().enumerate() {
            map.push((spec.key, idx));
            let fields: Vec<NetField> = spec
                .fields
                .iter()
                .map(|&(key, label, ty)| {
                    let mut f = NetField {
                        label: tr(app, label).into(),
                        kind: "text".into(),
                        text: value_text(&item[key]).into(),
                        ..Default::default()
                    };
                    match ty {
                        Ty::Check => {
                            f.kind = "check".into();
                            f.on = item[key].as_bool().unwrap_or(false);
                        }
                        Ty::Choice(options) => {
                            f.kind = "choice".into();
                            f.options = ModelRc::new(VecModel::from(
                                options.iter().map(|o| SharedString::from(tr(app, o))).collect::<Vec<_>>(),
                            ));
                            f.index = options.iter().position(|o| item[key] == json!(o)).unwrap_or(0) as i32;
                        }
                        _ => {}
                    }
                    f
                })
                .collect();
            let title = ["name", "ip", "port"]
                .iter()
                .map(|k| value_text(&item[*k]))
                .find(|t| !t.is_empty() && t != "0")
                .unwrap_or_else(|| tr(app, "New"));
            let is_paired = paired(item);
            cards.push(NetCard {
                title: format!("{}: {title}", tr(app, spec.name)).into(),
                pairable: spec.pair.is_some(),
                paired: is_paired,
                pair_label: pair_label(spec.key, item["ip"].as_str().unwrap_or_default(), is_paired).into(),
                fields: ModelRc::new(VecModel::from(fields)),
            });
        }
    }
    local(|l| {
        l.net_key = key;
        l.net_cards = map;
        l.net_problems.clear();
    });
    g.set_net_cards(ModelRc::new(VecModel::from(cards)));
    net_problems(w);
}

fn strips(w: &AppWindow, app: &AppHandle, settings: &Value) {
    let g = w.global::<SettingsPage>();
    let protocols: Vec<SharedString> = PROTOCOLS.iter().map(|p| tr(app, p.1).into()).collect();
    fill(g.get_protocols(), protocols, |m| g.set_protocols(m));
    let list = &settings["led_strips"];
    let key = serde_json::to_string(list).unwrap_or_default();
    if local(|l| l.strip_key == key) {
        return;
    }
    let rows: Vec<Strip> = list
        .as_array()
        .into_iter()
        .flatten()
        .map(|item| {
            let name = value_text(&item["name"]);
            let port = value_text(&item["port"]);
            Strip {
                title: if name.is_empty() { tr(app, "LED strip") } else { name.clone() }.into(),
                meta: if port.is_empty() { tr(app, "No port") } else { port.clone() }.into(),
                name: name.into(),
                port: port.into(),
                protocol: PROTOCOLS
                    .iter()
                    .position(|p| item["protocol"] == json!(p.0))
                    .unwrap_or(0) as i32,
                baud: value_text(&item["baud"]).into(),
                leds: value_text(&item["leds"]).into(),
            }
        })
        .collect();
    local(|l| {
        l.strip_key = key;
        l.strip_text.clear();
        l.strip_protocol.clear();
        l.strip_problems.clear();
    });
    fill(g.get_strips(), rows, |m| g.set_strips(m));
    strip_problems(w);
}

fn backends(w: &AppWindow, app: &AppHandle, snap: &Value) {
    let g = w.global::<SettingsPage>();
    let rows: Vec<StateRow> = snap["backends"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|b| {
            let id = b["name"].as_str().unwrap_or_default();
            let name = BACKENDS.iter().find(|n| n.0 == id).map_or(id, |n| n.1);
            let devices = b["devices"].as_u64().unwrap_or(0) as usize;
            let (mark, state, problem) = match b["error"].as_str() {
                Some(e) => ("warn", tr(app, "Error"), sentence(&tr(app, e))),
                None if !b["active"].as_bool().unwrap_or(false) => ("off", tr(app, "Off"), String::new()),
                None if devices > 0 => ("on", tr(app, &plural(devices, "device")), String::new()),
                None => ("on", tr(app, "On"), String::new()),
            };
            StateRow {
                name: tr(app, name).into(),
                mark: mark.into(),
                state: state.into(),
                problem: problem.into(),
                ..Default::default()
            }
        })
        .collect();
    fill(g.get_backends(), rows, |m| g.set_backends(m));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_addresses() {
        assert!(local_ip("192.168.1.20"));
        assert!(local_ip("10.0.0.1"));
        assert!(local_ip("172.16.0.1"));
        assert!(!local_ip("172.32.0.1"));
        assert!(!local_ip("8.8.8.8"));
        assert!(local_ip("::1"));
        assert!(local_ip("fd12:3456::1"));
        assert!(local_ip("fe80::1"));
        assert!(!local_ip("2001:db8::1"));
    }
}
