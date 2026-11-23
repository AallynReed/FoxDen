//! The pipe AI assistants reach FoxDen through, via foxden-mcp.exe, while the user allows it.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use foxden_core::{Effect, from_hex};
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

use crate::fan_proposal::{self, DESK, Hooks};
use crate::mixer::Mixer;
use crate::state::{Shared, lock};
use crate::{battery, commands, fans, log, mixer};

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
                "id": d["id"], "name": d["name"], "kind": d["kind"],
                "leds": d["leds"].as_array().map_or(0, Vec::len),
                "enabled": d["enabled"], "brightness": percent(&d["brightness"]),
                "driven_by": if own { "hardware" } else { "foxden" },
            });
            if let Some(effect) = snap["overrides"].get(d["id"].as_str().unwrap_or_default()) {
                out["own_effect"] = effect.clone();
            }
            out
        })
        .collect();
    Ok(json!({
        "version": snap["version"],
        "effect": snap["effect"],
        "layers": snap["layers"].as_array().map_or(0, Vec::len),
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

fn effects() -> Value {
    let list: Vec<Value> = Effect::catalog()
        .into_iter()
        .map(|info| {
            let params: Vec<Value> = info
                .params
                .iter()
                .map(|p| {
                    let mut out = serde_json::to_value(&p.control).unwrap_or_default();
                    out["name"] = json!(p.name);
                    out["label"] = json!(p.label);
                    if out["type"] == "color" {
                        out["format"] = json!("#rrggbb or [r,g,b]");
                    }
                    out
                })
                .collect();
            json!({"kind": kind_of(&info.default), "label": info.label, "params": params, "default": info.default})
        })
        .collect();
    json!({"effects": list})
}

/// The effect `kind` with `params` laid over the current one of that kind, or over its default.
fn build_effect(current: Option<&Value>, kind: &str, params: &Map<String, Value>) -> Result<Effect, String> {
    let catalog = Effect::catalog();
    let Some(info) = catalog.iter().find(|i| kind_of(&i.default) == kind) else {
        let kinds: Vec<String> = catalog.iter().map(|i| kind_of(&i.default)).collect();
        return Err(format!("Unknown effect kind \"{kind}\". Kinds: {}.", kinds.join(", ")));
    };
    let mut effect = match current {
        Some(c) if c["kind"] == kind => c.clone(),
        _ => serde_json::to_value(&info.default).map_err(|e| e.to_string())?,
    };
    for (name, v) in params {
        if name == "kind" {
            continue;
        }
        let Some(p) = info.params.iter().find(|p| p.name == name) else {
            let names: Vec<&str> = info.params.iter().map(|p| p.name).collect();
            return Err(format!("{kind} has no parameter \"{name}\". It takes: {}.", names.join(", ")));
        };
        let control = serde_json::to_value(&p.control).unwrap_or_default();
        effect[name.as_str()] = param(&control, name, v)?;
    }
    serde_json::from_value(effect).map_err(|e| format!("Those parameters don't fit {kind}: {e}"))
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
    let shown = serde_json::to_value(&effect).unwrap_or_default();
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
        (app, shared)
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
        assert_eq!(access_of(&pipe), format!("O:{0}D:P(A;;FA;;;{0})", user_sid().unwrap()));
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
