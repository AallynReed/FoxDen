//! New releases from GitHub: checks a minute after start, then every hour, and tells the user.
//! With `auto_update` on it installs by itself once the app is in the tray or a minute has passed.
//! It reads the same latest.json as the Tauri build, so releases must publish a native installer there.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::Engine;
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};

use crate::state::{Shared, lock};

const ENDPOINT: &str = "https://github.com/AallynReed/FoxDen/releases/latest/download/latest.json";
const PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEI2OUExOEQ0OTdDRTBEMDEKUldRQkRjNlgxQmlhdHNRcDlJR2xIcVh4R1dobE1tU0JCMW0yall4U3BoRVdJcW9jTXh1WmVldVQK";
const TARGETS: [&str; 2] = ["windows-x86_64-nsis", "windows-x86_64"];
const FIRST: Duration = Duration::from_secs(60);
const EVERY: Duration = Duration::from_secs(60 * 60);
const GRACE: Duration = Duration::from_secs(60);
const DEV: &str = "development builds do not install updates";

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Info {
    pub version: String,
    pub notes: String,
    /// Installs by itself unless the user says Later.
    pub auto: bool,
}

#[derive(Clone, Serialize)]
struct Progress {
    done: u64,
    total: Option<u64>,
}

#[derive(Clone)]
struct Release {
    version: String,
    url: String,
    signature: String,
}

static FOUND: Mutex<Option<(Release, Info)>> = Mutex::new(None);
/// The version the user put off with Later.
static LATER: Mutex<Option<String>> = Mutex::new(None);
static INSTALLING: Mutex<bool> = Mutex::new(false);
/// Whether a countdown to an automatic install is running.
static WAITING: Mutex<bool> = Mutex::new(false);

fn guard<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Whether the automatic install goes ahead now.
pub fn due(auto: bool, put_off: bool, idle: bool, waited: Duration) -> bool {
    auto && !put_off && (idle || waited >= GRACE)
}

pub fn start(app: &AppHandle, shared: Shared) {
    if cfg!(debug_assertions) {
        fake(app);
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio_sleep(FIRST).await;
        loop {
            if let Ok(Some(info)) = check(&app, &shared).await {
                wait_and_install(&app, &info).await;
            }
            tokio_sleep(EVERY).await;
        }
    });
}

async fn wait_and_install(app: &AppHandle, info: &Info) {
    if !info.auto || std::mem::replace(&mut *guard(&WAITING), true) {
        return;
    }
    let found = Instant::now();
    loop {
        let put_off = guard(&LATER).as_deref() == Some(info.version.as_str());
        let idle = app
            .get_webview_window("main")
            .is_none_or(|w| !w.is_visible().unwrap_or(false));
        if put_off || *guard(&INSTALLING) {
            break;
        }
        if due(true, put_off, idle, found.elapsed()) {
            if let Err(e) = install(app).await {
                crate::log::warn("updates", format!("installing {} failed: {e}", info.version));
            }
            break;
        }
        tokio_sleep(Duration::from_secs(1)).await;
    }
    *guard(&WAITING) = false;
}

/// Test-only path for development builds: `FOXDEN_FAKE_UPDATE=<version>` shows the UI for a release that is not there.
fn fake(app: &AppHandle) {
    let Ok(version) = std::env::var("FOXDEN_FAKE_UPDATE") else {
        return;
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio_sleep(Duration::from_secs(3)).await;
        let info = Info {
            version,
            notes: "A test release. Nothing will be installed.".into(),
            auto: false,
        };
        let _ = app.emit("update", &info);
    });
}

fn client(timeout: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(concat!("FoxDen/", env!("CARGO_PKG_VERSION")))
        .timeout(timeout)
        .build()
        .map_err(|e| e.to_string())
}

fn numbers(version: &str) -> (Vec<u64>, bool) {
    let v = version.trim().trim_start_matches('v');
    let (core, pre) = match v.split_once(['-', '+']) {
        Some((c, rest)) => (c, v.as_bytes()[c.len()] == b'-' && !rest.is_empty()),
        None => (v, false),
    };
    (core.split('.').map(|n| n.parse().unwrap_or(0)).collect(), pre)
}

/// Whether `remote` is a newer release than `current`; a pre-release ranks below its release.
fn newer(remote: &str, current: &str) -> bool {
    let (r, r_pre) = numbers(remote);
    let (c, c_pre) = numbers(current);
    let len = r.len().max(c.len()).max(3);
    let pad = |mut v: Vec<u64>| {
        v.resize(len, 0);
        v
    };
    match pad(r).cmp(&pad(c)) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => c_pre && !r_pre,
    }
}

fn read_release(json: &Value) -> Result<Release, String> {
    let version = json["version"].as_str().ok_or("the release has no version")?;
    let platforms = &json["platforms"];
    let (url, signature) = TARGETS
        .iter()
        .find_map(|t| {
            let p = &platforms[*t];
            Some((p["url"].as_str()?, p["signature"].as_str()?))
        })
        .or_else(|| Some((json["url"].as_str()?, json["signature"].as_str()?)))
        .ok_or("the release has no Windows installer")?;
    Ok(Release {
        version: version.trim_start_matches('v').to_string(),
        url: url.to_string(),
        signature: signature.to_string(),
    })
}

/// Asks GitHub; a newer release is announced once per version.
async fn check(app: &AppHandle, shared: &Shared) -> Result<Option<Info>, String> {
    let reply = client(Duration::from_secs(30))?
        .get(ENDPOINT)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if reply.status().as_u16() == 404 {
        return Ok(None);
    }
    if !reply.status().is_success() {
        return Err(format!("the update server answered {}", reply.status().as_u16()));
    }
    let json: Value = reply.json().await.map_err(|e| e.to_string())?;
    let release = read_release(&json)?;
    if !newer(&release.version, env!("CARGO_PKG_VERSION")) {
        return Ok(None);
    }
    let info = Info {
        version: release.version.clone(),
        notes: json["notes"].as_str().unwrap_or_default().to_string(),
        auto: lock(shared).config.settings.auto_update,
    };
    let known = guard(&FOUND)
        .replace((release, info.clone()))
        .is_some_and(|(_, old)| old == info);
    if !known {
        let _ = app.emit("update", &info);
    }
    Ok(Some(info))
}

fn text64(b64: &str) -> Result<String, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .map_err(|_| "the update signature could not be read")?;
    String::from_utf8(bytes).map_err(|_| "the update signature could not be read".into())
}

/// The installer must carry the release key's signature, made for the announced version.
fn verify(data: &[u8], release: &Release) -> Result<(), String> {
    let bad = |e: minisign_verify::Error| format!("the update signature is not valid ({e})");
    let key = minisign_verify::PublicKey::decode(&text64(PUBKEY)?).map_err(bad)?;
    let signature = minisign_verify::Signature::decode(&text64(&release.signature)?).map_err(bad)?;
    key.verify(data, &signature, true).map_err(bad)?;
    let signed = signature
        .trusted_comment()
        .split('\t')
        .find_map(|f| f.strip_prefix("version:"));
    match signed {
        Some(v) if v.trim_start_matches('v') != release.version => {
            Err(format!("the update was signed for {v}, not {}", release.version))
        }
        _ => Ok(()),
    }
}

async fn install(app: &AppHandle) -> Result<(), String> {
    if cfg!(debug_assertions) {
        return Err(DEV.into());
    }
    let release = guard(&FOUND)
        .as_ref()
        .map(|(r, _)| r.clone())
        .ok_or("no update to install")?;
    {
        let mut busy = guard(&INSTALLING);
        if *busy {
            return Err("already installing".into());
        }
        *busy = true;
    }
    let result = download_and_run(app, &release).await;
    *guard(&INSTALLING) = false;
    result?;
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || crate::quit(&handle));
    Ok(())
}

async fn download_and_run(app: &AppHandle, release: &Release) -> Result<(), String> {
    let mut reply = client(Duration::from_secs(600))?
        .get(&release.url)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !reply.status().is_success() {
        return Err(format!("the download answered {}", reply.status().as_u16()));
    }
    let total = reply.content_length();
    let mut data = Vec::with_capacity(total.unwrap_or(0) as usize);
    while let Some(chunk) = reply.chunk().await.map_err(|e| e.to_string())? {
        data.extend_from_slice(&chunk);
        let _ = app.emit(
            "update-progress",
            Progress {
                done: data.len() as u64,
                total,
            },
        );
    }
    verify(&data, release)?;
    if data.starts_with(b"PK") {
        return Err("the update is a zip archive, which this build cannot install".into());
    }
    let dir = std::env::temp_dir().join(format!("FoxDen-{}-update", release.version));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = dir.join(format!("FoxDen_{}_x64-setup.exe", release.version));
    std::fs::write(&file, &data).map_err(|e| e.to_string())?;
    run_installer(&file)
}

/// Starts the installer in passive mode, as the Tauri updater does.
fn run_installer(file: &std::path::Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOW;
    let wide = |s: &std::ffi::OsStr| s.encode_wide().chain(Some(0)).collect::<Vec<u16>>();
    let file = wide(file.as_os_str());
    let args = wide("/P /UPDATE".as_ref());
    let verb = wide("open".as_ref());
    let out = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            args.as_ptr(),
            std::ptr::null(),
            SW_SHOW,
        )
    };
    if out as isize <= 32 {
        return Err(format!("the installer could not start (error {})", out as isize));
    }
    Ok(())
}

pub async fn check_update(app: AppHandle, shared: Shared) -> Result<Option<Info>, String> {
    if cfg!(debug_assertions) {
        return Err(DEV.into());
    }
    let found = check(&app, &shared).await?;
    if let Some(info) = found.clone() {
        let app = app.clone();
        tauri::async_runtime::spawn(async move { wait_and_install(&app, &info).await });
    }
    Ok(found)
}

pub async fn install_update(app: AppHandle) -> Result<(), String> {
    install(&app).await
}

pub fn later_update() {
    *guard(&LATER) = guard(&FOUND).as_ref().map(|(_, info)| info.version.clone());
}

pub fn get_update() -> Option<Info> {
    guard(&FOUND).as_ref().map(|(_, info)| info.clone())
}

async fn tokio_sleep(time: Duration) {
    let _ = tauri::async_runtime::spawn_blocking(move || std::thread::sleep(time)).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number() {
        assert!(newer("0.18.0", "0.17.0"));
        assert!(newer("0.17.10", "0.17.9"));
        assert!(!newer("0.17.0", "0.17.0"));
        assert!(!newer("0.16.5", "0.17.0"));
        assert!(newer("0.17.0", "0.17.0-beta"));
        assert!(!newer("0.17.0-beta", "0.17.0"));
    }

    #[test]
    fn the_client_has_tls() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        assert!(client(Duration::from_secs(1)).is_ok());
    }
}
