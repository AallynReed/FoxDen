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
        "list_macros" | "save_macro" | "delete_macro" | "play_macro" | "stop_macros" | "set_macros_enabled"
        | "list_macro_presets" | "add_macro_preset" | "list_mouse_macros" | "store_macro_on_mouse"
        | "reset_mouse_button" => {
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
        "set_fan_curve" | "remove_fan_curve" | "set_fan_rule" => fan_edit(app, tool, args, client),
        "set_fan_service" => {
            let on = flag(args, "running")?.ok_or("running must be true or false.")?;
            if on { fans::start_service()? } else { fans::stop_service()? }
            sensors()
        }
        "set_audio" | "route_channel" | "unassign_app" | "set_eq" | "set_processing" | "sound_profile_code"
        | "apply_sound_profile" | "save_sound_profile" | "rename_sound_profile" | "delete_sound_profile"
        | "import_sound_profile" | "get_audio_priority" | "set_audio_priority" | "get_app_volumes" | "set_app_volume"
        | "set_device_volume" | "set_default_device" => audio_tool(app, tool, args),
        "analyze_mic" | "auto_tune_mic" | "undo_auto_tune" => mic_tool(app, tool, args, client),
        "set_hardware_mode" => set_hardware_mode(app, args),
        "reset_layout" => done(commands::reset_layout(app.clone(), shared(app)?), "Every device is back in its first place.".into()),
        "detect_devices" => {
            let jobs = app.try_state::<Jobs>().ok_or("FoxDen is still starting.")?;
            done(commands::detect(jobs), "Detecting hardware.".into())
        }
        "set_zone_leds" => set_zone_leds(app, args, app.try_state::<Jobs>().ok_or("FoxDen is still starting.")?),
        "get_headsets" => headsets(app),
        "set_headset" | "set_headset_eq" => headset_tool(app, tool, args),
        "get_device_info" => {
            let snap = snapshot(app)?;
            let id = device_id(&snap, &text(args, "device")?)?;
            let name = snap["devices"].as_array().into_iter().flatten().find(|d| d["id"] == id.as_str()).map(|d| d["name"].as_str().unwrap_or_default().to_string()).unwrap_or_default();
            Ok(json!(tauri::async_runtime::block_on(crate::device_info::device_info(id, name))?))
        }
        "get_logs" => logs_of(&log::get_logs(), args),
        "get_conflicts" => serde_json::to_value(commands::get_conflicts(app.clone(), shared(app)?)).map_err(|e| e.to_string()),
        "close_conflicts" => close_conflicts(app, args, client),
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
            if d["hardware"]["modes"].as_array().is_some_and(|m| !m.is_empty()) {
                out["hardware_modes"] = d["hardware"]["modes"].clone();
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
const ASKED_SETTINGS: [(&str, &str); 9] = [
    ("autostart", "Start with Windows"),
    ("lighting_module", "Lighting"),
    ("native_drivers", "Native drivers"),
    ("fans_module", "Fans"),
    ("network_enabled", "Lights on this network"),
    ("openrgb_enabled", "Use an OpenRGB server"),
    ("auto_update", "Install updates automatically"),
    ("close_conflicts", "Close conflicting software"),
    ("send_reports", "Send anonymous error reports"),
];

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
fn approve(
    gate: &Gate,
    last: &Mutex<Option<Value>>,
    args: &Value,
    client: &str,
    title: &'static str,
    lines: Vec<String>,
) -> Result<bool, String> {
    let reason = match args["reason"].as_str().map(|r| fan_proposal::clean_text(r, 300)) {
        Some(r) if !r.is_empty() => r,
        _ => return Err("This needs a reason: a sentence for the person approving.".into()),
    };
    let client = fan_proposal::clean_text(client, 40);
    let record = |outcome: &str| *lock(last) = Some(json!({"client": client, "outcome": outcome}));
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
            if !approve(gate, &LAST_MACRO, args, client, crate::ui::approval::SAVE_MACRO, lines)? {
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
            if !approve(gate, &LAST_MACRO, args, client, crate::ui::approval::PLAY_MACRO, lines)? {
                return Ok(json!(fan_proposal::REJECTED));
            }
            m.play(&e.id, Duration::from_secs_f64(delay))?;
            Ok(json!(format!("Playing {} in {delay} s. Pause stops it.", e.macro_.name)))
        }
        "stop_macros" => {
            m.stop_all();
            Ok(json!("Stopped every macro."))
        }
        "list_macro_presets" => Ok(json!(
            foxden_macros::presets::ALL
                .iter()
                .map(|p| json!({"id": p.id, "group": p.group, "name": p.name, "does": p.about}))
                .collect::<Vec<_>>()
        )),
        "add_macro_preset" => {
            let id = text(args, "preset")?;
            let p = foxden_macros::presets::get(&id).ok_or_else(|| format!("No preset {id}. list_macro_presets shows them."))?;
            let hotkey = match args["hotkey"].as_str().map(str::trim) {
                Some(h) if !h.is_empty() => Some(Hotkey::parse(h)?),
                _ => None,
            };
            let mut macro_ = (p.make)();
            let lib = m.library();
            macro_.name = (1..)
                .map(|i| if i == 1 { p.name.to_string() } else { format!("{} {i}", p.name) })
                .find(|n| !lib.entries.iter().any(|e| e.macro_.name.eq_ignore_ascii_case(n)))
                .unwrap_or_default();
            let entry = Entry { id: String::new(), macro_, hotkey, block: true };
            lib.clone().put(entry.clone())?;
            let mut lines = vec![
                format!("Name: {}", entry.macro_.name),
                hotkey.map(|h| format!("Hotkey: {} (hidden from other apps)", h.label())).unwrap_or("No hotkey".into()),
                format!("Mode: {}", entry.macro_.mode.label()),
            ];
            lines.extend(step_lines(&entry.macro_));
            if !approve(gate, &LAST_MACRO, args, client, crate::ui::approval::SAVE_MACRO, lines)? {
                return Ok(json!(fan_proposal::REJECTED));
            }
            let name = entry.macro_.name.clone();
            let id = m.put(entry)?;
            crate::macros::changed(app);
            Ok(json!({"id": id, "saved": name}))
        }
        "list_mouse_macros" => {
            let mm = crate::mouse_macros::get(app)?;
            let mice = mm.scan();
            Ok(json!({
                "mice": mice.iter().map(|x| json!({
                    "name": x.name,
                    "buttons": x.buttons.iter().map(|b| json!({
                        "button": b.slot + 1,
                        "factory": b.factory,
                        "macro": b.macro_name,
                    })).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
                "note": crate::mouse_macros::NOTE,
            }))
        }
        "store_macro_on_mouse" | "reset_mouse_button" => {
            let mm = crate::mouse_macros::get(app)?;
            let button = args["button"].as_u64().filter(|b| (1..=16).contains(b)).ok_or("button must be a number from 1 to 16.")?;
            let slot = (button - 1) as u8;
            let store = match tool {
                "store_macro_on_mouse" => {
                    let e = find("id")?;
                    crate::mouse_macros::ops(&e.macro_)?;
                    Some(e.macro_)
                }
                _ => None,
            };
            let mice = mm.scan();
            let mouse = mice.first().ok_or("No mouse that keeps macros in its own memory is connected.")?;
            let b = mouse.buttons.get(slot as usize).ok_or_else(|| format!("{} has no button {button}.", mouse.name))?;
            let mut lines = vec![format!("{}: {}", mouse.name, b.label)];
            match &store {
                Some(macro_) => {
                    lines.push(format!("Store \"{}\" there", macro_.name));
                    lines.extend(step_lines(macro_));
                }
                None => lines.push(format!("Put back to: {}", b.factory)),
            }
            if !approve(gate, &LAST_MACRO, args, client, crate::ui::approval::MOUSE_MACRO, lines)? {
                return Ok(json!(fan_proposal::REJECTED));
            }
            let done = match store {
                Some(macro_) => mm.store("", slot, &macro_)?,
                None => mm.reset("", slot)?,
            };
            crate::macros::changed(app);
            Ok(json!(done))
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
                if !approve(gate, &LAST_MACRO, args, client, crate::ui::approval::MACROS_ON, lines)? {
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

// ---- Audio in full: routing, EQ, processing, sound profiles, priority and Windows volumes

const SIDES: [&str; 3] = ["game", "media", "aux"];
const EQ_PRESETS: [&str; 4] = ["flat", "bass_boost", "voice_clarity", "treble"];
const BLOCKS: [&str; 5] = ["denoise", "agc", "comp", "deesser", "limiter"];
const MIC_BLOCKS: [&str; 3] = ["denoise", "agc", "deesser"];

fn chan_cfg<'a>(v: &'a Value, id: &str) -> Result<&'a Value, String> {
    v["config"]["channels"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|c| c["channel"] == id)
        .ok_or_else(|| format!("The mixer has no {id} channel."))
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)).collect()
}

fn name_of(id: &str) -> &'static str {
    CHANNELS.iter().find(|x| x.1 == id).map_or("", |x| x.0)
}

/// Processing as an AI sees it: outputs and the master keep only the compressor and limiter.
fn proc_view(p: &Value, mic: bool, config: &Value) -> Value {
    let mut out = json!({"comp": p["comp"], "limiter": p["limiter"]});
    if mic {
        for key in MIC_BLOCKS {
            out[key] = p[key].clone();
        }
        out["order"] = p["order"].clone();
        out["gate"] = json!({
            "on": config["gate_on"], "threshold_db": config["gate_db"],
            "attack_ms": p["gate"]["attack_ms"], "hold_ms": p["gate"]["hold_ms"], "release_ms": p["gate"]["release_ms"],
        });
    }
    out
}

fn audio_of(v: &Value) -> Value {
    let config = &v["config"];
    let channels: Vec<Value> = config["channels"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| {
            let mic = c["channel"] == "chatCapture";
            json!({
                "channel": name_of(c["channel"].as_str().unwrap_or_default()), "on": c["on"],
                "volume": percent(&c["volume"]), "muted": c["muted"],
                "source": c["source"], "source_is_output": c["loopback"], "apps": c["apps"],
                "eq": {"on": c["eq_on"], "bands": c["eq"]},
                "processing": proc_view(&c["processing"], mic, config),
            })
        })
        .collect();
    let apps: Vec<&Value> = v["apps"].as_array().into_iter().flatten().map(|a| &a["exe"]).collect();
    let profiles: Vec<Value> = v["profiles"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| json!({"name": p["name"], "kind": p["kind"], "builtin": p["builtin"]}))
        .collect();
    let filters: Vec<&Value> = v["filters"].as_array().into_iter().flatten().map(|f| &f["id"]).collect();
    let sides: Vec<&str> = config["game_side"].as_array().into_iter().flatten().filter_map(|s| s.as_str()).collect();
    json!({
        "running": v["running"], "output": config["output"], "playing_on": v["playing_on"],
        "park": config["park"], "parked_on": v["parked_on"],
        "master": percent(&config["master"]), "master_muted": config["master_muted"],
        "master_processing": proc_view(&config["master_processing"], false, config),
        "chat_mix": config["chat_mix"].as_f64().map(|b| (b * 100.0).round()),
        "game_side": sides,
        "monitor": {"on": config["monitor"], "volume": percent(&config["monitor_volume"])},
        "auto_assign": config["auto_assign"], "auto_assign_skips": config["no_auto"],
        "virtual_mic": {"on": config["virtual_mic"], "installed": v["vmic"]["installed"], "ready": v["vmic"]["ready"], "playing": v["vmic"]["playing"]},
        "channels": channels, "outputs": v["outputs"], "inputs": v["inputs"], "apps_with_sound": apps,
        "sound_profiles": profiles, "eq_filters": filters, "eq_presets": EQ_PRESETS,
        "noise_removal": foxden_mixer::denoise::status(),
        "problem": v["error"],
        "note": "The mic chain runs noise removal, then the gate, then the stabilizer (agc), compressor, de-esser and EQ in `order`, then the limiter. High pass is an EQ band with filter HighPass.",
    })
}

fn audio(app: &AppHandle) -> Reply {
    let mut v = audio_of(&mixer_view(app)?);
    v["last_recording_request"] = json!(lock(&LAST_MIC).clone());
    v["auto_tune_undo"] = json!(crate::mic_check::can_undo());
    Ok(v)
}

fn flag(args: &Value, key: &str) -> Result<Option<bool>, String> {
    match &args[key] {
        Value::Null => Ok(None),
        Value::Bool(b) => Ok(Some(*b)),
        _ => Err(format!("{key} must be true or false.")),
    }
}

fn ranged(args: &Value, key: &str, lo: f64, hi: f64) -> Result<Option<f64>, String> {
    if args[key].is_null() {
        return Ok(None);
    }
    let n = number(args, key)?;
    if !(lo..=hi).contains(&n) {
        return Err(format!("{key} must be from {lo} to {hi}."));
    }
    Ok(Some(n))
}

/// The mixer changes `set_audio` asks for, checked against the mixer's view.
fn audio_changes(args: &Value, v: &Value) -> Result<Vec<Value>, String> {
    let config = &v["config"];
    let mut out = Vec::new();
    if let Some(name) = args["output"].as_str() {
        if !strings(&v["outputs"]).iter().any(|o| o == name) {
            return Err(format!("No output named {name}; get_audio lists outputs."));
        }
        out.push(json!({"kind": "output", "name": name}));
    }
    if let Some(name) = args["park"].as_str() {
        if !name.is_empty() && !strings(&v["outputs"]).iter().any(|o| o == name) {
            return Err(format!("No output named {name}; leave park empty to let FoxDen pick."));
        }
        out.push(json!({"kind": "park", "name": name}));
    }
    if let Some(m) = ranged(args, "master", 0.0, 100.0)? {
        out.push(json!({"kind": "master", "volume": m / 100.0}));
    }
    if let Some(m) = flag(args, "master_muted")? {
        out.push(json!({"kind": "master_mute", "muted": m}));
    }
    if let Some(b) = ranged(args, "chat_mix", -100.0, 100.0)? {
        out.push(json!({"kind": "chat_mix", "balance": b / 100.0}));
    }
    if !args["game_side"].is_null() {
        let list = strings(&args["game_side"]);
        if list.iter().any(|s| !SIDES.contains(&s.as_str())) || !args["game_side"].is_array() {
            return Err("game_side takes game, media and aux.".into());
        }
        out.push(json!({"kind": "mix_sides", "channels": list}));
    }
    if let Some(on) = flag(args, "running")? {
        out.push(json!({"kind": "run", "on": on}));
    }
    if let Some(on) = flag(args, "auto_assign")? {
        out.push(json!({"kind": "auto_assign", "on": on}));
    }
    if let Some(exe) = args["allow_auto_assign"].as_str() {
        out.push(json!({"kind": "allow_auto", "exe": exe}));
    }
    let monitor = flag(args, "monitor")?;
    let monitor_volume = ranged(args, "monitor_volume", 0.0, 100.0)?;
    if monitor.is_some() || monitor_volume.is_some() {
        out.push(json!({
            "kind": "monitor",
            "on": monitor.unwrap_or(config["monitor"].as_bool().unwrap_or(false)),
            "volume": monitor_volume.map_or(config["monitor_volume"].as_f64().unwrap_or(0.5), |m| m / 100.0),
        }));
    }
    if let Some(on) = flag(args, "virtual_mic")? {
        if on && v["vmic"]["installed"] != true {
            return Err("The virtual mic needs VB-CABLE, which is set up in FoxDen's Audio tab.".into());
        }
        out.push(json!({"kind": "virtual_mic", "on": on}));
    }
    if out.is_empty() {
        return Err("Pass at least one setting to change.".into());
    }
    Ok(out)
}

fn apply_all(app: &AppHandle, changes: Vec<Value>) -> Reply {
    for c in changes {
        mix(app, c)?;
    }
    audio(app)
}

fn route_change(args: &Value, v: &Value) -> Result<Value, String> {
    let (id, label) = channel(args)?;
    let c = chan_cfg(v, id)?;
    let source = match args["source"].as_str() {
        Some(s) => s.trim().to_string(),
        None => c["source"].as_str().unwrap_or_default().to_string(),
    };
    let (outputs, inputs) = (strings(&v["outputs"]), strings(&v["inputs"]));
    let is_out = outputs.contains(&source);
    let is_in = inputs.contains(&source);
    let loopback = match flag(args, "source_is_output")? {
        Some(l) => l,
        None if !args["source"].is_null() => is_out && !is_in,
        None => c["loopback"].as_bool().unwrap_or(false),
    };
    if !source.is_empty() && !(if loopback { is_out } else { is_in }) {
        let list = if loopback { "outputs" } else { "inputs" };
        return Err(format!("{source} is not among the {list} get_audio lists."));
    }
    let has_apps = !strings(&c["apps"]).is_empty();
    let on = flag(args, "on")?.unwrap_or(!source.is_empty() || has_apps);
    if on && source.is_empty() && !has_apps {
        return Err(format!("{label} needs a source or an app to turn on."));
    }
    Ok(json!({"kind": "route", "channel": id, "source": source, "loopback": loopback, "on": on}))
}

fn unassign_change(args: &Value, v: &Value) -> Result<Value, String> {
    let exe = text(args, "app")?;
    for (_, id, _) in &CHANNELS[..4] {
        let apps = strings(&chan_cfg(v, id)?["apps"]);
        if apps.iter().any(|a| a.eq_ignore_ascii_case(&exe)) {
            let left: Vec<&String> = apps.iter().filter(|a| !a.eq_ignore_ascii_case(&exe)).collect();
            return Ok(json!({"kind": "apps", "channel": id, "apps": left}));
        }
    }
    Err(format!("{exe} is on no channel."))
}

fn band_of(v: &Value, base: &Value, filters: &[String]) -> Result<Value, String> {
    let mut b = if base.is_object() { base.clone() } else { json!({"enabled": true, "filter": "PeakingEQ", "frequency": 1000.0, "gain_db": 0.0, "q": 1.414}) };
    let given = v.as_object().ok_or("Each band is an object.")?;
    for (key, x) in given {
        match key.as_str() {
            "enabled" => b[key] = json!(x.as_bool().ok_or("enabled must be true or false.")?),
            "filter" => {
                let f = x.as_str().unwrap_or_default();
                if !filters.iter().any(|g| g == f) {
                    return Err(format!("filter must be one of {}.", filters.join(", ")));
                }
                b[key] = json!(f);
            }
            "frequency" | "gain_db" | "q" => {
                let (lo, hi) = match key.as_str() {
                    "frequency" => (20.0, 20000.0),
                    "gain_db" => (-24.0, 24.0),
                    _ => (0.1, 10.0),
                };
                let n = x.as_f64().filter(|n| (lo..=hi).contains(n)).ok_or_else(|| format!("{key} must be from {lo} to {hi}."))?;
                b[key] = json!(n);
            }
            "index" => {}
            _ => return Err(format!("A band has no {key}; it takes enabled, filter, frequency, gain_db and q.")),
        }
    }
    Ok(b)
}

fn eq_change(args: &Value, v: &Value) -> Result<Value, String> {
    let (id, _) = channel(args)?;
    let c = chan_cfg(v, id)?;
    let filters = strings(&json!(v["filters"].as_array().into_iter().flatten().map(|f| f["id"].clone()).collect::<Vec<_>>()));
    let mut bands: Vec<Value> = c["eq"].as_array().cloned().unwrap_or_default();
    let mut enabled = flag(args, "enabled")?;
    if let Some(p) = args["preset"].as_str() {
        let n = EQ_PRESETS.iter().position(|x| *x == p).ok_or_else(|| format!("preset must be one of {}.", EQ_PRESETS.join(", ")))?;
        bands = crate::ui::audio::FREQS
            .iter()
            .zip(crate::ui::audio::PRESETS[n])
            .map(|(fq, g)| json!({"enabled": true, "filter": "PeakingEQ", "frequency": fq, "gain_db": g, "q": 1.414}))
            .collect();
        enabled = enabled.or(Some(n != 0 || c["eq_on"] == true));
    }
    if let Some(list) = args["bands"].as_array() {
        if list.is_empty() || list.len() > foxden_mixer::dsp::MAX_BANDS {
            return Err(format!("bands takes 1 to {} bands.", foxden_mixer::dsp::MAX_BANDS));
        }
        bands = list.iter().map(|b| band_of(b, &Value::Null, &filters)).collect::<Result<_, _>>()?;
    }
    if let Some(list) = args["band_changes"].as_array() {
        for b in list {
            let i = b["index"].as_u64().map(|i| i as usize).filter(|i| *i < bands.len())
                .ok_or_else(|| format!("Each band change needs an index from 0 to {}.", bands.len().saturating_sub(1)))?;
            bands[i] = band_of(b, &bands[i], &filters)?;
        }
    }
    if enabled.is_none() && args["preset"].is_null() && args["bands"].is_null() && args["band_changes"].is_null() {
        return Err("Pass enabled, preset, bands or band_changes.".into());
    }
    Ok(json!({"kind": "eq", "channel": id, "enabled": enabled.unwrap_or(c["eq_on"] == true), "bands": bands}))
}

/// Lays `given` over `current` field by field, refusing names and types `current` lacks.
fn overlay(current: &mut Value, given: &Value, what: &str) -> Result<(), String> {
    let given = given.as_object().ok_or_else(|| format!("{what} must be an object."))?;
    for (key, x) in given {
        let Some(now) = current.get(key) else {
            let names: Vec<&String> = current.as_object().map(|o| o.keys().collect()).unwrap_or_default();
            return Err(format!("{what} has no {key}; it takes {}.", names.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")));
        };
        let same = match now {
            Value::Bool(_) => x.is_boolean(),
            Value::Number(_) => x.as_f64().is_some_and(f64::is_finite),
            Value::String(_) => x.is_string(),
            _ => false,
        };
        if !same {
            return Err(format!("{what}.{key} must be like {now}."));
        }
        current[key] = x.clone();
    }
    Ok(())
}

fn proc_changes(args: &Value, v: &Value) -> Result<Vec<Value>, String> {
    let asked = args["channel"].as_str().unwrap_or_default().trim().to_lowercase();
    let (id, mic) = if asked == "master" {
        (None, false)
    } else {
        let (id, _) = channel(args)?;
        (Some(id), id == "chatCapture")
    };
    let config = &v["config"];
    let mut p = match id {
        Some(id) => chan_cfg(v, id)?["processing"].clone(),
        None => config["master_processing"].clone(),
    };
    let mut out = Vec::new();
    let mut any = false;
    for key in BLOCKS {
        if args[key].is_null() {
            continue;
        }
        if !mic && MIC_BLOCKS.contains(&key) {
            return Err(format!("{key} is for the mic; outputs and the master take comp and limiter."));
        }
        if key == "denoise" && !args[key]["engine"].is_null() {
            let e = args[key]["engine"].as_str().unwrap_or_default();
            if !["light", "strong", "auto"].contains(&e) {
                return Err("denoise.engine must be light, strong or auto.".into());
            }
        }
        overlay(&mut p[key], &args[key], key)?;
        any = true;
    }
    if !args["order"].is_null() {
        if !mic {
            return Err("order is for the mic.".into());
        }
        let order: Vec<String> = strings(&args["order"]);
        let all = ["agc", "comp", "deesser", "eq"];
        if order.len() != 4 || all.iter().any(|s| !order.iter().any(|o| o == s)) {
            return Err("order lists agc, comp, deesser and eq, each once.".into());
        }
        p["order"] = json!(order);
        any = true;
    }
    if !args["gate"].is_null() {
        if !mic {
            return Err("The gate is for the mic.".into());
        }
        let g = &args["gate"];
        let mut timing = g.clone();
        if let Some(o) = timing.as_object_mut() {
            o.remove("on");
            o.remove("threshold_db");
        }
        if timing.as_object().is_some_and(|o| !o.is_empty()) {
            overlay(&mut p["gate"], &timing, "gate")?;
            any = true;
        }
        let on = flag(g, "on")?;
        let db = ranged(g, "threshold_db", -90.0, 0.0)?;
        if on.is_some() || db.is_some() {
            out.push(json!({
                "kind": "gate",
                "on": on.unwrap_or(config["gate_on"].as_bool().unwrap_or(false)),
                "threshold_db": db.unwrap_or(config["gate_db"].as_f64().unwrap_or(-50.0)),
            }));
        }
    }
    if any {
        let clean: foxden_mixer::chain::Processing = serde_json::from_value(p).map_err(|e| e.to_string())?;
        out.insert(0, json!({"kind": "processing", "channel": id, "processing": clean.clean(mic)}));
    }
    if out.is_empty() {
        return Err("Pass a block to change: denoise, gate, agc, comp, deesser, limiter or order.".into());
    }
    Ok(out)
}

fn profile_named<'a>(v: &'a Value, name: &str) -> Result<&'a Value, String> {
    v["profiles"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|p| p["name"] == name)
        .ok_or_else(|| format!("No sound profile named {name}; get_audio lists sound_profiles."))
}

fn sound_profile_change(tool: &str, args: &Value, v: &Value) -> Result<Value, String> {
    match tool {
        "apply_sound_profile" => {
            let (id, label) = channel(args)?;
            let p = profile_named(v, &text(args, "name")?)?;
            let kind = if id == "chatCapture" { "mic" } else { "output" };
            if p["kind"] != kind {
                return Err(format!("That is a{} profile; {label} takes {kind} profiles.", if kind == "mic" { "n output" } else { " mic" }));
            }
            Ok(json!({"kind": "apply_profile", "channel": id, "profile": p}))
        }
        "save_sound_profile" => {
            let (id, _) = channel(args)?;
            let name = crate::profiles::clean_name(&text(args, "name")?);
            Ok(json!({"kind": "save_profile", "channel": id, "name": name}))
        }
        "rename_sound_profile" => {
            let name = text(args, "name")?;
            if profile_named(v, &name)?["builtin"] == true {
                return Err("Built-in profiles keep their names.".into());
            }
            Ok(json!({"kind": "rename_profile", "from": name, "to": crate::profiles::clean_name(&text(args, "to")?)}))
        }
        "delete_sound_profile" => {
            let name = text(args, "name")?;
            if profile_named(v, &name)?["builtin"] == true {
                return Err("Built-in profiles cannot be deleted.".into());
            }
            Ok(json!({"kind": "delete_profile", "name": name}))
        }
        _ => {
            let p = crate::profiles::sound_decode(text(args, "code")?)?;
            Ok(json!({"kind": "add_profile", "profile": p}))
        }
    }
}

fn priority_config(args: &Value, now: &Value) -> Result<crate::priority::PriorityConfig, String> {
    let mut c = now["config"].clone();
    for key in ["on", "mixer", "windows"] {
        if let Some(b) = flag(args, key)? {
            c[key] = json!(b);
        }
    }
    for (key, choices) in [("outputs", "choices_out"), ("inputs", "choices_in")] {
        let Some(list) = args[key].as_array() else { continue };
        let known: Vec<&Value> = now[choices].as_array().into_iter().flatten().chain(now["config"][key].as_array().into_iter().flatten()).collect();
        let mut out = Vec::new();
        for want in list {
            let w = want.as_str().unwrap_or_default();
            let e = known
                .iter()
                .find(|e| e["id"] == w || e["name"] == w)
                .ok_or_else(|| format!("{w} is not a connected device or already in the list; get_audio_priority lists them."))?;
            out.push(json!({"id": e["id"], "name": e["name"]}));
        }
        c[key] = json!(out);
    }
    serde_json::from_value(c).map_err(|e| e.to_string())
}

fn volume_rows(app: &AppHandle) -> Reply {
    serde_json::to_value(crate::volumes::get_volumes(mixer_state(app)?)?).map_err(|e| e.to_string())
}

fn find_row<'a>(rows: &'a Value, list: &str, wanted: &str, key: &str) -> Result<&'a Value, String> {
    rows[list]
        .as_array()
        .into_iter()
        .flatten()
        .find(|r| r[key].as_str().is_some_and(|k| k.eq_ignore_ascii_case(wanted)) || r["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(wanted)))
        .ok_or_else(|| format!("{wanted} is not in get_app_volumes' {list}."))
}

fn audio_tool(app: &AppHandle, tool: &str, args: &Value) -> Reply {
    match tool {
        "set_audio" => apply_all(app, audio_changes(args, &mixer_view(app)?)?),
        "route_channel" => apply_all(app, vec![route_change(args, &mixer_view(app)?)?]),
        "unassign_app" => apply_all(app, vec![unassign_change(args, &mixer_view(app)?)?]),
        "set_eq" => apply_all(app, vec![eq_change(args, &mixer_view(app)?)?]),
        "set_processing" => apply_all(app, proc_changes(args, &mixer_view(app)?)?),
        "sound_profile_code" => {
            let v = mixer_view(app)?;
            let p = serde_json::from_value(profile_named(&v, &text(args, "name")?)?.clone()).map_err(|e| e.to_string())?;
            Ok(json!(crate::profiles::sound_code(p)))
        }
        "apply_sound_profile" | "save_sound_profile" | "rename_sound_profile" | "delete_sound_profile" | "import_sound_profile" => {
            apply_all(app, vec![sound_profile_change(tool, args, &mixer_view(app)?)?])
        }
        "get_audio_priority" => serde_json::to_value(crate::priority::get_priority(app.clone())?).map_err(|e| e.to_string()),
        "set_audio_priority" => {
            let now = serde_json::to_value(crate::priority::get_priority(app.clone())?).map_err(|e| e.to_string())?;
            let c = priority_config(args, &now)?;
            serde_json::to_value(crate::priority::set_priority(app.clone(), mixer_state(app)?, c)?).map_err(|e| e.to_string())
        }
        "get_app_volumes" => volume_rows(app),
        "set_app_volume" | "set_device_volume" => {
            let volume = ranged(args, "volume", 0.0, 100.0)?.map(|v| (v / 100.0) as f32);
            let muted = flag(args, "muted")?;
            if volume.is_none() && muted.is_none() {
                return Err("Pass volume or muted.".into());
            }
            let rows = volume_rows(app)?;
            let out = if tool == "set_app_volume" {
                let r = find_row(&rows, "apps", &text(args, "app")?, "exe")?;
                let exe = r["exe"].as_str().unwrap_or_default().to_string();
                let endpoint = r["endpoint"].as_str().unwrap_or_default().to_string();
                crate::volumes::set_app_volume(mixer_state(app)?, exe, endpoint, volume, muted)?
            } else {
                let r = find_row(&rows, "devices", &text(args, "device")?, "id")?;
                crate::volumes::set_device_volume(mixer_state(app)?, r["id"].as_str().unwrap_or_default().into(), volume, muted)?
            };
            serde_json::to_value(out).map_err(|e| e.to_string())
        }
        _ => {
            let rows = volume_rows(app)?;
            let r = find_row(&rows, "devices", &text(args, "device")?, "id")?;
            let out = crate::volumes::make_default_device(mixer_state(app)?, r["id"].as_str().unwrap_or_default().into())?;
            serde_json::to_value(out).map_err(|e| e.to_string())
        }
    }
}

// ---- Mic samples: every recording is approved on screen, each time

static LAST_MIC: Mutex<Option<Value>> = Mutex::new(None);

type Recorder<'a> = &'a dyn Fn(&crate::mic_check::MicSetup, f64) -> Result<(Vec<f32>, u32), String>;

/// What the mic tools need from the mixer, so tests can stand in for it.
struct MicHooks<'a> {
    setup: &'a dyn Fn() -> Result<crate::mic_check::MicSetup, String>,
    record: Recorder<'a>,
    commit: &'a dyn Fn(&crate::mic_check::MicSetup, &crate::mic_check::Tuning) -> Result<(), String>,
}

fn mic_tool(app: &AppHandle, tool: &str, args: &Value, client: &str) -> Reply {
    if tool == "undo_auto_tune" {
        let old = crate::mic_check::undo(app)?;
        return Ok(json!({"restored": old}));
    }
    let who = match fan_proposal::clean_text(client, 40) {
        c if c.is_empty() => "An AI assistant".to_string(),
        c => c,
    };
    let show = |ask: fan_proposal::Ask| {
        log::info("ai", format!("{who} asked to record the microphone; waiting for approval"));
        let a = app.clone();
        on_main(app, Duration::from_secs(10), move || crate::ui::approval::open(&a, ask))?
    };
    let close = |id| {
        let _ = app.run_on_main_thread(move || crate::ui::approval::close(id));
    };
    let gate = Gate { desk: &DESK, show: &show, close: &close, wait: fan_proposal::WAIT };
    let setup = || crate::mic_check::current(app);
    let commit = |old: &crate::mic_check::MicSetup, t: &crate::mic_check::Tuning| crate::mic_check::commit(app, old, t);
    let hooks = MicHooks { setup: &setup, record: &crate::mic_check::record, commit: &commit };
    mic_tool_with(tool, args, client, &gate, &hooks)
}

fn mic_tool_with(tool: &str, args: &Value, client: &str, gate: &Gate, hooks: &MicHooks) -> Reply {
    use crate::mic_check::{MAX_SECONDS, MIN_SECONDS};
    let default = if tool == "auto_tune_mic" { 6.0 } else { 5.0 };
    let seconds = ranged(args, "seconds", MIN_SECONDS, MAX_SECONDS)?.unwrap_or(default);
    let target = if tool == "auto_tune_mic" {
        let t = args["target"].as_str().unwrap_or_default();
        Some(serde_json::from_value::<foxden_mixer::tune::Target>(json!(t)).map_err(|_| {
            "target must be discord, streaming, podcast, gaming, meeting or music.".to_string()
        })?)
    } else {
        None
    };
    let mono = match args["delivery"].as_str() {
        None | Some("stereo") => false,
        Some("mono") => true,
        Some(_) => return Err("delivery must be stereo or mono.".into()),
    };
    let setup = (hooks.setup)()?;
    if setup.source.is_empty() {
        return Err("Pick a microphone for the Mic channel first.".into());
    }
    let what = match target {
        Some(_) => "then works out and applies new mic settings; Undo in the Audio tab puts the old ones back",
        None => "then measures it",
    };
    let lines = vec![
        format!("Records {} seconds from {}.", seconds, setup.source),
        format!("FoxDen keeps the sound in memory only, {what}, and deletes it at once. Nothing is saved or sent."),
    ];
    if !approve(gate, &LAST_MIC, args, client, crate::ui::approval::RECORD_MIC, lines)? {
        return Err("The person at this PC did not allow the recording; nothing was recorded.".into());
    }
    let (raw, rate) = (hooks.record)(&setup, seconds)?;
    let out = match target {
        None => crate::mic_check::measure(&raw, rate, &setup),
        Some(target) => {
            let t = crate::mic_check::work_out(&raw, rate, &setup, target, mono);
            (hooks.commit)(&setup, &t)?;
            let mut r = crate::mic_check::report(&t);
            r["applied"] = json!(true);
            r["undo"] = json!("undo_auto_tune puts the previous mic chain back.");
            r
        }
    };
    drop(raw);
    Ok(out)
}

// ---- Fans: edits of one curve or rule, approved like a whole proposal

fn fan_edit(app: &AppHandle, tool: &str, args: &Value, client: &str) -> Reply {
    let state = fans::get_fans()?;
    let v = fan_proposal::view(&state, Value::Null);
    let proposal = fan_edit_proposal(tool, args, &v)?;
    propose(app, &proposal, client)
}

/// The whole config `propose_fan_curves` takes, with one curve or rule changed.
fn fan_edit_proposal(tool: &str, args: &Value, v: &Value) -> Result<Value, String> {
    let mut curves: Vec<Value> = v["curves"].as_array().cloned().unwrap_or_default();
    let mut rules: Vec<Value> = v["rules"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| json!({"fan": r["fan"], "curve": r["curve"], "min_duty": r["min_duty"], "max_duty": r["max_duty"], "ramp": r["ramp"]}))
        .collect();
    match tool {
        "set_fan_curve" => {
            let name = text(args, "name")?;
            let old = curves.iter().position(|c| c["name"] == name.as_str());
            let mut c = old.map(|i| curves[i].clone()).unwrap_or_else(|| json!({"name": name, "hysteresis": 2.0}));
            for key in ["sensors", "points", "hysteresis"] {
                if !args[key].is_null() {
                    c[key] = args[key].clone();
                }
            }
            if c["sensors"].is_null() || c["points"].is_null() {
                return Err("A new curve needs sensors and points.".into());
            }
            match old {
                Some(i) => curves[i] = c,
                None => curves.push(c),
            }
        }
        "remove_fan_curve" => {
            let name = text(args, "name")?;
            let before = curves.len();
            curves.retain(|c| c["name"] != name.as_str());
            if curves.len() == before {
                return Err(format!("No curve named {name}; get_fan_config lists them."));
            }
            if let Some(r) = rules.iter().find(|r| r["curve"] == name.as_str()) {
                return Err(format!("{} follows {name}; give it another curve first with set_fan_rule.", r["fan"]));
            }
        }
        _ => {
            let fan = text(args, "fan")?;
            if !v["fans"].as_array().into_iter().flatten().any(|f| f["id"] == fan.as_str()) {
                return Err(format!("No fan {fan}; get_fan_config lists fan ids."));
            }
            let at = rules.iter().position(|r| r["fan"] == fan.as_str());
            if args["curve"].is_null() && args.get("curve").is_some() {
                if let Some(i) = at {
                    rules.remove(i);
                }
            } else {
                let mut r = at.map(|i| rules[i].clone()).unwrap_or_else(|| json!({"fan": fan, "min_duty": 0.0, "max_duty": 100.0, "ramp": 20.0}));
                for key in ["curve", "min_duty", "max_duty", "ramp"] {
                    if !args[key].is_null() {
                        r[key] = args[key].clone();
                    }
                }
                if r["curve"].is_null() {
                    return Err("A fan without a rule needs a curve.".into());
                }
                match at {
                    Some(i) => rules[i] = r,
                    None => rules.push(r),
                }
            }
        }
    }
    Ok(json!({"curves": curves, "rules": rules, "reason": args["reason"]}))
}

// ---- Devices

fn set_hardware_mode(app: &AppHandle, args: &Value) -> Reply {
    let snap = snapshot(app)?;
    let id = device_id(&snap, &text(args, "device")?)?;
    let d = snap["devices"].as_array().into_iter().flatten().find(|d| d["id"] == id.as_str()).cloned().unwrap_or_default();
    let modes = d["hardware"]["modes"].as_array().cloned().unwrap_or_default();
    if modes.is_empty() {
        return Err(format!("{id} has no effects of its own."));
    }
    let setting = match &args["mode"] {
        Value::Null if args.get("mode").is_some() => None,
        m => {
            let name = m.as_str().map(str::to_string).or_else(|| d["hardware_setting"]["mode"].as_str().map(str::to_string)).ok_or("Pass a mode.")?;
            let mode = modes.iter().find(|x| x["id"] == name.as_str()).ok_or_else(|| {
                let ids: Vec<&str> = modes.iter().filter_map(|x| x["id"].as_str()).collect();
                format!("{id} has no mode {name}; it has {}.", ids.join(", "))
            })?;
            let mut base = d["hardware_setting"].clone();
            if !base.is_object() || base["mode"] != name.as_str() {
                base = json!({"mode": name});
            }
            for key in ["speed", "brightness", "direction", "random"] {
                if !args[key].is_null() {
                    base[key] = args[key].clone();
                }
            }
            if let Some(list) = args["colors"].as_array() {
                let colors: Vec<Value> = list.iter().map(|c| color("colors", c)).collect::<Result<_, _>>()?;
                base["colors"] = json!(colors);
            }
            let s = crate::ui::devices::hardware_values(mode, Some(&base));
            Some(serde_json::from_value(s).map_err(|e| e.to_string())?)
        }
    };
    let own = setting.is_some();
    commands::set_hardware(app.clone(), shared(app)?, id.clone(), setting)?;
    Ok(json!(if own { format!("{id} runs its own effect.") } else { format!("{id} follows FoxDen again.") }))
}

fn set_zone_leds(app: &AppHandle, args: &Value, jobs: State<'static, Jobs>) -> Reply {
    let snap = snapshot(app)?;
    let key = text(args, "zone")?;
    let z = snap["devices"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|d| d["zones"].as_array().into_iter().flatten())
        .find(|z| z["key"] == key.as_str())
        .cloned()
        .ok_or_else(|| format!("No resizable zone {key}; get_status lists zones by key."))?;
    let n = number(args, "leds")? as u64;
    let (lo, hi, step) = (z["min"].as_u64().unwrap_or(0), z["max"].as_u64().unwrap_or(0), z["step"].as_u64().unwrap_or(1).max(1));
    if !(lo..=hi).contains(&n) || !n.is_multiple_of(step) {
        return Err(format!("{key} takes {lo} to {hi} LEDs in steps of {step}."));
    }
    let s = shared(app)?;
    let mut settings = lock(&s).config.settings.clone();
    settings.zone_leds.insert(key.clone(), n as usize);
    commands::set_settings(app.clone(), s, jobs, settings)?;
    Ok(json!(format!("{key} has {n} LEDs.")))
}

fn headsets(app: &AppHandle) -> Reply {
    let list = tauri::async_runtime::block_on(crate::headset::headset_list(shared(app)?))?;
    serde_json::to_value(list).map_err(|e| e.to_string())
}

fn headset_tool(app: &AppHandle, tool: &str, args: &Value) -> Reply {
    let list = headsets(app)?;
    let wanted = text(args, "headset")?;
    let h = list
        .as_array()
        .into_iter()
        .flatten()
        .find(|h| h["key"] == wanted.as_str() || h["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(&wanted)))
        .ok_or_else(|| format!("No headset {wanted}; get_headsets lists them."))?;
    let key = h["key"].as_str().unwrap_or_default().to_string();
    if tool == "set_headset_eq" {
        let eq = &h["eq"];
        if !eq.is_object() {
            return Err("This headset has no EQ.".into());
        }
        let (lo, hi) = (eq["min"].as_i64().unwrap_or(0), eq["max"].as_i64().unwrap_or(0));
        let bands: Vec<i32> = args["bands"]
            .as_array()
            .ok_or("bands must be a list of whole numbers.")?
            .iter()
            .map(|b| b.as_i64().filter(|b| (lo..=hi).contains(b)).map(|b| b as i32).ok_or_else(|| format!("Each band is a whole number from {lo} to {hi}.")))
            .collect::<Result<_, _>>()?;
        tauri::async_runtime::block_on(crate::headset::headset_eq(shared(app)?, key, bands))?;
        return Ok(json!("EQ set."));
    }
    let control = text(args, "control")?;
    let c = h["controls"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|c| c["id"] == control.as_str())
        .ok_or_else(|| format!("This headset has no {control}; get_headsets lists its controls."))?;
    let value = number(args, "value")? as i64;
    let shape = &c["shape"];
    let n = |k: &str| shape[k].as_i64().unwrap_or(0);
    let fits = match shape["type"].as_str() {
        Some("switch") => (0..=1).contains(&value),
        Some("steps") => (n("min")..=n("max")).contains(&value),
        Some("minutes") => (0..=n("max")).contains(&value),
        Some("choice") => shape["options"].as_array().into_iter().flatten().any(|o| o[0].as_i64() == Some(value)),
        _ => false,
    };
    if !fits {
        return Err(format!("{value} does not fit {control}: {shape}."));
    }
    let id: foxden_headset::Control = serde_json::from_value(json!(control)).map_err(|e| e.to_string())?;
    tauri::async_runtime::block_on(crate::headset::headset_set(shared(app)?, key, id, value as i32))?;
    Ok(json!(format!("{control} set to {value}.")))
}

// ---- Logs and conflicts

fn logs_of(entries: &[log::Entry], args: &Value) -> Result<Value, String> {
    let level = args["level"].as_str();
    if level.is_some_and(|l| !["info", "warn", "error"].contains(&l)) {
        return Err("level must be info, warn or error.".into());
    }
    let rank = |l: &str| ["info", "warn", "error"].iter().position(|x| *x == l).unwrap_or(0);
    let floor = level.map_or(0, rank);
    let source = args["source"].as_str().map(str::to_lowercase);
    let limit = args["limit"].as_u64().unwrap_or(100).clamp(1, 1000) as usize;
    let kept: Vec<Value> = entries
        .iter()
        .filter(|e| {
            let l = serde_json::to_value(e.level).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
            rank(&l) >= floor && source.as_ref().is_none_or(|s| e.source.to_lowercase() == *s)
        })
        .map(|e| json!(e))
        .collect();
    let start = kept.len().saturating_sub(limit);
    Ok(json!({"entries": &kept[start..], "total": kept.len()}))
}

fn close_conflicts(app: &AppHandle, args: &Value, client: &str) -> Reply {
    if args["reason"].as_str().is_none_or(|r| r.trim().is_empty()) {
        return Err("This needs a reason: a sentence for the person approving.".into());
    }
    let found = commands::get_conflicts(app.clone(), shared(app)?);
    let closing: Vec<String> = found.iter().filter(|f| f.running && f.closes).map(|f| f.name.to_string()).collect();
    if closing.is_empty() {
        return Ok(json!("Nothing to close."));
    }
    let show = |ask: fan_proposal::Ask| {
        let a = app.clone();
        on_main(app, Duration::from_secs(10), move || crate::ui::approval::open(&a, ask))?
    };
    let close = |id| {
        let _ = app.run_on_main_thread(move || crate::ui::approval::close(id));
    };
    let gate = Gate { desk: &DESK, show: &show, close: &close, wait: fan_proposal::WAIT };
    let lines = closing.iter().map(|n| format!("Close {n}")).collect();
    if !approve(&gate, &LAST_SETTINGS, args, client, crate::ui::approval::CLOSE_APPS, lines)? {
        return Ok(json!(fan_proposal::REJECTED));
    }
    let r = commands::close_conflicts_now(app.clone(), shared(app)?);
    serde_json::to_value(r).map_err(|e| e.to_string())
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
    fn presets_are_added_only_once_approved() {
        let (app, _) = app_with(true);
        let (store, dir) = crate::macros::tests::temp();
        app.manage(store);
        let desk = fan_proposal::Desk::new();
        let answer = Mutex::new(fan_proposal::Decision::Reject);
        let show = |ask: fan_proposal::Ask| {
            desk.decide(ask.id, *lock(&answer));
            Ok(())
        };
        let close = |_| {};
        let gate = Gate { desk: &desk, show: &show, close: &close, wait: Duration::from_millis(50) };
        let run = |tool: &str, args: Value| macro_tool_with(&app, tool, &args, "Tester", &gate);
        let all = run("list_macro_presets", json!({})).unwrap();
        assert!(all.as_array().unwrap().iter().any(|p| p["id"] == "copy" && p["does"] == "Ctrl+C"));
        assert!(run("add_macro_preset", json!({"preset": "nope", "reason": "r"})).unwrap_err().contains("No preset"));
        let copy = json!({"preset": "copy", "hotkey": "F14", "reason": "r"});
        assert_eq!(run("add_macro_preset", copy.clone()), Ok(json!(fan_proposal::REJECTED)));
        assert_eq!(run("list_macros", json!({})).unwrap()["macros"], json!([]));
        *lock(&answer) = fan_proposal::Decision::Apply;
        assert_eq!(run("add_macro_preset", copy).unwrap()["saved"], "Copy");
        assert_eq!(run("add_macro_preset", json!({"preset": "copy", "reason": "r"})).unwrap()["saved"], "Copy 2");
        let list = run("list_macros", json!({})).unwrap();
        assert_eq!(list["macros"][0]["hotkey"], "F14");
        assert_eq!(list["macros"][1]["hotkey"], Value::Null);
        std::fs::remove_dir_all(dir).unwrap();
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
        assert!(err(&app, "set_settings", json!({"openrgb_host": "10.0.0.2"})).contains("only in FoxDen's window"));
        assert!(err(&app, "set_settings", json!({"close_conflicts": false})).contains("needs a reason"));
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

    /// A mixer view as `get_mixer` gives it, with two outputs and one input.
    fn mixer_json() -> Value {
        let mut config = crate::mixer::AudioConfig::default();
        config.channels[0].apps = vec!["game.exe".into()];
        json!({
            "config": config, "running": false, "outputs": ["Speakers", "Headphones"], "inputs": ["USB Mic"],
            "profiles": crate::profiles::builtins(), "apps": [{"exe": "game.exe"}],
            "filters": [{"id": "PeakingEQ"}, {"id": "HighPass"}, {"id": "LowPass"}],
            "vmic": {"installed": false},
        })
    }

    #[test]
    fn get_audio_shows_every_chain() {
        let a = audio_of(&mixer_json());
        let mic = a["channels"].as_array().unwrap().iter().find(|c| c["channel"] == "mic").unwrap();
        for key in ["denoise", "gate", "agc", "comp", "deesser", "limiter", "order"] {
            assert!(!mic["processing"][key].is_null(), "{key}");
        }
        assert_eq!(mic["processing"]["gate"]["threshold_db"], -50.0);
        assert_eq!(mic["processing"]["denoise"]["engine"], "light");
        let game = &a["channels"][0];
        assert!(game["processing"]["deesser"].is_null() && !game["processing"]["comp"].is_null());
        assert_eq!(game["eq"]["bands"].as_array().unwrap().len(), 10);
        assert_eq!((a["chat_mix"].clone(), a["master"].clone()), (json!(0.0), json!(100.0)));
        assert_eq!(a["inputs"], json!(["USB Mic"]));
        assert!(a["sound_profiles"].as_array().unwrap().iter().any(|p| p["kind"] == "mic"));
        assert_eq!(a["eq_presets"][1], "bass_boost");
    }

    #[test]
    fn audio_settings_become_mixer_changes() {
        let v = mixer_json();
        let c = audio_changes(&json!({"output": "Headphones", "master": 50, "chat_mix": -40, "game_side": ["game", "aux"], "monitor_volume": 30}), &v).unwrap();
        assert_eq!(c[0], json!({"kind": "output", "name": "Headphones"}));
        assert_eq!(c[1]["volume"], 0.5);
        assert_eq!(c[2]["balance"], -0.4);
        assert_eq!(c[3]["channels"], json!(["game", "aux"]));
        assert_eq!(c[4], json!({"kind": "monitor", "on": false, "volume": 0.3}));
        for c in &c {
            serde_json::from_value::<crate::mixer::Change>(c.clone()).unwrap();
        }
        assert!(audio_changes(&json!({"output": "Nope"}), &v).unwrap_err().contains("No output"));
        assert!(audio_changes(&json!({"master": 140}), &v).is_err());
        assert!(audio_changes(&json!({"game_side": ["chat"]}), &v).is_err());
        assert!(audio_changes(&json!({"virtual_mic": true}), &v).unwrap_err().contains("VB-CABLE"));
        assert!(audio_changes(&json!({}), &v).is_err());

        let r = route_change(&json!({"channel": "mic", "source": "USB Mic"}), &v).unwrap();
        assert_eq!(r, json!({"kind": "route", "channel": "chatCapture", "source": "USB Mic", "loopback": false, "on": true}));
        let r = route_change(&json!({"channel": "media", "source": "Speakers"}), &v).unwrap();
        assert_eq!((r["loopback"].clone(), r["on"].clone()), (json!(true), json!(true)));
        assert!(route_change(&json!({"channel": "mic", "source": "Webcam"}), &v).is_err());
        assert!(route_change(&json!({"channel": "aux", "source": "", "on": true}), &v).unwrap_err().contains("needs a source"));
        assert_eq!(route_change(&json!({"channel": "game", "source": ""}), &v).unwrap()["on"], true, "its apps keep it on");
        serde_json::from_value::<crate::mixer::Change>(r).unwrap();

        assert_eq!(unassign_change(&json!({"app": "GAME.exe"}), &v).unwrap(), json!({"kind": "apps", "channel": "game", "apps": []}));
        assert!(unassign_change(&json!({"app": "other.exe"}), &v).is_err());
    }

    #[test]
    fn eq_edits_keep_the_other_bands() {
        let v = mixer_json();
        let c = eq_change(&json!({"channel": "mic", "band_changes": [{"index": 0, "filter": "HighPass", "frequency": 90}]}), &v).unwrap();
        assert_eq!((c["bands"][0]["filter"].clone(), c["bands"][0]["frequency"].clone()), (json!("HighPass"), json!(90.0)));
        assert_eq!(c["bands"][5]["frequency"], 1000.0);
        assert_eq!(c["enabled"], false);
        let c = eq_change(&json!({"channel": "game", "preset": "bass_boost"}), &v).unwrap();
        assert_eq!((c["enabled"].clone(), c["bands"][0]["gain_db"].clone()), (json!(true), json!(6.0)));
        let c = eq_change(&json!({"channel": "chat", "bands": [{"filter": "LowPass", "frequency": 8000}], "enabled": true}), &v).unwrap();
        assert_eq!(c["bands"].as_array().unwrap().len(), 1);
        serde_json::from_value::<crate::mixer::Change>(c).unwrap();
        assert!(eq_change(&json!({"channel": "mic", "band_changes": [{"index": 12}]}), &v).is_err());
        assert!(eq_change(&json!({"channel": "mic", "bands": [{"filter": "Wobble"}]}), &v).unwrap_err().contains("filter must be"));
        assert!(eq_change(&json!({"channel": "mic", "bands": [{"gain_db": 40}]}), &v).is_err());
        assert!(eq_change(&json!({"channel": "mic"}), &v).is_err());
    }

    #[test]
    fn processing_merges_into_the_chain() {
        let v = mixer_json();
        let c = proc_changes(&json!({
            "channel": "mic",
            "denoise": {"on": true, "engine": "strong", "strength": 0.7},
            "deesser": {"on": true, "threshold_db": -18},
            "limiter": {"lookahead_ms": 0},
            "gate": {"on": true, "threshold_db": -45, "hold_ms": 60},
            "order": ["eq", "agc", "comp", "deesser"],
        }), &v).unwrap();
        let p: foxden_mixer::chain::Processing = serde_json::from_value(c[0]["processing"].clone()).unwrap();
        assert_eq!((p.denoise.engine, p.denoise.strength), (foxden_mixer::denoise::Engine::Strong, 0.7));
        assert!(p.deesser.on && p.deesser.threshold_db == -18.0 && p.deesser.frequency == 6500.0);
        assert_eq!((p.limiter.lookahead_ms, p.limiter.ceiling_db), (0.0, -1.0));
        assert_eq!((p.gate.hold_ms, p.gate.attack_ms), (60.0, 2.0));
        assert_eq!(p.order[0], foxden_mixer::chain::Stage::Eq);
        assert_eq!(c[1], json!({"kind": "gate", "on": true, "threshold_db": -45.0}));
        serde_json::from_value::<crate::mixer::Change>(c[0].clone()).unwrap();

        let m = proc_changes(&json!({"channel": "master", "limiter": {"on": true, "ceiling_db": -2}}), &v).unwrap();
        assert_eq!(m[0]["channel"], Value::Null);
        assert!(proc_changes(&json!({"channel": "game", "deesser": {"on": true}}), &v).unwrap_err().contains("for the mic"));
        assert!(proc_changes(&json!({"channel": "mic", "comp": {"punch": 3}}), &v).unwrap_err().contains("has no punch"));
        assert!(proc_changes(&json!({"channel": "mic", "comp": {"ratio": "high"}}), &v).is_err());
        assert!(proc_changes(&json!({"channel": "mic", "order": ["eq", "eq", "comp", "agc"]}), &v).is_err());
        assert!(proc_changes(&json!({"channel": "mic", "denoise": {"engine": "magic"}}), &v).is_err());
        assert!(proc_changes(&json!({"channel": "mic"}), &v).is_err());
    }

    #[test]
    fn sound_profiles_and_priority_follow_the_mixer() {
        let v = mixer_json();
        let mic_profile = crate::profiles::builtins().into_iter().find(|p| p.kind == crate::profiles::Kind::Mic).unwrap();
        let out_profile = crate::profiles::builtins().into_iter().find(|p| p.kind == crate::profiles::Kind::Output).unwrap();
        let c = sound_profile_change("apply_sound_profile", &json!({"channel": "mic", "name": mic_profile.name}), &v).unwrap();
        assert_eq!(c["channel"], "chatCapture");
        assert!(sound_profile_change("apply_sound_profile", &json!({"channel": "mic", "name": out_profile.name}), &v).is_err());
        assert!(sound_profile_change("delete_sound_profile", &json!({"name": mic_profile.name}), &v).unwrap_err().contains("Built-in"));
        let code = crate::profiles::sound_code(mic_profile.clone());
        let c = sound_profile_change("import_sound_profile", &json!({"code": code}), &v).unwrap();
        serde_json::from_value::<crate::mixer::Change>(c).unwrap();
        let c = sound_profile_change("save_sound_profile", &json!({"channel": "chat", "name": " Mine "}), &v).unwrap();
        assert_eq!(c["name"], "Mine");

        let now = json!({
            "config": crate::priority::PriorityConfig::default(),
            "choices_out": [{"id": "a", "name": "Speakers"}, {"id": "b", "name": "Headphones"}], "choices_in": [],
        });
        let c = priority_config(&json!({"on": true, "outputs": ["Headphones", "a"]}), &now).unwrap();
        assert!(c.on && c.mixer);
        assert_eq!(c.outputs.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["b", "a"]);
        assert!(priority_config(&json!({"inputs": ["nothing"]}), &now).is_err());
    }

    fn mic_hooks_run(answers: Vec<Option<fan_proposal::Decision>>, calls: Vec<(&str, Value)>) -> (Vec<Reply>, usize, usize, usize) {
        let desk = fan_proposal::Desk::new();
        let answers = Mutex::new(answers.into_iter());
        let shown = Mutex::new(0usize);
        let show = |ask: fan_proposal::Ask| {
            *lock(&shown) += 1;
            assert!(ask.other.as_ref().unwrap().1[0].contains("seconds from USB Mic"), "{:?}", ask.other);
            if let Some(Some(d)) = lock(&answers).next() {
                desk.decide(ask.id, d);
            }
            Ok(())
        };
        let close = |_| {};
        let gate = Gate { desk: &desk, show: &show, close: &close, wait: Duration::from_millis(50) };
        let recorded = Mutex::new(0usize);
        let committed = Mutex::new(0usize);
        let setup = || {
            let mut s = crate::mic_check::setup_of(&serde_json::to_value(crate::mixer::AudioConfig::default()).unwrap())?;
            s.source = "USB Mic".into();
            Ok(s)
        };
        let record = |_: &crate::mic_check::MicSetup, secs: f64| {
            *lock(&recorded) += 1;
            let mut x = foxden_mixer_test_voice(secs);
            x.iter_mut().step_by(7).for_each(|v| *v += 0.001);
            Ok((x, 48000))
        };
        let commit = |_: &crate::mic_check::MicSetup, _: &crate::mic_check::Tuning| {
            *lock(&committed) += 1;
            Ok(())
        };
        let hooks = MicHooks { setup: &setup, record: &record, commit: &commit };
        let out = calls.into_iter().map(|(tool, args)| mic_tool_with(tool, &args, "Tester", &gate, &hooks)).collect();
        let (s, r, c) = (*lock(&shown), *lock(&recorded), *lock(&committed));
        (out, s, r, c)
    }

    fn foxden_mixer_test_voice(secs: f64) -> Vec<f32> {
        (0..(48000.0 * secs) as usize)
            .map(|i| {
                let t = i as f32 / 48000.0;
                0.1 * (t * std::f32::consts::TAU * 150.0).sin() * (t * std::f32::consts::TAU * 2.0).sin().abs()
            })
            .collect()
    }

    #[test]
    fn every_recording_asks_again() {
        use fan_proposal::Decision::Apply;
        let ask = json!({"seconds": 2, "reason": "Check the mic"});
        let (out, shown, recorded, _) = mic_hooks_run(vec![Some(Apply), Some(Apply)], vec![("analyze_mic", ask.clone()), ("analyze_mic", ask)]);
        assert_eq!((shown, recorded), (2, 2), "the second recording was asked for again");
        let r = out[0].as_ref().unwrap();
        for key in ["raw", "processed", "difference"] {
            assert!(r[key].is_object(), "{key}");
        }
        assert!(r["raw"]["lufs"].is_number() && r["raw"]["bands"].is_array());
        assert!(out[1].is_ok());
    }

    #[test]
    fn a_denied_or_unanswered_recording_records_nothing() {
        use fan_proposal::Decision::{Apply, Reject};
        let ask = json!({"seconds": 2, "reason": "Check the mic"});
        let (out, shown, recorded, _) = mic_hooks_run(vec![Some(Reject)], vec![("analyze_mic", ask.clone())]);
        assert_eq!((shown, recorded), (1, 0));
        assert!(out[0].as_ref().unwrap_err().contains("nothing was recorded"));
        let (out, shown, recorded, _) = mic_hooks_run(vec![None], vec![("analyze_mic", ask.clone())]);
        assert_eq!((shown, recorded), (1, 0));
        assert_eq!(out[0], Err(fan_proposal::LATE.into()));
        assert_eq!(lock(&LAST_MIC).as_ref().unwrap()["outcome"], "not approved in time");
        let (out, shown, recorded, committed) = mic_hooks_run(
            vec![Some(Reject), Some(Apply)],
            vec![("auto_tune_mic", json!({"target": "podcast", "seconds": 2, "reason": "Tune"})), ("auto_tune_mic", json!({"target": "podcast", "seconds": 2, "reason": "Tune"}))],
        );
        assert_eq!((shown, recorded, committed), (2, 1, 1), "auto-tune asks for its own recording each run");
        assert!(out[0].is_err());
        let r = out[1].as_ref().unwrap();
        assert_eq!(r["target"], "podcast");
        assert!(r["before"]["lufs"].is_number() && r["after"]["lufs"].is_number() && r["settings"]["processing"]["denoise"]["engine"] == "strong");
        let (_, shown, recorded, _) = mic_hooks_run(vec![], vec![("analyze_mic", json!({"seconds": 20, "reason": "x"})), ("auto_tune_mic", json!({"target": "karaoke", "reason": "x"})), ("analyze_mic", json!({"seconds": 2}))]);
        assert_eq!((shown, recorded), (0, 0), "bad arguments are refused before asking");
    }

    #[test]
    fn fan_edits_change_one_curve_or_rule() {
        let v = json!({
            "curves": [{"name": "Quiet", "sensors": ["cpu"], "points": [[30, 20], [80, 100]], "hysteresis": 2}],
            "rules": [{"fan": "board/fan1", "name": "CPU fan", "curve": "Quiet", "min_duty": 20, "max_duty": 100, "ramp": 20}],
            "fans": [{"id": "board/fan1"}, {"id": "board/fan2"}],
        });
        let p = fan_edit_proposal("set_fan_curve", &json!({"name": "Quiet", "points": [[40, 25], [85, 100]], "reason": "r"}), &v).unwrap();
        assert_eq!((p["curves"][0]["points"][0].clone(), p["curves"][0]["sensors"].clone()), (json!([40, 25]), json!(["cpu"])));
        assert_eq!(p["rules"][0]["curve"], "Quiet");
        let p = fan_edit_proposal("set_fan_curve", &json!({"name": "Loud", "sensors": ["gpu"], "points": [[30, 50]], "reason": "r"}), &v).unwrap();
        assert_eq!(p["curves"].as_array().unwrap().len(), 2);
        assert!(fan_edit_proposal("set_fan_curve", &json!({"name": "New", "reason": "r"}), &v).is_err());
        assert!(fan_edit_proposal("remove_fan_curve", &json!({"name": "Quiet"}), &v).unwrap_err().contains("follows"));
        let p = fan_edit_proposal("set_fan_rule", &json!({"fan": "board/fan2", "curve": "Quiet", "max_duty": 70, "reason": "r"}), &v).unwrap();
        assert_eq!(p["rules"][1], json!({"fan": "board/fan2", "curve": "Quiet", "min_duty": 0.0, "max_duty": 70, "ramp": 20.0}));
        let p = fan_edit_proposal("set_fan_rule", &json!({"fan": "board/fan1", "curve": null, "reason": "r"}), &v).unwrap();
        assert!(p["rules"].as_array().unwrap().is_empty(), "back to firmware");
        assert!(fan_edit_proposal("set_fan_rule", &json!({"fan": "board/fan9", "curve": "Quiet"}), &v).is_err());
        assert_eq!(p["reason"], "r");
    }

    #[test]
    fn devices_logs_and_settings_tools_answer() {
        let (app, shared) = with_devices();
        assert!(err(&app, "set_hardware_mode", json!({"device": "TKL keyboard", "mode": "wave"})).contains("no effects of its own"));
        ok(&app, "reset_layout", json!({}));
        ok(&app, "detect_devices", json!({}));
        assert!(err(&app, "set_zone_leds", json!({"zone": "nope", "leds": 10})).contains("No resizable zone"));
        assert!(err(&app, "get_device_info", json!({"device": "nothing"})).contains("No device"));
        assert!(err(&app, "set_headset", json!({"headset": "nope", "control": "sidetone", "value": 1})).contains("No headset"));
        assert!(err(&app, "set_fan_service", json!({})).contains("running"));
        assert!(err(&app, "close_conflicts", json!({})).contains("reason"));
        assert!(ok(&app, "get_conflicts", json!({})).is_array());

        log::warn("test", "a warning for get_logs");
        let logs = ok(&app, "get_logs", json!({"level": "warn", "source": "test"}));
        assert!(logs["entries"].as_array().unwrap().iter().all(|e| e["level"] != "info"));
        assert!(logs["entries"].as_array().unwrap().iter().any(|e| e["text"] == "a warning for get_logs"));
        assert!(err(&app, "get_logs", json!({"level": "loud"})).contains("level"));

        let desk = fan_proposal::Desk::new();
        let shown = Mutex::new(Vec::new());
        let show = |ask: fan_proposal::Ask| {
            lock(&shown).push(ask.settings.clone());
            desk.decide(ask.id, fan_proposal::Decision::Reject);
            Ok(())
        };
        let close = |_| {};
        let r = set_settings_with(&app, &json!({"fans_module": false, "send_reports": true, "reason": "r"}), "T", &desk, &show, &close, Duration::from_millis(50));
        assert_eq!(r, Ok(json!(fan_proposal::REJECTED)));
        assert_eq!(lock(&shown)[0], [("Fans", false), ("Send anonymous error reports", true)]);
        assert!(lock(&shared).config.settings.fans_module);
        assert!(set_settings_with(&app, &json!({"ai_access": false}), "T", &desk, &show, &close, Duration::from_millis(50)).is_err());
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
        s.trim_end_matches('\0').to_string()
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
