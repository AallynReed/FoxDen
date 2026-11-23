//! New releases from GitHub: checks shortly after start, then every hour, and tells the user.
//! With `auto_update` on it installs by itself once the app is in the tray or a minute has passed.
//! It reads the same latest.json as the Tauri build, so releases must publish a native installer there.

use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};

use crate::state::{Shared, lock};

const ENDPOINT: &str = "https://github.com/AallynReed/FoxDen/releases/latest/download/latest.json";
const CHANGELOG: &str = "https://github.com/AallynReed/FoxDen/releases/latest/download/changelog.json";
const PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEI2OUExOEQ0OTdDRTBEMDEKUldRQkRjNlgxQmlhdHNRcDlJR2xIcVh4R1dobE1tU0JCMW0yall4U3BoRVdJcW9jTXh1WmVldVQK";
const TARGETS: [&str; 2] = ["windows-x86_64-nsis", "windows-x86_64"];
const FIRST: Duration = Duration::from_secs(5);
const EVERY: Duration = Duration::from_secs(60 * 60);
const GRACE: Duration = Duration::from_secs(60);
const DEV: &str = "development builds do not install updates";
const MAX_INSTALLER: u64 = 200 << 20;
const MAX_CHANGELOG: u64 = 1 << 20;
const FILE_SHARE_READ: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Info {
    pub version: String,
    pub notes: String,
    /// Installs by itself unless the user says Later.
    pub auto: bool,
    /// Changelog entries from the one after this version up to the new one.
    pub changes: Vec<Value>,
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

/// Whether the automatic install goes ahead now.
fn due(idle: bool, waited: Duration) -> bool {
    idle || waited >= GRACE
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
    if !info.auto || std::mem::replace(&mut *lock(&WAITING), true) {
        return;
    }
    let found = Instant::now();
    loop {
        let put_off = lock(&LATER).as_deref() == Some(info.version.as_str());
        let idle = app
            .get_webview_window("main")
            .is_none_or(|w| !w.is_visible().unwrap_or(false));
        if put_off || *lock(&INSTALLING) {
            break;
        }
        if due(idle, found.elapsed()) {
            if let Err(e) = install(app).await {
                crate::log::warn("updates", format!("installing {} failed: {e}", info.version));
            }
            break;
        }
        tokio_sleep(Duration::from_secs(1)).await;
    }
    *lock(&WAITING) = false;
}

/// Test-only path for development builds: `FOXDEN_FAKE_UPDATE=<version>` shows the UI for a release that is not there.
fn fake(app: &AppHandle) {
    let Ok(version) = std::env::var("FOXDEN_FAKE_UPDATE") else {
        return;
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio_sleep(Duration::from_secs(3)).await;
        let from = std::env::var("FOXDEN_FAKE_FROM").unwrap_or_else(|_| env!("CARGO_PKG_VERSION").into());
        let info = Info {
            changes: crate::changelog::between(&crate::changelog::built_in(), &from, &version),
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
pub fn newer(remote: &str, current: &str) -> bool {
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
    let changes = crate::changelog::between(&changelog().await, env!("CARGO_PKG_VERSION"), &release.version);
    let info = Info {
        changes,
        version: release.version.clone(),
        notes: json["notes"].as_str().unwrap_or_default().to_string(),
        auto: {
            let settings = &lock(shared).config.settings;
            settings.auto_update && settings.update_ignored != release.version
        },
    };
    let known = lock(&FOUND)
        .replace((release, info.clone()))
        .is_some_and(|(_, old)| old == info);
    if !known {
        let _ = app.emit("update", &info);
    }
    Ok(Some(info))
}

/// The newest release's changelog, or the one built in when it cannot be fetched.
async fn changelog() -> Value {
    let fetched = async {
        let reply = client(Duration::from_secs(30)).ok()?.get(CHANGELOG).send().await.ok()?;
        if !reply.status().is_success() {
            return None;
        }
        let data = read_capped(reply, MAX_CHANGELOG, |_, _| {}).await.ok()?;
        serde_json::from_slice::<Value>(&data).ok().filter(Value::is_array)
    };
    match fetched.await {
        Some(v) => v,
        None => crate::changelog::built_in(),
    }
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
    signed_for(signature.trusted_comment(), &release.version)
}

fn setup_name(version: &str) -> String {
    format!("FoxDen_{version}_x64-setup.exe")
}

/// The signer names the signed file in the trusted comment, which ties the signature to one version.
fn signed_for(trusted_comment: &str, version: &str) -> Result<(), String> {
    let want = setup_name(version);
    match trusted_comment.split('\t').find_map(|f| f.strip_prefix("file:")) {
        Some(f) if f == want => Ok(()),
        Some(f) => Err(format!("the update was signed as {f}, not {want}")),
        None => Err("the update signature does not name its file".into()),
    }
}

async fn read_capped(
    mut reply: reqwest::Response,
    cap: u64,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<Vec<u8>, String> {
    let total = reply.content_length();
    if total.is_some_and(|t| t > cap) {
        return Err("the download is too large".into());
    }
    let mut data = Vec::with_capacity(total.unwrap_or(0) as usize);
    while let Some(chunk) = reply.chunk().await.map_err(|e| e.to_string())? {
        if (data.len() + chunk.len()) as u64 > cap {
            return Err("the download is too large".into());
        }
        data.extend_from_slice(&chunk);
        progress(data.len() as u64, total);
    }
    Ok(data)
}

/// A new folder per attempt, so nothing left from an earlier one is reused.
fn fresh_dir() -> Result<PathBuf, String> {
    let base = std::env::temp_dir();
    for n in 0..100u32 {
        let t = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let dir = base.join(format!("FoxDen-update-{}-{t:x}-{n}", std::process::id()));
        match std::fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    Err("no folder for the update could be made".into())
}

/// Writes the file and returns a read-only handle that keeps it from being changed or deleted.
fn write_locked(file: &Path, data: &[u8]) -> Result<File, String> {
    let mut out = OpenOptions::new()
        .write(true)
        .create_new(true)
        .share_mode(0)
        .open(file)
        .map_err(|e| e.to_string())?;
    out.write_all(data).map_err(|e| e.to_string())?;
    drop(out);
    let mut held = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(file)
        .map_err(|e| e.to_string())?;
    let mut back = Vec::with_capacity(data.len());
    held.read_to_end(&mut back).map_err(|e| e.to_string())?;
    if back != data {
        return Err("the update changed after it was written".into());
    }
    Ok(held)
}

async fn install(app: &AppHandle) -> Result<(), String> {
    if cfg!(debug_assertions) {
        return Err(DEV.into());
    }
    let release = lock(&FOUND)
        .as_ref()
        .map(|(r, _)| r.clone())
        .ok_or("no update to install")?;
    {
        let mut busy = lock(&INSTALLING);
        if *busy {
            return Err("already installing".into());
        }
        *busy = true;
    }
    let result = download_and_run(app, &release).await;
    *lock(&INSTALLING) = false;
    result?;
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || crate::quit(&handle));
    Ok(())
}

async fn download_and_run(app: &AppHandle, release: &Release) -> Result<(), String> {
    let reply = client(Duration::from_secs(600))?
        .get(&release.url)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !reply.status().is_success() {
        return Err(format!("the download answered {}", reply.status().as_u16()));
    }
    let data = read_capped(reply, MAX_INSTALLER, |done, total| {
        let _ = app.emit("update-progress", Progress { done, total });
    })
    .await?;
    verify(&data, release)?;
    if data.starts_with(b"PK") {
        return Err("the update is a zip archive, which this build cannot install".into());
    }
    let file = fresh_dir()?.join(setup_name(&release.version));
    let held = write_locked(&file, &data)?;
    let started = run_installer(&file);
    drop(held);
    started
}

/// Starts the installer in passive mode, as the Tauri updater does.
fn run_installer(file: &Path) -> Result<(), String> {
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
    *lock(&LATER) = lock(&FOUND).as_ref().map(|(_, info)| info.version.clone());
}

pub fn get_update() -> Option<Info> {
    lock(&FOUND).as_ref().map(|(_, info)| info.clone())
}

async fn tokio_sleep(time: Duration) {
    let _ = tauri::async_runtime::spawn_blocking(move || std::thread::sleep(time)).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_automatic_install_waits_for_the_tray_or_a_minute() {
        assert!(!due(false, Duration::ZERO));
        assert!(due(true, Duration::ZERO));
        assert!(due(false, GRACE));
    }

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
    fn the_signature_must_name_the_versions_installer() {
        let real = "timestamp:1795450659\tfile:FoxDen_0.18.2_x64-setup.exe";
        assert!(signed_for(real, "0.18.2").is_ok());
        assert!(signed_for(real, "0.18.3").is_err());
        assert!(signed_for("timestamp:1795450659", "0.18.2").is_err());
        assert!(signed_for("timestamp:1\tfile:FoxDen_0.18.2_x64-setup.exe.zip", "0.18.2").is_err());
    }

    #[test]
    fn a_released_signature_names_its_file() {
        let sig = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVRQkRjNlgxQmlhdHVnd3RZaUdjeGsxL2FmUXBEYmYrcjllTkErN2huYlZtUDdqOUw1eUlEc0YwZ0pTa3FUcFFWSkJ6RTk3bXgyR0tQcytFYXdBYXlBbFdSYnZLbGJzNWdVPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzk1NDUwNjU5CWZpbGU6Rm94RGVuXzAuMTguMl94NjQtc2V0dXAuZXhlCmVIekFGRWl5WU9kMnh5RUFRanRST002M3NBbDhBRUF6MGYxdkt4Vkduc1Y4bmV5eDVOcTRmYjZHVzdUQ29SSm1ENDdjajZiYkdKS0QvRlFVWHA3MUF3PT0K";
        let signature = minisign_verify::Signature::decode(&text64(sig).unwrap()).unwrap();
        assert!(signed_for(signature.trusted_comment(), "0.18.2").is_ok());
    }

    #[test]
    fn the_written_installer_runs_but_cannot_be_changed() {
        let system = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let data = std::fs::read(Path::new(&system).join(r"System32\whoami.exe")).unwrap();
        let dir = fresh_dir().unwrap();
        let other = fresh_dir().unwrap();
        assert_ne!(dir, other);
        let _ = std::fs::remove_dir(&other);
        let file = dir.join("whoami.exe");
        let held = write_locked(&file, &data).unwrap();
        assert!(OpenOptions::new().write(true).open(&file).is_err());
        assert!(std::fs::remove_file(&file).is_err());
        assert!(std::process::Command::new(&file).output().unwrap().status.success());
        drop(held);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_client_has_tls() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        assert!(client(Duration::from_secs(1)).is_ok());
    }
}
