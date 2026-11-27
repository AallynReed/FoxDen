//! The pipe AI assistants reach FoxDen through, via foxden-mcp.exe, while the user allows it.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use foxden_core::{
    Color, Effect, Layer, LayerEffect, ParamInfo, Post, Scope, Spot, When, clean_key_map, from_hex, key_layout,
    spots, to_hex, zoned,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tauri::{AppHandle, Emitter, Manager, State};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_PIPE_CONNECTED, GetLastError, HANDLE, INVALID_HANDLE_VALUE, LocalFree,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
use windows_sys::Win32::System::IO::CancelSynchronousIo;
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use crate::fan_proposal::{self, DESK, Hooks, await_answer};
use crate::mixer::Mixer;
use crate::config::Settings;
use crate::key_maps::{self, KeyMap};
use crate::state::{Jobs, Shared, lock, update};
use crate::ui::effects_keys::key_name;
use crate::{apps, battery, commands, fans, log, mixer};

const MAX_LINE: usize = 64 * 1024;
const MAX_CLIENTS: usize = 4;
/// A client that sends no complete request for this long is dropped.
const IDLE: Duration = if cfg!(test) { Duration::from_secs(1) } else { Duration::from_secs(10) };
const TURNED_OFF: &str = "Turn on AI assistant access in FoxDen's Settings.";

static SERVER: Mutex<Option<Server>> = Mutex::new(None);

/// Opens or closes the pipe to follow the setting.
pub fn set_enabled(app: &AppHandle, on: bool) {
    let mut slot = lock(&SERVER);
    if on == slot.is_some() {
        return;
    }
    if let Some(server) = slot.take() {
        DESK.cancel();
        server.stop();
        log::info("ai", "AI assistant access is off");
        return;
    }
    let a = app.clone();
    let handler: Handler = Arc::new(move |line| handle(&a, line));
    match pipe_name().and_then(|name| Server::start(name, handler)) {
        Ok(server) => {
            *slot = Some(server);
            log::info("ai", "AI assistant access is on");
        }
        Err(e) => log::warn("ai", e),
    }
}

/// `\\.\pipe\foxden-app-<SID>`, which foxden-mcp.exe derives the same way.
pub fn pipe_name() -> Result<String, String> {
    Ok(format!(r"\\.\pipe\foxden-app-{}", user_sid()?))
}

fn user_sid() -> Result<String, String> {
    let failed = || format!("cannot read this account's id: {}", std::io::Error::last_os_error());
    unsafe {
        let mut token: HANDLE = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(failed());
        }
        let mut len = 0;
        GetTokenInformation(token, TokenUser, null_mut(), 0, &mut len);
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        let ok = GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), len, &mut len);
        CloseHandle(token);
        if ok == 0 {
            return Err(failed());
        }
        let sid = (*buf.as_ptr().cast::<TOKEN_USER>()).User.Sid;
        let mut text = null_mut();
        if ConvertSidToStringSidW(sid, &mut text) == 0 {
            return Err(failed());
        }
        let mut n = 0;
        while *text.add(n) != 0 {
            n += 1;
        }
        let s = String::from_utf16_lossy(std::slice::from_raw_parts(text, n));
        LocalFree(text.cast());
        Ok(s)
    }
}

// ---- The pipe server

type Handler = Arc<dyn Fn(&str) -> String + Send + Sync>;
type Idle = Arc<Mutex<Option<Instant>>>;

struct Conn {
    thread: JoinHandle<()>,
    idle: Idle,
}

struct Server {
    name: String,
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

impl Server {
    fn start(name: String, handler: Handler) -> Result<Server, String> {
        // Clients of a server just stopped may still hold the name for a moment.
        let mut first = create(&name, true);
        for _ in 0..10 {
            if first.is_ok() {
                break;
            }
            thread::sleep(Duration::from_millis(200));
            first = create(&name, true);
        }
        let first = first?;
        let stop = Arc::new(AtomicBool::new(false));
        let (n, s) = (name.clone(), stop.clone());
        let thread = thread::Builder::new()
            .name("foxden-ai-pipe".into())
            .spawn(move || accept(&n, first, &s, &handler))
            .map_err(|e| e.to_string())?;
        Ok(Server { name, stop, thread })
    }

    fn stop(self) {
        self.stop.store(true, Ordering::SeqCst);
        wake(&self.name);
        let _ = self.thread.join();
    }
}

/// Only this user may open or add to the pipe, and it is named as the owner even when elevated.
fn create(name: &str, first: bool) -> Result<File, String> {
    let sid = user_sid()?;
    let wide = |text: &str| -> Vec<u16> { text.encode_utf16().chain(Some(0)).collect() };
    let (name_w, access_w) = (wide(name), wide(&format!("O:{sid}D:P(A;;FA;;;{sid})")));
    let mut descriptor = null_mut();
    let converted = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(access_w.as_ptr(), SDDL_REVISION_1, &mut descriptor, null_mut())
    };
    if converted == 0 {
        return Err(format!("cannot build the access list of the AI pipe: {}", std::io::Error::last_os_error()));
    }
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    let first_flag = if first { FILE_FLAG_FIRST_PIPE_INSTANCE } else { 0 };
    let handle = unsafe {
        CreateNamedPipeW(
            name_w.as_ptr(),
            PIPE_ACCESS_DUPLEX | first_flag,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            MAX_LINE as u32,
            MAX_LINE as u32,
            0,
            &attributes,
        )
    };
    let e = std::io::Error::last_os_error();
    unsafe { LocalFree(descriptor) };
    if handle == INVALID_HANDLE_VALUE {
        return Err(format!("cannot open the AI pipe: {e}"));
    }
    Ok(unsafe { File::from_raw_handle(handle as _) })
}

fn connect(pipe: &File) -> bool {
    let ok = unsafe { ConnectNamedPipe(pipe.as_raw_handle() as _, null_mut()) };
    ok != 0 || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED
}

/// Unblocks `accept` by connecting to it.
fn wake(name: &str) {
    for _ in 0..100 {
        match OpenOptions::new().read(true).write(true).open(name) {
            Err(e) if e.raw_os_error() == Some(231) => thread::sleep(Duration::from_millis(20)),
            Err(e) if e.kind() == ErrorKind::NotFound => return,
            _ => return,
        }
    }
}

fn accept(name: &str, first: File, stop: &Arc<AtomicBool>, handler: &Handler) {
    let conns: Arc<Mutex<Vec<Conn>>> = Arc::default();
    let reaper = {
        let (conns, stop) = (conns.clone(), stop.clone());
        thread::Builder::new().name("foxden-ai-reap".into()).spawn(move || reap(&conns, &stop))
    };
    let mut listener = first;
    while !stop.load(Ordering::SeqCst) {
        let connected = connect(&listener);
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let next = match create(name, false) {
            Ok(next) => next,
            Err(e) => {
                log::warn("ai", e);
                thread::sleep(Duration::from_secs(1));
                continue;
            }
        };
        let pipe = std::mem::replace(&mut listener, next);
        if !connected {
            continue;
        }
        let mut list = lock(&conns);
        list.retain(|c| !c.thread.is_finished());
        if list.len() >= MAX_CLIENTS {
            continue;
        }
        let idle: Idle = Arc::new(Mutex::new(Some(Instant::now())));
        let (mine, stop, handler) = (idle.clone(), stop.clone(), handler.clone());
        let spawned = thread::Builder::new()
            .name("foxden-ai-client".into())
            .spawn(move || serve_client(&pipe, &mine, &stop, &handler));
        if let Ok(thread) = spawned {
            list.push(Conn { thread, idle });
        }
    }
    drop(listener);
    if let Ok(reaper) = reaper {
        let _ = reaper.join();
    }
}

/// Ends the reads of clients idle too long, and of every client once the server stops.
fn reap(conns: &Mutex<Vec<Conn>>, stop: &AtomicBool) {
    loop {
        thread::sleep(Duration::from_millis(200));
        let stopping = stop.load(Ordering::SeqCst);
        for conn in lock(conns).iter() {
            let idle = lock(&conn.idle);
            let waiting = idle.is_some_and(|since| stopping || since.elapsed() > IDLE);
            if waiting && !conn.thread.is_finished() {
                unsafe { CancelSynchronousIo(conn.thread.as_raw_handle() as _) };
            }
        }
        if stopping {
            return;
        }
    }
}

fn serve_client(pipe: &File, idle: &Mutex<Option<Instant>>, stop: &AtomicBool, handler: &Handler) {
    let mut reader = BufReader::new(pipe);
    let mut writer = pipe;
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.by_ref().take(MAX_LINE as u64 + 1).read_until(b'\n', &mut buf) {
            Ok(n) if n > 0 => {}
            _ => return,
        }
        *lock(idle) = None;
        if stop.load(Ordering::SeqCst) {
            return;
        }
        let complete = buf.ends_with(b"\n");
        let mut reply = if complete {
            handler(String::from_utf8_lossy(&buf).trim_end())
        } else {
            json!({"error": "The request is too long."}).to_string()
        };
        reply.push('\n');
        *lock(idle) = Some(Instant::now());
        if writer.write_all(reply.as_bytes()).is_err() || !complete {
            return;
        }
    }
}

// ---- Requests

#[derive(Deserialize)]
struct Request {
    tool: String,
    #[serde(default)]
    args: Value,
    #[serde(default)]
    client: String,
}

type Reply = Result<Value, String>;

/// Answers one request line with `{"ok": ..}` or `{"error": ".."}`.
pub fn handle(app: &AppHandle, line: &str) -> String {
    let reply = match serde_json::from_str::<Request>(line) {
        Err(_) => Err("FoxDen could not read the request.".into()),
        Ok(_) if !allowed(app) => Err(TURNED_OFF.into()),
        Ok(r) => {
            log::info("ai", format!("AI assistant used {}", r.tool));
            call(app, &r.tool, &r.args, &r.client)
        }
    };
    match reply {
        Ok(v) => json!({ "ok": v }),
        Err(e) => json!({ "error": e }),
    }
    .to_string()
}

fn allowed(app: &AppHandle) -> bool {
    app.try_state::<Shared>()
        .is_some_and(|s| lock(&s).config.settings.ai_access)
}

fn shared(app: &AppHandle) -> Result<State<'static, Shared>, String> {
    app.try_state::<Shared>().ok_or_else(|| "FoxDen is still starting.".into())
}

fn call(app: &AppHandle, tool: &str, args: &Value, client: &str) -> Reply {
    let done = |r: Result<(), String>, text: String| r.map(|_| json!(text));
    match tool {
        "get_status" => status(app),
        "list_effects" => Ok(effects()),
        "set_effect" => set_effect(app, args),
        "set_brightness" => {
            let v = number(args, "value")?;
            if !(0.0..=100.0).contains(&v) {
                return Err("value must be from 0 to 100.".into());
            }
            let r = commands::set_brightness(app.clone(), shared(app)?, (v / 100.0) as f32);
            done(r, format!("Brightness set to {}%.", v.round()))
        }
        "pause" => done(commands::set_paused(app.clone(), shared(app)?, true), "Lighting paused.".into()),
        "resume" => done(commands::set_paused(app.clone(), shared(app)?, false), "Lighting resumed.".into()),
        "list_profiles" => {
            let s = shared(app)?;
            let state = lock(&s);
            let names: Vec<&String> = state.config.profiles.keys().collect();
            Ok(json!({"profiles": names, "active": state.config.active_profile}))
        }
        "apply_profile" => {
            let name = text(args, "name")?;
            done(commands::load_profile(app.clone(), shared(app)?, name.clone()), format!("Applied profile {name}."))
        }
        "save_profile" => save_profile(app, args),
        "get_profile" => get_profile(app, args),
        "delete_profile" => {
            let name = text(args, "name")?;
            done(commands::delete_profile(app.clone(), shared(app)?, name.clone()), format!("Deleted profile {name}."))
        }
        "rename_profile" => rename_profile(app, args),
        "list_layers" => Ok(json!({"layers": layers_view(&lock(&*shared(app)?).engine.layers)})),
        "add_layer" => add_layer(app, args),
        "update_layer" => update_layer(app, args),
        "remove_layer" => edit_layers(app, |layers| {
            let i = index(args, layers.len())?;
            layers.remove(i);
            Ok(())
        }),
        "move_layer" => edit_layers(app, |layers| {
            let i = index(args, layers.len())?;
            let to = number(args, "to")?;
            if to < 0.0 || to as usize >= layers.len() || to.fract() != 0.0 {
                return Err(format!("to must be a layer index from 0 to {}.", layers.len().saturating_sub(1)));
            }
            let layer = layers.remove(i);
            layers.insert(to as usize, layer);
            Ok(())
        }),
        "toggle_layer" => edit_layers(app, |layers| {
            let i = index(args, layers.len())?;
            let on = args["enabled"].as_bool().unwrap_or(!layers[i].enabled);
            layers[i].enabled = on;
            Ok(())
        }),
        "clear_device_effect" => {
            let id = device_id(&snapshot(app)?, &text(args, "device")?)?;
            done(commands::set_override(app.clone(), shared(app)?, id.clone(), None), format!("{id} follows the main effect again."))
        }
        "set_device" => set_device(app, args),
        "rescan_devices" => {
            let jobs = app.try_state::<Jobs>().ok_or("FoxDen is still starting.")?;
            done(commands::rescan(jobs), "Looking for devices again.".into())
        }
        "list_keys" => Ok(list_keys()),
        "get_key_map" => key_map_of(app, args),
        "list_key_maps" => {
            let s = shared(app)?;
            let maps: Vec<Value> = lock(&s)
                .config
                .key_maps
                .iter()
                .map(|m| json!({"name": m.name, "keys": readable_map(&m.map)}))
                .collect();
            Ok(json!({"key_maps": maps}))
        }
        "save_key_map" => save_key_map(app, args),
        "delete_key_map" => {
            let name = text(args, "name")?;
            done(key_maps::delete_key_map(app.clone(), shared(app)?, name.clone()), format!("Deleted key map {name}."))
        }
        "list_macros" | "save_macro" | "delete_macro" | "play_macro" | "stop_macros" | "set_macros_enabled" => {
            macro_tool(app, tool, args, client)
        }
        "list_apps" => Ok(json!({"apps": apps::windowed()})),
        "get_settings" => get_settings(app),
        "set_settings" => set_settings(app, args, client),
        "get_sensors" => sensors(),
        "get_audio" => audio(app),
        "set_channel_volume" => {
            let (id, label) = channel(args)?;
            let v = number(args, "volume")?;
            if !(0.0..=100.0).contains(&v) {
                return Err("volume must be from 0 to 100.".into());
            }
            mix(app, json!({"kind": "volume", "channel": id, "volume": v / 100.0}))?;
            Ok(json!(format!("{label} volume set to {}%.", v.round())))
        }
        "set_channel_mute" => {
            let (id, label) = channel(args)?;
            let muted = args["muted"].as_bool().ok_or("muted must be true or false.")?;
            mix(app, json!({"kind": "mute", "channel": id, "muted": muted}))?;
            Ok(json!(format!("{label} {}.", if muted { "muted" } else { "unmuted" })))
        }
        "assign_app" => assign_app(app, args),
        "get_batteries" => Ok(json!({"batteries": battery::get_batteries()})),
        "get_fan_config" => Ok(fan_proposal::view(&fans::get_fans()?, DESK.last())),
        "propose_fan_curves" => propose(app, args, client),
        _ => Err(format!("FoxDen has no tool named {tool}.")),
    }
}

fn number(args: &Value, key: &str) -> Result<f64, String> {
    args[key]
        .as_f64()
        .filter(|v| v.is_finite())
        .ok_or_else(|| format!("{key} must be a number."))
}

fn text(args: &Value, key: &str) -> Result<String, String> {
    match args[key].as_str().map(str::trim) {
        Some(s) if !s.is_empty() => Ok(s.to_string()),
        _ => Err(format!("{key} must be a non-empty string.")),
    }
}

fn percent(v: &Value) -> Value {
    v.as_f64().map_or(Value::Null, |v| json!((v * 100.0).round()))
}

// ---- Lighting

fn snapshot(app: &AppHandle) -> Result<Value, String> {
    let s = shared(app)?;
    let snap = lock(&s).snapshot();
    serde_json::to_value(snap).map_err(|e| e.to_string())
}

fn status(app: &AppHandle) -> Reply {
    let snap = snapshot(app)?;
    let devices: Vec<Value> = snap["devices"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|d| {
            let own = d["hardware_setting"].is_object();
            let mut out = json!({
                "id": d["id"], "name": d["name"], "kind": d["kind"], "backend": d["backend"],
                "leds": d["leds"].as_array().map_or(0, Vec::len),
                "enabled": d["enabled"], "brightness": percent(&d["brightness"]),
                "layout": d["rect"],
                "driven_by": if own { "hardware" } else { "foxden" },
            });
            if let Some(effect) = snap["overrides"].get(d["id"].as_str().unwrap_or_default()) {
                out["own_effect"] = effect.clone();
            }
            if own {
                out["hardware_mode"] = d["hardware_setting"].clone();
            }
            if d["zones"].as_array().is_some_and(|z| !z.is_empty()) {
                out["zones"] = d["zones"].clone();
            }
            out
        })
        .collect();
    let layers: Vec<Layer> = serde_json::from_value(snap["layers"].clone()).unwrap_or_default();
    let layers: Vec<Value> = layers_view(&layers)
        .into_iter()
        .map(|l| json!({"index": l["index"], "kind": l["effect"]["kind"], "enabled": l["enabled"], "scope": l["scope"], "when": l["when"]}))
        .collect();
    let key_maps: Vec<&Value> = snap["key_maps"].as_array().into_iter().flatten().map(|m| &m["name"]).collect();
    let mut effect = snap["effect"].clone();
    tidy(&mut effect);
    let mut devices = devices;
    devices.iter_mut().for_each(tidy);
    Ok(json!({
        "version": snap["version"],
        "effect": effect,
        "layers": layers,
        "layers_note": "Layers are drawn over the main effect in order, the last on top; list_layers shows them in full.",
        "key_maps": key_maps,
        "brightness": percent(&snap["brightness"]),
        "paused": snap["paused"],
        "active_profile": snap["active_profile"],
        "devices": devices,
    }))
}

fn kind_of(effect: &Effect) -> String {
    serde_json::to_value(effect)
        .ok()
        .and_then(|v| v["kind"].as_str().map(str::to_string))
        .unwrap_or_default()
}

fn params_view(params: &[ParamInfo]) -> Vec<Value> {
    params
        .iter()
        .map(|p| {
            let mut out = serde_json::to_value(&p.control).unwrap_or_default();
            out["name"] = json!(p.name);
            out["label"] = json!(p.label);
            match out["type"].as_str() {
                Some("color") => out["format"] = json!("#rrggbb or [r,g,b]"),
                Some("keys") => out["format"] = json!("list of key names from list_keys, such as [\"Q\", \"W\", \"F1\"]"),
                Some("key_colours") => out["format"] = json!("object of key name to colour, such as {\"Q\": \"#ff0000\"}"),
                _ => {}
            }
            out
        })
        .collect()
}

fn effects() -> Value {
    let list: Vec<Value> = Effect::catalog()
        .into_iter()
        .map(|info| {
            json!({"kind": kind_of(&info.default), "label": info.label, "params": params_view(&info.params), "default": info.default})
        })
        .collect();
    let layer: Vec<Value> = Post::catalog()
        .into_iter()
        .filter(|info| info.default != Post::None)
        .map(|info| {
            let mut default = serde_json::to_value(&info.default).unwrap_or_default();
            readable_effect(&mut default);
            json!({"kind": post_kind(&info.default), "label": info.label, "params": params_view(&info.params), "default": default})
        })
        .collect();
    json!({
        "effects": list,
        "layer_effects": layer,
        "note": "A layer takes a layer effect or any of effects, which then fills the layer.",
    })
}

/// The effect `kind` with `params` laid over the current one of that kind, or over its default.
fn build_effect(current: Option<&Value>, kind: &str, params: &Map<String, Value>) -> Result<Effect, String> {
    let catalog = Effect::catalog();
    let Some(info) = catalog.iter().find(|i| kind_of(&i.default) == kind) else {
        let kinds: Vec<String> = catalog.iter().map(|i| kind_of(&i.default)).collect();
        return Err(format!("Unknown effect kind \"{kind}\". Kinds: {}.", kinds.join(", ")));
    };
    let effect = fill(current, kind, &info.default, &info.params, params)?;
    serde_json::from_value(effect).map_err(|e| format!("Those parameters don't fit {kind}: {e}"))
}

/// `default`, or `current` when it is of this kind, with `params` checked and laid over it.
fn fill(
    current: Option<&Value>,
    kind: &str,
    default: &impl serde::Serialize,
    info: &[ParamInfo],
    params: &Map<String, Value>,
) -> Result<Value, String> {
    let mut effect = match current {
        Some(c) if c["kind"] == kind => c.clone(),
        _ => serde_json::to_value(default).map_err(|e| e.to_string())?,
    };
    for (name, v) in params {
        if name == "kind" {
            continue;
        }
        let Some(p) = info.iter().find(|p| p.name == name) else {
            let names: Vec<&str> = info.iter().map(|p| p.name).collect();
            return Err(format!("{kind} has no parameter \"{name}\". It takes: {}.", names.join(", ")));
        };
        let control = serde_json::to_value(&p.control).unwrap_or_default();
        effect[name.as_str()] = param(&control, name, v)?;
    }
    Ok(effect)
}

fn post_kind(p: &Post) -> String {
    serde_json::to_value(p).ok().and_then(|v| v["kind"].as_str().map(str::to_string)).unwrap_or_default()
}

/// A layer's effect: a layer effect from `list_effects`' `layer_effects`, or any main effect filling the layer.
fn build_layer_effect(current: Option<&Value>, kind: &str, params: &Map<String, Value>) -> Result<LayerEffect, String> {
    let posts = Post::catalog();
    if let Some(info) = posts.iter().find(|i| kind != "none" && post_kind(&i.default) == kind) {
        let v = fill(current, kind, &info.default, &info.params, params)?;
        return serde_json::from_value(v)
            .map(LayerEffect::Post)
            .map_err(|e| format!("Those parameters don't fit {kind}: {e}"));
    }
    if kind == "none" {
        return Err("A layer needs an effect; remove_layer takes a layer away.".into());
    }
    build_effect(current, kind, params).map(LayerEffect::Fill).map_err(|e| {
        let kinds: Vec<String> = posts.iter().map(|i| post_kind(&i.default)).filter(|k| k != "none").collect();
        format!("{e} Layer effects: {}.", kinds.join(", "))
    })
}

fn param(control: &Value, name: &str, v: &Value) -> Result<Value, String> {
    match control["type"].as_str().unwrap_or_default() {
        "range" => {
            let x = v.as_f64().filter(|x| x.is_finite()).ok_or(format!("{name} must be a number."))?;
            let (lo, hi) = (control["min"].as_f64().unwrap_or(x), control["max"].as_f64().unwrap_or(x));
            Ok(json!(x.clamp(lo, hi)))
        }
        "color" => color(name, v),
        "toggle" => v.as_bool().map(Value::Bool).ok_or(format!("{name} must be true or false.")),
        "choice" => {
            let options = control["options"].as_array().cloned().unwrap_or_default();
            if v.is_string() && options.contains(v) {
                Ok(v.clone())
            } else {
                Err(format!("{name} must be one of {}.", Value::Array(options)))
            }
        }
        "sensor" | "battery" => v.as_str().map(|s| json!(s)).ok_or(format!("{name} must be a string.")),
        "keys" => {
            let keys = v.as_array().ok_or(format!("{name} must be a list of key names from list_keys."))?;
            let mut codes = keys.iter().map(key_code).collect::<Result<Vec<u32>, String>>()?;
            codes.sort_unstable();
            codes.dedup();
            Ok(json!(codes))
        }
        "key_colours" => Ok(json!(key_colours(v)?)),
        "stops" => {
            let max = control["max"].as_f64().unwrap_or(100.0);
            let stops = v.as_array().ok_or(format!("{name} must be a list of {{temp, color}}."))?;
            stops
                .iter()
                .map(|s| {
                    let at = s["temp"].as_f64().filter(|t| t.is_finite()).ok_or(format!("Each of {name} needs a number temp."))?;
                    Ok(json!({"temp": at.clamp(0.0, max), "color": color(name, &s["color"])?}))
                })
                .collect::<Result<Vec<Value>, String>>()
                .map(Value::Array)
        }
        _ => Err(format!("{name} can only be set in FoxDen's window.")),
    }
}

fn color(name: &str, v: &Value) -> Result<Value, String> {
    let bad = || format!("{name} must be a colour as \"#rrggbb\" or [r,g,b].");
    if let Some(hex) = v.as_str() {
        let hex = hex.trim().trim_start_matches('#');
        return match from_hex(hex) {
            Ok(c) if c.len() == 1 => Ok(json!(c[0])),
            _ => Err(bad()),
        };
    }
    let parts = v.as_array().filter(|a| a.len() == 3).ok_or_else(bad)?;
    let rgb: Option<Vec<u64>> = parts.iter().map(|p| p.as_u64().filter(|c| *c <= 255)).collect();
    rgb.map(|c| json!(c)).ok_or_else(bad)
}

/// A device's id, from its id or its name.
fn device_id(snap: &Value, wanted: &str) -> Result<String, String> {
    let devices = snap["devices"].as_array().cloned().unwrap_or_default();
    if devices.iter().any(|d| d["id"] == wanted) {
        return Ok(wanted.to_string());
    }
    let low = wanted.to_lowercase();
    let named: Vec<&Value> = devices
        .iter()
        .filter(|d| d["name"].as_str().is_some_and(|n| n.to_lowercase() == low))
        .collect();
    match named.as_slice() {
        [one] => Ok(one["id"].as_str().unwrap_or_default().to_string()),
        [] => Err(format!("No device \"{wanted}\". get_status lists them.")),
        _ => Err(format!("Several devices are named \"{wanted}\"; use its id from get_status.")),
    }
}

fn set_effect(app: &AppHandle, args: &Value) -> Reply {
    let kind = text(args, "kind")?.to_lowercase();
    let params = match &args["params"] {
        Value::Null => Map::new(),
        Value::Object(m) => m.clone(),
        _ => return Err("params must be an object.".into()),
    };
    let snap = snapshot(app)?;
    let device = match args["device"].as_str().map(str::trim) {
        Some(d) if !d.is_empty() => Some(device_id(&snap, d)?),
        _ => None,
    };
    let current = match &device {
        Some(id) => snap["overrides"].get(id).or(Some(&snap["effect"])),
        None => Some(&snap["effect"]),
    };
    let effect = build_effect(current, &kind, &params)?;
    let mut shown = serde_json::to_value(&effect).unwrap_or_default();
    tidy(&mut shown);
    match device {
        Some(id) => commands::set_override(app.clone(), shared(app)?, id, Some(effect)),
        None => commands::set_effect(app.clone(), shared(app)?, effect),
    }?;
    Ok(json!({"effect": shown}))
}

fn save_profile(app: &AppHandle, args: &Value) -> Reply {
    let name: String = text(args, "name")?.chars().take(40).collect::<String>().trim().to_string();
    let replace = args["replace"].as_bool().unwrap_or(false);
    let s = shared(app)?;
    if !replace && lock(&s).config.profiles.contains_key(&name) {
        return Err(format!("A profile named \"{name}\" already exists. Pass replace: true to overwrite it."));
    }
    commands::save_profile(app.clone(), s, name.clone())?;
    Ok(json!(format!("Saved profile {name}.")))
}

fn get_profile(app: &AppHandle, args: &Value) -> Reply {
    let name = text(args, "name")?;
    let s = shared(app)?;
    let state = lock(&s);
    let p = state.config.profiles.get(&name).ok_or_else(|| format!("No profile named \"{name}\". list_profiles names them."))?;
    let mut out = json!({
        "name": name,
        "effect": p.effect,
        "layers": layers_view(&p.layers),
        "device_effects": p.overrides,
        "brightness": (p.brightness * 100.0).round(),
        "active": state.config.active_profile.as_deref() == Some(name.as_str()),
    });
    tidy(&mut out);
    Ok(out)
}

fn rename_profile(app: &AppHandle, args: &Value) -> Reply {
    let (name, to) = (text(args, "name")?, text(args, "to")?);
    let to: String = to.chars().take(40).collect::<String>().trim().to_string();
    let replace = args["replace"].as_bool().unwrap_or(false);
    update(app, &*shared(app)?, |s| {
        if name != to && !replace && s.config.profiles.contains_key(&to) {
            return Err(format!("A profile named \"{to}\" already exists. Pass replace: true to overwrite it."));
        }
        let p = s.config.profiles.remove(&name).ok_or_else(|| format!("No profile named \"{name}\"."))?;
        s.config.profiles.insert(to.clone(), p);
        if s.config.active_profile.as_deref() == Some(name.as_str()) {
            s.config.active_profile = Some(to.clone());
        }
        Ok(())
    })?;
    Ok(json!(format!("Renamed profile {name} to {to}.")))
}

// ---- Layers

/// Names of the keys, for key codes no one reads.
fn key_label(vk: u32) -> String {
    let named = match vk {
        0xa0 => "LShift",
        0xa1 => "RShift",
        0xa2 => "LCtrl",
        0xa3 => "RCtrl",
        0xa4 => "LAlt",
        0xa5 => "RAlt",
        0x5b => "LWin",
        0x5c => "RWin",
        0x25 => "Left",
        0x26 => "Up",
        0x27 => "Right",
        0x28 => "Down",
        0x6a => "Num*",
        0x6b => "Num+",
        0x6d => "Num-",
        0x6e => "Num.",
        0x6f => "Num/",
        0x60..=0x69 => return format!("Num{}", vk - 0x60),
        _ => return key_name(vk),
    };
    named.into()
}

fn key_code(v: &Value) -> Result<u32, String> {
    let known = |vk: u32| key_layout().iter().any(|k| k.vk == vk);
    if let Some(n) = v.as_u64() {
        return u32::try_from(n).ok().filter(|&n| known(n)).ok_or(format!("No key with code {n}. list_keys names them."));
    }
    let asked = v.as_str().map(str::trim).unwrap_or_default();
    let low = asked.to_lowercase();
    let low = match low.as_str() {
        "shift" => "lshift",
        "ctrl" | "control" => "lctrl",
        "alt" => "lalt",
        "win" | "windows" => "lwin",
        "escape" => "esc",
        "space bar" | "spacebar" => "space",
        "capslock" | "caps lock" => "caps",
        "return" => "enter",
        other => other,
    };
    key_layout()
        .iter()
        .map(|k| k.vk)
        .find(|&vk| key_label(vk).to_lowercase() == low)
        .ok_or(format!("No key named \"{asked}\". list_keys names them."))
}

/// `{key: colour}` or `[[key, colour]]`, as key codes and colours.
fn key_colours(v: &Value) -> Result<Vec<(u32, Color)>, String> {
    let pairs: Vec<(Value, Value)> = match v {
        Value::Object(m) => m.iter().map(|(k, c)| (json!(k), c.clone())).collect(),
        Value::Array(a) => a
            .iter()
            .map(|p| match p.as_array().map(Vec::as_slice) {
                Some([k, c]) => Ok((k.clone(), c.clone())),
                _ => Err("Each key colour must be [key, colour].".to_string()),
            })
            .collect::<Result<_, _>>()?,
        _ => return Err("Key colours must be an object of key name to colour.".into()),
    };
    let mut map = Vec::new();
    for (k, c) in pairs {
        let vk = key_code(&k)?;
        let c: Color = serde_json::from_value(color("colour", &c)?).map_err(|e| e.to_string())?;
        map.push((vk, c));
    }
    Ok(clean_key_map(map))
}

fn readable_map(map: &[(u32, Color)]) -> Value {
    let m: Map<String, Value> = map.iter().map(|&(vk, c)| (key_label(vk), json!(format!("#{}", to_hex(&[c]))))).collect();
    Value::Object(m)
}

/// Numbers to four places, without the noise of f32.
fn tidy(v: &mut Value) {
    match v {
        Value::Number(n) if n.is_f64() => {
            if let Some(x) = n.as_f64() {
                *v = json!((x * 10_000.0).round() / 10_000.0);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(tidy),
        Value::Object(m) => m.values_mut().for_each(tidy),
        _ => {}
    }
}

/// Key codes in an effect as key names.
fn readable_effect(e: &mut Value) {
    tidy(e);
    if let Some(keys) = e["keys"].as_array() {
        let names: Vec<String> = keys.iter().filter_map(Value::as_u64).map(|vk| key_label(vk as u32)).collect();
        e["keys"] = json!(names);
    }
    if e["map"].is_array() {
        let map: Vec<(u32, Color)> = serde_json::from_value(e["map"].clone()).unwrap_or_default();
        e["map"] = readable_map(&map);
    }
}

fn layers_view(layers: &[Layer]) -> Vec<Value> {
    layers
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let mut v = serde_json::to_value(l).unwrap_or_default();
            readable_effect(&mut v["effect"]);
            let mut out = json!({
                "index": i, "enabled": l.enabled, "effect": v["effect"],
                "opacity": (l.opacity * 100.0).round(), "blend": v["blend"], "scope": v["scope"], "when": v["when"],
            });
            if l.scope == Scope::Chosen {
                out["devices"] = json!(l.devices);
            }
            if matches!(l.when, When::Running | When::Front) {
                out["apps"] = json!(l.apps);
            }
            if l.when == When::BatteryLow {
                out["battery"] = json!(l.battery);
                out["low"] = json!(l.low);
            }
            out
        })
        .collect()
}

fn index(args: &Value, len: usize) -> Result<usize, String> {
    let i = number(args, "index")?;
    if len == 0 {
        return Err("There are no layers.".into());
    }
    if i < 0.0 || i.fract() != 0.0 || i as usize >= len {
        return Err(format!("index must be a layer index from 0 to {}; list_layers shows them.", len - 1));
    }
    Ok(i as usize)
}

fn edit_layers(app: &AppHandle, change: impl FnOnce(&mut Vec<Layer>) -> Result<(), String>) -> Reply {
    let s = shared(app)?;
    let mut layers = lock(&s).engine.layers.clone();
    change(&mut layers)?;
    commands::set_layers(app.clone(), s, layers.clone())?;
    Ok(json!({"layers": layers_view(&layers)}))
}

fn word<T: serde::de::DeserializeOwned>(args: &Value, key: &str, options: &str) -> Result<Option<T>, String> {
    match &args[key] {
        Value::Null => Ok(None),
        v => serde_json::from_value(v.clone()).map(Some).map_err(|_| format!("{key} must be one of {options}.")),
    }
}

/// Lays the layer settings found in `args` over `layer`.
fn layer_fields(layer: &mut Layer, args: &Value, snap: &Value) -> Result<(), String> {
    if !args["enabled"].is_null() {
        layer.enabled = args["enabled"].as_bool().ok_or("enabled must be true or false.")?;
    }
    if !args["opacity"].is_null() {
        let v = number(args, "opacity")?;
        if !(0.0..=100.0).contains(&v) {
            return Err("opacity must be from 0 to 100.".into());
        }
        layer.opacity = (v / 100.0) as f32;
    }
    if let Some(b) = word(args, "blend", "normal, add, screen, multiply, tint")? {
        layer.blend = b;
    }
    if !args["devices"].is_null() {
        let list = args["devices"].as_array().ok_or("devices must be a list of device ids or names.")?;
        layer.devices = list
            .iter()
            .map(|d| device_id(snap, d.as_str().unwrap_or_default().trim()))
            .collect::<Result<Vec<String>, String>>()?;
        if args["scope"].is_null() {
            layer.scope = if layer.devices.is_empty() { Scope::All } else { Scope::Chosen };
        }
    }
    if let Some(scope) = word(args, "scope", "all, keyboards, chosen")? {
        layer.scope = scope;
    }
    if !args["apps"].is_null() {
        let list = args["apps"].as_array().ok_or("apps must be a list of executable names.")?;
        layer.apps = list
            .iter()
            .filter_map(|a| a.as_str().map(str::trim).filter(|a| !a.is_empty()).map(str::to_string))
            .take(32)
            .collect();
    }
    if let Some(when) = word(args, "when", "always, running, front, battery_low")? {
        layer.when = when;
    }
    if !args["battery"].is_null() {
        layer.battery = args["battery"].as_str().ok_or("battery must be a string.")?.trim().to_string();
    }
    if !args["low"].is_null() {
        layer.low = number(args, "low")?.clamp(0.0, 100.0) as f32;
    }
    if layer.scope == Scope::Chosen && layer.devices.is_empty() {
        return Err("A layer on chosen devices needs devices.".into());
    }
    if matches!(layer.when, When::Running | When::Front) && layer.apps.is_empty() {
        return Err("A layer shown while an app runs or is in front needs apps, such as [\"game.exe\"].".into());
    }
    Ok(())
}

fn effect_params(args: &Value) -> Result<Map<String, Value>, String> {
    match &args["params"] {
        Value::Null => Ok(Map::new()),
        Value::Object(m) => Ok(m.clone()),
        _ => Err("params must be an object.".into()),
    }
}

fn add_layer(app: &AppHandle, args: &Value) -> Reply {
    let kind = text(args, "kind")?.to_lowercase();
    let effect = build_layer_effect(None, &kind, &effect_params(args)?)?;
    let mut layer = Layer::new(effect);
    let snap = snapshot(app)?;
    layer_fields(&mut layer, args, &snap)?;
    edit_layers(app, |layers| {
        let at = match &args["position"] {
            Value::Null => layers.len(),
            v => v.as_u64().filter(|&p| p as usize <= layers.len()).ok_or(format!("position must be from 0 to {}.", layers.len()))? as usize,
        };
        layers.insert(at, layer);
        Ok(())
    })
}

fn update_layer(app: &AppHandle, args: &Value) -> Reply {
    let snap = snapshot(app)?;
    edit_layers(app, |layers| {
        let i = index(args, layers.len())?;
        let mut layer = layers[i].clone();
        let current = serde_json::to_value(&layer.effect).unwrap_or_default();
        let kind = match args["kind"].as_str().map(str::trim) {
            Some(k) if !k.is_empty() => k.to_lowercase(),
            _ => current["kind"].as_str().unwrap_or_default().to_string(),
        };
        let params = effect_params(args)?;
        if !params.is_empty() || kind != current["kind"] {
            layer.effect = build_layer_effect(Some(&current), &kind, &params)?;
        }
        layer_fields(&mut layer, args, &snap)?;
        layers[i] = layer;
        Ok(())
    })
}

// ---- Devices and keys

fn set_device(app: &AppHandle, args: &Value) -> Reply {
    let snap = snapshot(app)?;
    let id = device_id(&snap, &text(args, "device")?)?;
    let d = snap["devices"].as_array().into_iter().flatten().find(|d| d["id"] == id.as_str()).cloned().unwrap_or_default();
    let enabled = match &args["enabled"] {
        Value::Null => d["enabled"].as_bool().unwrap_or(true),
        v => v.as_bool().ok_or("enabled must be true or false.")?,
    };
    let brightness = match &args["brightness"] {
        Value::Null => d["brightness"].as_f64().unwrap_or(1.0),
        _ => {
            let v = number(args, "brightness")?;
            if !(0.0..=100.0).contains(&v) {
                return Err("brightness must be from 0 to 100.".into());
            }
            v / 100.0
        }
    };
    let rect: [f32; 4] = match &args["layout"] {
        Value::Null => serde_json::from_value(d["rect"].clone()).map_err(|e| e.to_string())?,
        v => serde_json::from_value(v.clone()).map_err(|_| "layout must be [x, y, width, height], each from 0 to 1.".to_string())?,
    };
    commands::set_device(app.clone(), shared(app)?, id.clone(), enabled, brightness as f32, rect)?;
    Ok(json!({"device": id, "enabled": enabled, "brightness": (brightness * 100.0).round(), "layout": rect}))
}

fn list_keys() -> Value {
    let keys: Vec<Value> = key_layout()
        .iter()
        .map(|k| json!({"name": key_label(k.vk), "code": k.vk, "row": k.row, "x": k.x}))
        .collect();
    json!({
        "keys": keys,
        "note": "Row 0 is the function row; x is in key widths from the left. The number pad sits right of x 18.",
    })
}

fn key_map_of(app: &AppHandle, args: &Value) -> Reply {
    let snap = snapshot(app)?;
    let id = device_id(&snap, &text(args, "device")?)?;
    let s = shared(app)?;
    let device = lock(&s).engine.devices().into_iter().find(|d| d.id == id).ok_or("The device is gone.")?;
    let round = |v: f32| (v * 1000.0).round() / 1000.0;
    let found = spots(&device);
    let lights: Vec<Value> = device
        .leds
        .iter()
        .enumerate()
        .map(|(i, &[x, y])| {
            let mut out = json!({"led": i, "x": round(x), "y": round(y)});
            match found.get(i) {
                Some(Spot::Key(vk)) => out["key"] = json!(key_label(*vk)),
                Some(Spot::Zone(keys)) => out["keys"] = json!(keys.iter().map(|&vk| key_label(vk)).collect::<Vec<_>>()),
                _ => {}
            }
            out
        })
        .collect();
    let layout = if found.is_empty() {
        "not_a_keyboard"
    } else if zoned(&device) {
        "zones"
    } else {
        "per_key"
    };
    Ok(json!({
        "device": id, "name": device.name, "leds": device.leds.len(), "layout": layout, "lights": lights,
        "note": "x and y are each 0 to 1 across the device. Key layers (key_lights, key_colours) light keys by name; FoxDen has no colour per raw LED.",
    }))
}

fn save_key_map(app: &AppHandle, args: &Value) -> Reply {
    let name = text(args, "name")?;
    let map = key_colours(&args["keys"])?;
    let s = shared(app)?;
    let exists = lock(&s).config.key_maps.iter().any(|m| m.name == name);
    if exists && !args["replace"].as_bool().unwrap_or(false) {
        return Err(format!("A key map named \"{name}\" already exists. Pass replace: true to overwrite it."));
    }
    let m = KeyMap { name, map }.clean();
    let saved = m.name.clone();
    key_maps::save_key_map(app.clone(), s, m)?;
    Ok(json!(format!("Saved key map {saved}.")))
}

// ---- Settings: some change freely, a few only with the approval of the person at the PC

const SAFE_SETTINGS: [&str; 7] =
    ["fps", "language", "close_to_tray", "start_minimized", "battery_tray", "battery_alerts", "auto_detect"];
/// Settings that reach outside FoxDen, by name and the label the approval shows.
const ASKED_SETTINGS: [(&str, &str); 3] =
    [("autostart", "Start with Windows"), ("lighting_module", "Lighting"), ("native_drivers", "Native drivers")];

static LAST_SETTINGS: Mutex<Option<Value>> = Mutex::new(None);

fn get_settings(app: &AppHandle) -> Reply {
    let all = serde_json::to_value(&lock(&*shared(app)?).config.settings).map_err(|e| e.to_string())?;
    let mut free = Map::new();
    for key in SAFE_SETTINGS {
        free.insert(key.into(), all[key].clone());
    }
    let mut asked = Map::new();
    for (key, _) in ASKED_SETTINGS {
        asked.insert(key.into(), all[key].clone());
    }
    let languages: Vec<&str> = std::iter::once("auto").chain(crate::ui::settings::LANGS.iter().map(|l| l.0)).collect();
    Ok(json!({
        "settings": free,
        "need_approval": asked,
        "languages": languages,
        "last_request": lock(&LAST_SETTINGS).clone(),
        "note": "need_approval settings change only once the person at this PC approves on screen. Other settings change only in FoxDen's window.",
    }))
}

fn set_settings(app: &AppHandle, args: &Value, client: &str) -> Reply {
    let who = match fan_proposal::clean_text(client, 40) {
        c if c.is_empty() => "An AI assistant".to_string(),
        c => c,
    };
    let show = |ask: fan_proposal::Ask| {
        log::info("ai", format!("{who} asked to change settings; waiting for approval"));
        let a = app.clone();
        on_main(app, Duration::from_secs(10), move || crate::ui::approval::open(&a, ask))?
    };
    let close = |id| {
        let _ = app.run_on_main_thread(move || crate::ui::approval::close(id));
    };
    set_settings_with(app, args, client, &DESK, &show, &close, fan_proposal::WAIT)
}

fn set_settings_with(
    app: &AppHandle,
    args: &Value,
    client: &str,
    desk: &fan_proposal::Desk,
    show: &dyn Fn(fan_proposal::Ask) -> Result<(), String>,
    close: &dyn Fn(u64),
    wait: Duration,
) -> Reply {
    let given = args.as_object().ok_or("Pass the settings to change.")?;
    let asked_names: Vec<&str> = ASKED_SETTINGS.iter().map(|a| a.0).collect();
    let names = || format!("{}, and with approval {}", SAFE_SETTINGS.join(", "), asked_names.join(", "));
    let s = shared(app)?;
    let mut v = serde_json::to_value(&lock(&s).config.settings).map_err(|e| e.to_string())?;
    let mut asked = Vec::new();
    for (key, value) in given {
        if key == "reason" {
            continue;
        }
        let label = ASKED_SETTINGS.iter().find(|a| a.0 == key).map(|a| a.1);
        if !SAFE_SETTINGS.contains(&key.as_str()) && label.is_none() {
            return Err(format!("{key} changes only in FoxDen's window. These can change here: {}.", names()));
        }
        let ok = match key.as_str() {
            "fps" => value.as_u64().is_some_and(|f| (10..=60).contains(&f)),
            "language" => value
                .as_str()
                .is_some_and(|l| l == "auto" || crate::ui::settings::LANGS.iter().any(|x| x.0 == l)),
            _ => value.is_boolean(),
        };
        if !ok {
            let want = match key.as_str() {
                "fps" => "a whole number from 10 to 60",
                "language" => "\"auto\" or a code from get_settings",
                _ => "true or false",
            };
            return Err(format!("{key} must be {want}."));
        }
        if let Some(label) = label.filter(|_| v[key.as_str()] != *value) {
            asked.push((label, value.as_bool().unwrap_or_default()));
        }
        v[key.as_str()] = value.clone();
    }
    if given.keys().all(|k| k == "reason") {
        return Err(format!("Pass at least one of {}.", names()));
    }
    let settings: Settings = serde_json::from_value(v).map_err(|e| e.to_string())?;
    let jobs = app.try_state::<Jobs>().ok_or("FoxDen is still starting.")?;
    if !asked.is_empty() {
        let reason = match args["reason"].as_str().map(|r| fan_proposal::clean_text(r, 300)) {
            Some(r) if !r.is_empty() => r,
            _ => return Err("Changing these settings needs a reason: a sentence for the person approving.".into()),
        };
        let client = fan_proposal::clean_text(client, 40);
        let record = |outcome: &str| *lock(&LAST_SETTINGS) = Some(json!({"client": client, "outcome": outcome}));
        let make = |id| {
            record("waiting for approval");
            fan_proposal::Ask { id, client: client.clone(), reason, diff: Default::default(), settings: asked, other: None }
        };
        match await_answer(desk, show, close, make, wait) {
            Err(e) => {
                if e != fan_proposal::BUSY {
                    record("not shown");
                }
                return Err(e);
            }
            Ok(None) => {
                record("not approved in time");
                return Err(fan_proposal::LATE.into());
            }
            Ok(Some(fan_proposal::Decision::Reject)) => {
                record("rejected");
                return Ok(json!(fan_proposal::REJECTED));
            }
            Ok(Some(fan_proposal::Decision::Apply)) => record("applied"),
        }
    }
    commands::set_settings(app.clone(), s, jobs, settings)?;
    get_settings(app)
}

// ---- Macros: reading, deleting and stopping are free; anything that sends input needs approval

/// How an approval is asked for, so tests can answer it.
struct Gate<'a> {
    desk: &'a fan_proposal::Desk,
    show: &'a dyn Fn(fan_proposal::Ask) -> Result<(), String>,
    close: &'a dyn Fn(u64),
    wait: Duration,
}

static LAST_MACRO: Mutex<Option<Value>> = Mutex::new(None);
const MACRO_LINES: usize = 14;

fn macro_tool(app: &AppHandle, tool: &str, args: &Value, client: &str) -> Reply {
    let who = match fan_proposal::clean_text(client, 40) {
        c if c.is_empty() => "An AI assistant".to_string(),
        c => c,
    };
    let show = |ask: fan_proposal::Ask| {
        log::info("ai", format!("{who} asked to use macros; waiting for approval"));
        let a = app.clone();
        on_main(app, Duration::from_secs(10), move || crate::ui::approval::open(&a, ask))?
    };
    let close = |id| {
        let _ = app.run_on_main_thread(move || crate::ui::approval::close(id));
    };
    let gate = Gate { desk: &DESK, show: &show, close: &close, wait: fan_proposal::WAIT };
    macro_tool_with(app, tool, args, client, &gate)
}

/// Asks the person at the PC; true when they approved.
fn approve(gate: &Gate, args: &Value, client: &str, title: &'static str, lines: Vec<String>) -> Result<bool, String> {
    let reason = match args["reason"].as_str().map(|r| fan_proposal::clean_text(r, 300)) {
        Some(r) if !r.is_empty() => r,
        _ => return Err("This needs a reason: a sentence for the person approving.".into()),
    };
    let client = fan_proposal::clean_text(client, 40);
    let record = |outcome: &str| *lock(&LAST_MACRO) = Some(json!({"client": client, "outcome": outcome}));
    let make = |id| {
        record("waiting for approval");
        fan_proposal::Ask {
            id,
            client: client.clone(),
            reason,
            diff: Default::default(),
            settings: Vec::new(),
            other: Some((title, lines)),
        }
    };
    match fan_proposal::await_answer(gate.desk, gate.show, gate.close, make, gate.wait) {
        Err(e) => {
            if e != fan_proposal::BUSY {
                record("not shown");
            }
            Err(e)
        }
        Ok(None) => {
            record("not approved in time");
            Err(fan_proposal::LATE.into())
        }
        Ok(Some(fan_proposal::Decision::Reject)) => {
            record("rejected");
            Ok(false)
        }
        Ok(Some(fan_proposal::Decision::Apply)) => {
            record("approved");
            Ok(true)
        }
    }
}

fn macro_view(e: &foxden_macros::Entry) -> Value {
    let mut v = foxden_macros::spec::macro_json(&e.macro_);
    v["id"] = json!(e.id);
    v["hotkey"] = foxden_macros::spec::hotkey_json(e.hotkey);
    v["block"] = json!(e.block);
    v
}

fn step_lines(m: &foxden_macros::Macro) -> Vec<String> {
    let all: Vec<&foxden_macros::Step> = m.press.iter().chain(&m.hold).chain(&m.release).collect();
    let mut lines: Vec<String> =
        all.iter().take(MACRO_LINES).enumerate().map(|(i, s)| format!("{}. {}", i + 1, s.describe())).collect();
    if all.len() > MACRO_LINES {
        lines.push(format!("and {} more steps", all.len() - MACRO_LINES));
    }
    lines
}

fn macro_tool_with(app: &AppHandle, tool: &str, args: &Value, client: &str, gate: &Gate) -> Reply {
    use foxden_macros::{Entry, Hotkey, spec};
    let m = crate::macros::get(app)?;
    let find = |key: &str| -> Result<Entry, String> {
        let id = text(args, key)?;
        m.find(&id).ok_or_else(|| format!("No macro {id}. list_macros shows them."))
    };
    match tool {
        "list_macros" => {
            let lib = m.library();
            let running = m.host().running();
            let names: Vec<&str> = foxden_macros::keys::all().map(|k| k.1).chain(["Pause"]).collect();
            Ok(json!({
                "enabled": lib.enabled,
                "running": running,
                "macros": lib.entries.iter().map(macro_view).collect::<Vec<_>>(),
                "key_names": names,
                "last_request": lock(&LAST_MACRO).clone(),
                "note": "Pause stops every macro. Macros reach apps running as administrator only when FoxDen runs as administrator too.",
            }))
        }
        "save_macro" => {
            let old = match args["id"].as_str() {
                Some(_) => Some(find("id")?),
                None => None,
            };
            let macro_ = spec::macro_from(args)?;
            let hotkey = match args.get("hotkey") {
                None => old.as_ref().and_then(|o| o.hotkey),
                Some(Value::Null) => None,
                Some(Value::String(h)) if h.trim().is_empty() => None,
                Some(Value::String(h)) => Some(Hotkey::parse(h)?),
                Some(_) => return Err("hotkey must be text such as \"F13\", or null.".into()),
            };
            let block = args["block"].as_bool().or(old.as_ref().map(|o| o.block)).unwrap_or(false);
            let entry = Entry { id: old.map(|o| o.id).unwrap_or_default(), macro_, hotkey, block };
            m.library().put(entry.clone())?;
            let mut lines = vec![
                format!("Name: {}", entry.macro_.name),
                match hotkey {
                    Some(h) if block => format!("Hotkey: {} (hidden from other apps)", h.label()),
                    Some(h) => format!("Hotkey: {}", h.label()),
                    None => "No hotkey".to_string(),
                },
                format!("Mode: {}", entry.macro_.mode.label()),
            ];
            lines.extend(step_lines(&entry.macro_));
            if !approve(gate, args, client, crate::ui::approval::SAVE_MACRO, lines)? {
                return Ok(json!(fan_proposal::REJECTED));
            }
            let name = entry.macro_.name.clone();
            let id = m.put(entry)?;
            crate::macros::changed(app);
            Ok(json!({"id": id, "saved": name}))
        }
        "delete_macro" => {
            let e = find("id")?;
            m.remove(&e.id)?;
            crate::macros::changed(app);
            Ok(json!(format!("Deleted macro {}.", e.macro_.name)))
        }
        "play_macro" => {
            let e = find("id")?;
            let delay = args["delay_s"].as_f64().unwrap_or(3.0);
            if !(0.0..=30.0).contains(&delay) {
                return Err("delay_s must be from 0 to 30.".into());
            }
            let mut lines = vec![format!("Play \"{}\" in {} s", e.macro_.name, delay)];
            lines.extend(step_lines(&e.macro_));
            if !approve(gate, args, client, crate::ui::approval::PLAY_MACRO, lines)? {
                return Ok(json!(fan_proposal::REJECTED));
            }
            m.play(&e.id, Duration::from_secs_f64(delay))?;
            Ok(json!(format!("Playing {} in {delay} s. Pause stops it.", e.macro_.name)))
        }
        "stop_macros" => {
            m.stop_all();
            Ok(json!("Stopped every macro."))
        }
        "set_macros_enabled" => {
            let on = args["on"].as_bool().ok_or("on must be true or false.")?;
            let lib = m.library();
            if on && !lib.enabled {
                let mut lines: Vec<String> = lib
                    .entries
                    .iter()
                    .filter_map(|e| e.hotkey.map(|h| format!("{}: {}", h.label(), e.macro_.name)))
                    .collect();
                if lines.is_empty() {
                    lines.push("No macro has a hotkey yet".into());
                }
                if !approve(gate, args, client, crate::ui::approval::MACROS_ON, lines)? {
                    return Ok(json!(fan_proposal::REJECTED));
                }
            }
            m.set_enabled(on)?;
            crate::macros::changed(app);
            Ok(json!(if on { "Macro hotkeys are on." } else { "Macro hotkeys are off." }))
        }
        _ => Err(format!("FoxDen has no tool named {tool}.")),
    }
}

// ---- Fans, read only

fn sensors() -> Reply {
    let state = serde_json::to_value(fans::get_fans()?).map_err(|e| e.to_string())?;
    let sensors: Vec<Value> = state["sensors"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|s| json!({"name": s["name"], "kind": s["kind"], "value": s["value"]}))
        .collect();
    let fans: Vec<Value> = state["fans"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|f| json!({"name": f["name"], "rpm": f["rpm"], "duty_percent": percent(&f["duty"]), "curve": f["curve"]}))
        .collect();
    Ok(json!({"service": state["service"], "sensors": sensors, "fans": fans}))
}

// ---- Fan curves, only with the approval of the person at the PC

fn on_main<T: Send + 'static>(app: &AppHandle, wait: Duration, f: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    let (tx, rx) = mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(f());
    })
    .map_err(|e| e.to_string())?;
    rx.recv_timeout(wait).map_err(|_| "FoxDen's window did not answer.".to_string())
}

fn propose(app: &AppHandle, args: &Value, client: &str) -> Reply {
    let state = fans::get_fans()?;
    let who = match fan_proposal::clean_text(client, 40) {
        c if c.is_empty() => "An AI assistant".to_string(),
        c => c,
    };
    let show = |ask: fan_proposal::Ask| {
        log::info("ai", format!("{who} proposed fan curves; waiting for approval"));
        let a = app.clone();
        on_main(app, Duration::from_secs(10), move || crate::ui::approval::open(&a, ask))?
    };
    let close = |id| {
        let _ = app.run_on_main_thread(move || crate::ui::approval::close(id));
    };
    let apply = |config| {
        let (tx, rx) = mpsc::channel();
        app.run_on_main_thread(move || crate::ui::apply_proposed_fans(config, tx)).map_err(|e| e.to_string())?;
        rx.recv_timeout(Duration::from_secs(15)).map_err(|_| "The fan service didn't answer in time.".to_string())?
    };
    let hooks = Hooks { show: &show, close: &close, apply: &apply };
    let result = fan_proposal::propose(&DESK, &state, args, client, &hooks, fan_proposal::WAIT);
    let said = match &result {
        Ok(text) | Err(text) => text,
    };
    log::info("ai", format!("Fan curve proposal from {who}: {said}"));
    result.map(|text| json!(text))
}

// ---- Audio

const CHANNELS: [(&str, &str, &str); 5] = [
    ("game", "game", "Game"),
    ("chat", "chatRender", "Chat"),
    ("media", "media", "Media"),
    ("aux", "aux", "Aux"),
    ("mic", "chatCapture", "Mic"),
];

/// The mixer's id and label of the channel named in `args`.
fn channel(args: &Value) -> Result<(&'static str, &'static str), String> {
    let asked = args["channel"].as_str().unwrap_or_default().trim().to_lowercase();
    CHANNELS
        .iter()
        .find(|(name, id, _)| *name == asked || id.to_lowercase() == asked)
        .map(|(_, id, label)| (*id, *label))
        .ok_or_else(|| "channel must be game, chat, media, aux or mic.".into())
}

fn mixer_state(app: &AppHandle) -> Result<State<'static, Mixer>, String> {
    app.try_state::<Mixer>().ok_or_else(|| "The audio mixer isn't available.".into())
}

fn mixer_view(app: &AppHandle) -> Reply {
    let view = mixer::get_mixer(mixer_state(app)?, false)?;
    serde_json::to_value(view).map_err(|e| e.to_string())
}

/// Applies a mixer change and has the Audio tab show it.
fn mix(app: &AppHandle, change: Value) -> Reply {
    let change = serde_json::from_value(change).map_err(|e| e.to_string())?;
    let view = mixer::mixer_set(mixer_state(app)?, change)?;
    let _ = app.emit("mixer", Value::Null);
    serde_json::to_value(view).map_err(|e| e.to_string())
}

fn audio(app: &AppHandle) -> Reply {
    let v = mixer_view(app)?;
    let config = &v["config"];
    let channels: Vec<Value> = config["channels"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| {
            let name = CHANNELS.iter().find(|x| c["channel"] == x.1).map_or("", |x| x.0);
            json!({
                "channel": name, "on": c["on"], "volume": percent(&c["volume"]), "muted": c["muted"],
                "source": c["source"], "apps": c["apps"],
            })
        })
        .collect();
    let apps: Vec<&Value> = v["apps"].as_array().into_iter().flatten().map(|a| &a["exe"]).collect();
    Ok(json!({
        "running": v["running"], "output": config["output"], "playing_on": v["playing_on"],
        "master": percent(&config["master"]), "master_muted": config["master_muted"],
        "channels": channels, "apps_with_sound": apps, "problem": v["error"],
    }))
}

fn assign_app(app: &AppHandle, args: &Value) -> Reply {
    let exe = text(args, "app")?;
    let (id, label) = channel(args)?;
    if id == "chatCapture" {
        return Err("The mic takes an input, not apps.".into());
    }
    let view = mixer_view(app)?;
    let find = |v: &Value| {
        v["config"]["channels"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|c| c["channel"] == id)
            .cloned()
            .unwrap_or_default()
    };
    let ch = find(&view);
    let mut apps: Vec<String> = ch["apps"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| a.as_str().map(str::to_string))
        .collect();
    if !apps.iter().any(|a| a.eq_ignore_ascii_case(&exe)) {
        apps.push(exe.clone());
    }
    let after = mix(app, json!({"kind": "apps", "channel": id, "apps": apps}))?;
    let ch = find(&after);
    if ch["on"] == false {
        mix(app, json!({"kind": "route", "channel": id, "source": ch["source"], "loopback": ch["loopback"], "on": true}))?;
    }
    Ok(json!(format!("{exe} now plays on {label}.")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::state::AppState;
    use foxden_core::Color;

    fn app_with(ai_access: bool) -> (AppHandle, Shared) {
        let mut config = Config::default();
        config.settings.ai_access = ai_access;
        let shared: Shared = Arc::new(Mutex::new(AppState::new(config)));
        let app = AppHandle::new();
        app.manage(shared.clone());
        let (jobs, rx) = mpsc::channel::<crate::state::Job>();
        std::mem::forget(rx);
        app.manage::<Jobs>(jobs);
        (app, shared)
    }

    fn with_devices() -> (AppHandle, Shared) {
        let (app, shared) = app_with(true);
        lock(&shared).engine.add("hid", vec![Box::new(foxden_core::VirtualBackend)]);
        (app, shared)
    }

    fn ok(app: &AppHandle, tool: &str, args: Value) -> Value {
        let r = ask(app, tool, args);
        assert!(r["error"].is_null(), "{tool}: {r}");
        r["ok"].clone()
    }

    fn err(app: &AppHandle, tool: &str, args: Value) -> String {
        let r = ask(app, tool, args);
        r["error"].as_str().unwrap_or_else(|| panic!("{tool} should fail: {r}")).to_string()
    }

    #[test]
    fn every_listed_tool_is_answered() {
        let source = include_str!("../../crates/foxden-mcp/src/tools.rs");
        let names: Vec<&str> = source
            .split("tool(")
            .skip(1)
            .filter_map(|rest| rest.trim_start().strip_prefix('"')?.split('"').next())
            .collect();
        assert!(names.len() > 30, "{names:?}");
        let (app, _) = app_with(true);
        for name in names {
            let line = json!({"tool": name, "args": {"name": "\u{0}none"}}).to_string();
            if name == "propose_fan_curves" {
                continue;
            }
            let r = json_of(&handle(&app, &line));
            assert!(!r["error"].as_str().unwrap_or_default().contains("no tool named"), "{name}: {r}");
        }
    }

    #[test]
    fn layers_are_added_changed_moved_and_removed() {
        let (app, shared) = with_devices();
        let l = ok(&app, "add_layer", json!({"kind": "glitter", "params": {"color": "#ff0000", "density": 0.1}}));
        assert_eq!(l["layers"][0]["effect"]["kind"], "glitter");
        assert_eq!(l["layers"][0]["effect"]["color"], json!([255, 0, 0]));
        assert_eq!(ok(&app, "get_status", json!({}))["layers"][0]["kind"], "glitter");

        let l = ok(&app, "add_layer", json!({
            "kind": "key_lights", "params": {"keys": ["q", "W", "E", "R", "Space", "F1"], "color": [0, 200, 255]},
            "when": "front", "apps": ["League of Legends.exe"], "scope": "keyboards", "opacity": 80,
        }));
        let keys = &l["layers"][1];
        assert_eq!(keys["effect"]["keys"], json!(["Space", "E", "Q", "R", "W", "F1"]));
        assert_eq!((keys["when"].clone(), keys["opacity"].clone()), (json!("front"), json!(80.0)));
        assert_eq!(keys["apps"], json!(["League of Legends.exe"]));
        let engine_layer = lock(&shared).engine.layers[1].clone();
        assert_eq!(engine_layer.effect, LayerEffect::Post(Post::KeyLights { color: Color(0, 200, 255), keys: vec![0x20, 0x45, 0x51, 0x52, 0x57, 0x70] }));

        let l = ok(&app, "add_layer", json!({"kind": "key_colours", "params": {"map": {"Esc": "#00ff00", "LShift": "#0000ff"}}, "position": 0}));
        assert_eq!(l["layers"][0]["effect"]["map"], json!({"Esc": "#00ff00", "LShift": "#0000ff"}));
        let l = ok(&app, "add_layer", json!({"kind": "rainbow", "devices": ["TKL keyboard"]}));
        assert_eq!((l["layers"][3]["scope"].clone(), l["layers"][3]["devices"].clone()), (json!("chosen"), json!(["virtual:keyboard"])));

        let l = ok(&app, "update_layer", json!({"index": 1, "params": {"density": 0.3}, "blend": "add"}));
        assert_eq!(l["layers"][1]["effect"]["density"], json!(0.3));
        assert_eq!(l["layers"][1]["effect"]["color"], json!([255, 0, 0]));
        assert_eq!(l["layers"][1]["blend"], "add");
        let l = ok(&app, "toggle_layer", json!({"index": 1}));
        assert_eq!(l["layers"][1]["enabled"], false);
        let l = ok(&app, "move_layer", json!({"index": 1, "to": 3}));
        assert_eq!(l["layers"][3]["effect"]["kind"], "glitter");
        let l = ok(&app, "remove_layer", json!({"index": 3}));
        assert_eq!(l["layers"].as_array().unwrap().len(), 3);
        assert_eq!(ok(&app, "list_layers", json!({}))["layers"], l["layers"]);

        assert!(err(&app, "add_layer", json!({"kind": "key_lights", "params": {"keys": ["Nope"]}})).contains("No key named"));
        assert!(err(&app, "add_layer", json!({"kind": "glitter", "when": "front"})).contains("needs apps"));
        assert!(err(&app, "add_layer", json!({"kind": "none"})).contains("remove_layer"));
        assert!(err(&app, "add_layer", json!({"kind": "lava"})).contains("Layer effects"));
        assert!(err(&app, "update_layer", json!({"index": 9})).contains("from 0 to 2"));
        assert!(err(&app, "update_layer", json!({"index": 0, "blend": "melt"})).contains("normal"));
        assert_eq!(lock(&shared).engine.layers.len(), 3);
    }

    #[test]
    fn settings_that_reach_outside_wait_for_approval() {
        let (app, shared) = app_with(true);
        let desk = fan_proposal::Desk::new();
        let shown = Mutex::new(Vec::new());
        let answer = Mutex::new(Some(fan_proposal::Decision::Reject));
        let show = |ask: fan_proposal::Ask| {
            lock(&shown).push(ask.settings.clone());
            if let Some(d) = *lock(&answer) {
                desk.decide(ask.id, d);
            }
            Ok(())
        };
        let close = |_| {};
        let set = |args: Value| set_settings_with(&app, &args, "Tester", &desk, &show, &close, Duration::from_millis(50));
        let ask = json!({"native_drivers": false, "fps": 20, "reason": "Testing"});

        assert_eq!(set(ask.clone()), Ok(json!(fan_proposal::REJECTED)));
        assert!(lock(&shared).config.settings.native_drivers);
        assert_eq!(lock(&shared).config.settings.fps, 30);
        assert_eq!(lock(&shown)[0], [("Native drivers", false)]);

        *lock(&answer) = Some(fan_proposal::Decision::Apply);
        let after = set(ask.clone()).unwrap();
        assert_eq!((after["need_approval"]["native_drivers"].clone(), after["settings"]["fps"].clone()), (json!(false), json!(20)));
        assert_eq!(after["last_request"]["outcome"], "applied");

        let before = lock(&shown).len();
        set(json!({"native_drivers": false, "fps": 25})).unwrap();
        assert_eq!(lock(&shown).len(), before);

        *lock(&answer) = None;
        assert_eq!(set(json!({"native_drivers": true, "reason": "Back"})), Err(fan_proposal::LATE.into()));
        assert!(!lock(&shared).config.settings.native_drivers);
        assert_eq!(ok(&app, "get_settings", json!({}))["last_request"]["outcome"], "not approved in time");
    }

    #[test]
    fn macros_send_input_only_once_approved() {
        let (app, _) = app_with(true);
        let (store, dir) = crate::macros::tests::temp();
        app.manage(store);
        let desk = fan_proposal::Desk::new();
        let shown: Mutex<Vec<(String, Vec<String>)>> = Mutex::new(Vec::new());
        let answer = Mutex::new(fan_proposal::Decision::Reject);
        let show = |ask: fan_proposal::Ask| {
            let (title, lines) = ask.other.clone().unwrap_or_default();
            lock(&shown).push((title.to_string(), lines));
            desk.decide(ask.id, *lock(&answer));
            Ok(())
        };
        let close = |_| {};
        let gate = Gate { desk: &desk, show: &show, close: &close, wait: Duration::from_millis(50) };
        let run = |tool: &str, args: Value| macro_tool_with(&app, tool, &args, "Tester", &gate);
        let copy = json!({
            "name": "Copy", "hotkey": "Ctrl+F13", "block": true, "reason": "Testing",
            "press": [{"type": "key", "key": "Ctrl", "action": "down"}, {"type": "key", "key": "C"}, {"type": "key", "key": "Ctrl", "action": "up"}],
        });

        let mut no_reason = copy.clone();
        no_reason["reason"] = Value::Null;
        assert!(run("save_macro", no_reason).unwrap_err().contains("reason"));
        assert_eq!(run("save_macro", copy.clone()), Ok(json!(fan_proposal::REJECTED)));
        let list = run("list_macros", json!({})).unwrap();
        assert_eq!(list["macros"], json!([]));
        assert_eq!(list["last_request"]["outcome"], "rejected");
        assert_eq!(lock(&shown)[0].0, "{0} wants to save a macro");
        assert_eq!(lock(&shown)[0].1[..3], ["Name: Copy", "Hotkey: Ctrl+F13 (hidden from other apps)", "Mode: Once"]);
        assert_eq!(lock(&shown)[0].1[4], "2. C down");

        *lock(&answer) = fan_proposal::Decision::Apply;
        assert_eq!(run("save_macro", copy.clone()).unwrap()["id"], "m1");
        let list = run("list_macros", json!({})).unwrap();
        assert_eq!(list["macros"][0]["hotkey"], "Ctrl+F13");
        assert_eq!(list["macros"][0]["press"].as_array().unwrap().len(), 4);
        assert!(list["key_names"].as_array().unwrap().contains(&json!("F13")));

        let asked = lock(&shown).len();
        let mut clash = copy.clone();
        clash["name"] = json!("Other");
        assert!(run("save_macro", clash).unwrap_err().contains("already starts"));
        assert!(run("save_macro", json!({"name": "x", "reason": "r", "press": [{"type": "jump"}]})).is_err());
        assert!(run("play_macro", json!({"id": "nope", "reason": "r"})).unwrap_err().contains("No macro"));
        assert!(run("play_macro", json!({"id": "Copy", "reason": "r", "delay_s": 99})).unwrap_err().contains("delay_s"));
        assert_eq!(lock(&shown).len(), asked);

        let mut edit = json!({"id": "m1", "name": "Copy", "reason": "r", "press": [{"type": "text", "text": "hi"}]});
        assert_eq!(run("save_macro", edit.clone()).unwrap()["id"], "m1");
        assert_eq!(run("list_macros", json!({})).unwrap()["macros"][0]["hotkey"], "Ctrl+F13");
        edit["hotkey"] = Value::Null;
        run("save_macro", edit).unwrap();
        assert_eq!(run("list_macros", json!({})).unwrap()["macros"][0]["hotkey"], Value::Null);

        *lock(&answer) = fan_proposal::Decision::Reject;
        assert_eq!(run("set_macros_enabled", json!({"on": true, "reason": "r"})), Ok(json!(fan_proposal::REJECTED)));
        assert_eq!(run("play_macro", json!({"id": "copy", "reason": "r"})), Ok(json!(fan_proposal::REJECTED)));
        assert_eq!(lock(&shown).last().unwrap().1[0], "Play \"Copy\" in 3 s");
        let list = run("list_macros", json!({})).unwrap();
        assert_eq!((list["enabled"].clone(), list["running"].clone()), (json!(false), json!([])));
        let asked = lock(&shown).len();
        run("set_macros_enabled", json!({"on": false})).unwrap();
        run("stop_macros", json!({})).unwrap();
        assert!(run("delete_macro", json!({"id": "Copy"})).unwrap().as_str().unwrap().contains("Copy"));
        assert!(run("delete_macro", json!({"id": "Copy"})).is_err());
        assert_eq!(lock(&shown).len(), asked);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn every_key_has_its_own_name() {
        let mut seen = std::collections::HashSet::new();
        for k in key_layout() {
            let name = key_label(k.vk);
            assert!(!name.is_empty() && seen.insert(name.to_lowercase()), "{:#x} {name}", k.vk);
            assert_eq!(key_code(&json!(name)), Ok(k.vk));
        }
    }

    #[test]
    fn keys_and_devices_are_described_and_set() {
        let (app, shared) = with_devices();
        let keys = ok(&app, "list_keys", json!({}))["keys"].clone();
        assert!(keys.as_array().unwrap().iter().any(|k| k["name"] == "Q" && k["code"] == 0x51));
        let map = ok(&app, "get_key_map", json!({"device": "virtual:keyboard"}));
        assert_eq!(map["layout"], "per_key");
        assert_eq!(map["leds"], 17 * 6);
        assert_eq!(map["lights"][0]["key"], "Esc");
        assert_eq!(ok(&app, "get_key_map", json!({"device": "virtual:gpu"}))["layout"], "not_a_keyboard");

        let d = ok(&app, "set_device", json!({"device": "virtual:gpu", "brightness": 50, "enabled": false}));
        assert_eq!((d["brightness"].clone(), d["enabled"].clone()), (json!(50.0), json!(false)));
        let status = ok(&app, "get_status", json!({}));
        let gpu = status["devices"].as_array().unwrap().iter().find(|d| d["id"] == "virtual:gpu").unwrap().clone();
        assert_eq!((gpu["brightness"].clone(), gpu["enabled"].clone()), (json!(50.0), json!(false)));
        assert!(err(&app, "set_device", json!({"device": "virtual:gpu", "brightness": 150})).contains("0 to 100"));

        ok(&app, "set_effect", json!({"kind": "static", "device": "virtual:gpu"}));
        assert!(lock(&shared).engine.overrides.contains_key("virtual:gpu"));
        ok(&app, "clear_device_effect", json!({"device": "virtual:gpu"}));
        assert!(lock(&shared).engine.overrides.is_empty());

        ok(&app, "save_key_map", json!({"name": "Binds", "keys": {"Q": "#ff0000", "Num5": "#00ff00"}}));
        assert!(err(&app, "save_key_map", json!({"name": "Binds", "keys": {}})).contains("replace"));
        let maps = ok(&app, "list_key_maps", json!({}))["key_maps"].clone();
        assert_eq!(maps, json!([{"name": "Binds", "keys": {"Num5": "#00ff00", "Q": "#ff0000"}}]));
        ok(&app, "delete_key_map", json!({"name": "Binds"}));
        assert!(lock(&shared).config.key_maps.is_empty());
        assert!(!ask(&app, "rescan_devices", json!({})).to_string().contains("no tool"));
    }

    #[test]
    fn profiles_are_read_renamed_and_deleted() {
        let (app, shared) = app_with(true);
        ok(&app, "add_layer", json!({"kind": "glitter"}));
        ok(&app, "save_profile", json!({"name": "Game"}));
        let p = ok(&app, "get_profile", json!({"name": "Game"}));
        assert_eq!((p["layers"][0]["effect"]["kind"].clone(), p["active"].clone()), (json!("glitter"), json!(true)));
        ok(&app, "save_profile", json!({"name": "Other"}));
        assert!(err(&app, "rename_profile", json!({"name": "Game", "to": "Other"})).contains("replace"));
        ok(&app, "apply_profile", json!({"name": "Game"}));
        ok(&app, "rename_profile", json!({"name": "Game", "to": "League"}));
        assert_eq!(lock(&shared).config.active_profile.as_deref(), Some("League"));
        ok(&app, "delete_profile", json!({"name": "League"}));
        assert_eq!(ok(&app, "list_profiles", json!({})), json!({"profiles": ["Other"], "active": null}));
        assert!(err(&app, "get_profile", json!({"name": "League"})).contains("No profile"));
    }

    #[test]
    fn only_safe_settings_change() {
        let (app, shared) = app_with(true);
        let s = ok(&app, "set_settings", json!({"fps": 45, "battery_alerts": false, "language": "en"}));
        assert_eq!(s["settings"]["fps"], 45);
        assert!(!lock(&shared).config.settings.battery_alerts);
        assert!(err(&app, "set_settings", json!({"ai_access": false})).contains("only in FoxDen's window"));
        assert!(err(&app, "set_settings", json!({"native_drivers": false})).contains("needs a reason"));
        assert!(lock(&shared).config.settings.native_drivers);
        assert!(err(&app, "set_settings", json!({"close_conflicts": true})).contains("only in FoxDen's window"));
        assert!(err(&app, "set_settings", json!({"fps": 5})).contains("10 to 60"));
        assert!(err(&app, "set_settings", json!({"language": "xx"})).contains("auto"));
        assert!(lock(&shared).config.settings.ai_access);
        assert!(ok(&app, "get_settings", json!({}))["settings"].get("ai_access").is_none());
    }

    fn ask(app: &AppHandle, tool: &str, args: Value) -> Value {
        serde_json::from_str(&handle(app, &json!({"tool": tool, "args": args}).to_string())).unwrap()
    }

    #[test]
    fn requests_are_refused_while_access_is_off() {
        let (app, shared) = app_with(false);
        let r = ask(&app, "set_brightness", json!({"value": 10}));
        assert_eq!(r["error"], TURNED_OFF);
        assert_eq!(lock(&shared).engine.brightness, 1.0);
        assert!(json_of(&handle(&app, "nonsense"))["error"].is_string());
    }

    fn json_of(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn lighting_changes_go_through_the_app_state() {
        let (app, shared) = app_with(true);
        assert!(ask(&app, "set_brightness", json!({"value": 40}))["ok"].is_string());
        assert!((lock(&shared).engine.brightness - 0.4).abs() < 1e-6);
        assert!(ask(&app, "set_brightness", json!({"value": 140}))["error"].is_string());

        let r = ask(&app, "set_effect", json!({"kind": "static", "params": {"color": "#ff0000"}}));
        assert!(r["ok"].is_object(), "{r}");
        assert_eq!(lock(&shared).engine.effect, Effect::Static { color: Color(255, 0, 0) });

        let r = ask(&app, "set_effect", json!({"kind": "static", "params": {"speed": 1}}));
        assert!(r["error"].as_str().unwrap().contains("no parameter"), "{r}");
        let r = ask(&app, "set_effect", json!({"kind": "lava"}));
        assert!(r["error"].as_str().unwrap().contains("Unknown effect kind"), "{r}");
        let r = ask(&app, "set_effect", json!({"kind": "static", "device": "nothing here"}));
        assert!(r["error"].as_str().unwrap().contains("No device"), "{r}");

        assert!(ask(&app, "pause", json!({}))["ok"].is_string());
        assert!(lock(&shared).engine.paused);
        assert!(ask(&app, "resume", json!({}))["ok"].is_string());
        assert!(!lock(&shared).engine.paused);
    }

    #[test]
    fn ranges_are_held_to_the_catalog() {
        let e = build_effect(None, "rainbow", json!({"speed": 99, "angle": -5}).as_object().unwrap()).unwrap();
        assert_eq!(e, Effect::Rainbow { speed: 5.0, scale: 1.0, angle: 0.0 });
        let current = json!({"kind": "rainbow", "speed": 2.0, "scale": 3.0, "angle": 90.0});
        let e = build_effect(Some(&current), "rainbow", json!({"speed": 1}).as_object().unwrap()).unwrap();
        assert_eq!(e, Effect::Rainbow { speed: 1.0, scale: 3.0, angle: 90.0 });
        assert!(build_effect(None, "static", json!({"color": "red"}).as_object().unwrap()).is_err());
        let e = build_effect(None, "static", json!({"color": [1, 2, 3]}).as_object().unwrap()).unwrap();
        assert_eq!(e, Effect::Static { color: Color(1, 2, 3) });
    }

    #[test]
    fn profiles_are_saved_and_applied_like_the_ui_does() {
        let (app, shared) = app_with(true);
        ask(&app, "set_brightness", json!({"value": 50}));
        assert!(ask(&app, "save_profile", json!({"name": " Night "}))["ok"].is_string());
        assert!(ask(&app, "save_profile", json!({"name": "Night"}))["error"].is_string());
        assert!(ask(&app, "save_profile", json!({"name": "Night", "replace": true}))["ok"].is_string());
        ask(&app, "set_brightness", json!({"value": 90}));
        assert!(ask(&app, "apply_profile", json!({"name": "Night"}))["ok"].is_string());
        assert!((lock(&shared).engine.brightness - 0.5).abs() < 1e-6);
        let list = ask(&app, "list_profiles", json!({}));
        assert_eq!(list["ok"], json!({"profiles": ["Night"], "active": "Night"}));
        assert!(ask(&app, "apply_profile", json!({"name": "Day"}))["error"].is_string());
    }

    #[test]
    fn reads_describe_the_app_without_hardware() {
        let (app, _) = app_with(true);
        let status = ask(&app, "get_status", json!({}))["ok"].clone();
        assert_eq!(status["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(status["brightness"], 100.0);
        assert!(status["devices"].is_array() && status["effect"]["kind"].is_string());
        let effects = ask(&app, "list_effects", json!({}))["ok"]["effects"].clone();
        let rainbow = effects.as_array().unwrap().iter().find(|e| e["kind"] == "rainbow").unwrap();
        assert_eq!(rainbow["params"][0]["type"], "range");
        assert!(ask(&app, "get_audio", json!({}))["error"].is_string());
        assert!(ask(&app, "set_channel_volume", json!({"channel": "nope", "volume": 5}))["error"].is_string());
        assert!(ask(&app, "get_batteries", json!({}))["ok"]["batteries"].is_array());
        assert!(ask(&app, "set_fan_curve", json!({}))["error"].is_string());
    }

    fn open(name: &str) -> File {
        for _ in 0..200 {
            match OpenOptions::new().read(true).write(true).open(name) {
                Ok(f) => return f,
                Err(_) => thread::sleep(Duration::from_millis(10)),
            }
        }
        panic!("cannot open {name}");
    }

    /// The pipe's owner and access list as the system reports them.
    fn access_of(pipe: &File) -> String {
        use windows_sys::Win32::Security::Authorization::{
            ConvertSecurityDescriptorToStringSecurityDescriptorW, GetSecurityInfo, SE_KERNEL_OBJECT,
        };
        use windows_sys::Win32::Security::{DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION};
        let what = OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
        let mut descriptor = null_mut();
        let failed = unsafe {
            GetSecurityInfo(pipe.as_raw_handle() as _, SE_KERNEL_OBJECT, what, null_mut(), null_mut(), null_mut(), null_mut(), &mut descriptor)
        };
        assert_eq!(failed, 0);
        let (mut text, mut len) = (null_mut(), 0);
        let converted = unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(descriptor, SDDL_REVISION_1, what, &mut text, &mut len)
        };
        assert_ne!(converted, 0);
        let s = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(text, len as usize) });
        unsafe {
            LocalFree(text.cast());
            LocalFree(descriptor);
        }
        s.trim_end_matches('\0').to_string()
    }

    /// `sddl` as the system writes it back, with well-known accounts shown by alias.
    fn normalised(sddl: &str) -> String {
        use windows_sys::Win32::Security::Authorization::{
            ConvertSecurityDescriptorToStringSecurityDescriptorW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        };
        use windows_sys::Win32::Security::{DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION};
        let wide: Vec<u16> = sddl.encode_utf16().chain([0]).collect();
        let mut descriptor = null_mut();
        let parsed = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(wide.as_ptr(), SDDL_REVISION_1, &mut descriptor, null_mut())
        };
        assert_ne!(parsed, 0);
        let what = OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
        let (mut text, mut len) = (null_mut(), 0);
        let converted = unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(descriptor, SDDL_REVISION_1, what, &mut text, &mut len)
        };
        assert_ne!(converted, 0);
        let s = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(text, len as usize) });
        unsafe {
            LocalFree(text.cast());
            LocalFree(descriptor);
        }
        s.trim_end_matches(' ').to_string()
    }

    fn try_ask(pipe: &File, line: &str) -> Option<String> {
        (&*pipe).write_all(format!("{line}\n").as_bytes()).ok()?;
        let mut out = String::new();
        BufReader::new(pipe).read_line(&mut out).ok()?;
        (!out.is_empty()).then_some(out)
    }

    #[test]
    fn the_pipe_serves_lines_and_closes_with_the_setting() {
        let name = format!(r"\\.\pipe\foxden-app-test-{}", std::process::id());
        let server = Server::start(name.clone(), Arc::new(|line: &str| format!("echo {line}"))).unwrap();
        assert!(create(&name, true).is_err());
        let pipe = open(&name);
        assert_eq!(access_of(&pipe), normalised(&format!("O:{0}D:P(A;;FA;;;{0})", user_sid().unwrap())));
        assert_eq!(try_ask(&pipe, "hi").as_deref(), Some("echo hi\n"));
        let long = "x".repeat(MAX_LINE + 10);
        assert!(try_ask(&open(&name), &long).unwrap().contains("too long"));

        let mut idle: Vec<File> = (0..MAX_CLIENTS - 1).map(|_| open(&name)).collect();
        assert_eq!(try_ask(&open(&name), "full"), None);
        idle.clear();
        thread::sleep(IDLE * 2);
        assert_eq!(try_ask(&pipe, "late"), None);
        assert!(try_ask(&open(&name), "again").is_some());

        let held = open(&name);
        server.stop();
        assert_eq!(try_ask(&held, "after"), None);
        let gone = OpenOptions::new().read(true).write(true).open(&name);
        assert!(gone.is_err());
    }
}
