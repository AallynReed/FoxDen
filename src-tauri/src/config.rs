use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use foxrgb_core::Effect;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};

const FILE: &str = "config.json";
const CORRUPT: &str = "config.corrupt.json";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub fps: u32,
    pub openrgb_enabled: bool,
    pub openrgb_host: String,
    pub openrgb_port: u16,
    pub native_drivers: bool,
    pub show_virtual: bool,
    pub close_to_tray: bool,
    pub start_minimized: bool,
    pub autostart: bool,
    /// Watch for hardware being plugged in or removed and for the OpenRGB server.
    pub auto_detect: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            fps: 30,
            openrgb_enabled: false,
            openrgb_host: "127.0.0.1".into(),
            openrgb_port: 6742,
            native_drivers: false,
            show_virtual: true,
            close_to_tray: true,
            start_minimized: false,
            autostart: false,
            auto_detect: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeviceConfig {
    pub enabled: bool,
    pub brightness: f32,
    /// Canvas placement chosen by the user, if any.
    pub rect: Option<[f32; 4]>,
}

impl Default for DeviceConfig {
    fn default() -> Self {
        DeviceConfig { enabled: true, brightness: 1.0, rect: None }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    #[serde(default, deserialize_with = "lenient")]
    pub effect: Effect,
    #[serde(default = "full")]
    pub brightness: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    #[serde(deserialize_with = "lenient")]
    pub effect: Effect,
    pub brightness: f32,
    pub settings: Settings,
    pub devices: BTreeMap<String, DeviceConfig>,
    pub profiles: BTreeMap<String, Profile>,
    pub active_profile: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            effect: Effect::default(),
            brightness: 1.0,
            settings: Settings::default(),
            devices: BTreeMap::new(),
            profiles: BTreeMap::new(),
            active_profile: None,
        }
    }
}

fn full() -> f32 {
    1.0
}

/// A value this build cannot read becomes the default instead of failing the whole file.
fn lenient<'de, D: Deserializer<'de>, T: DeserializeOwned + Default>(d: D) -> Result<T, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}

/// The app's identifier. The uninstaller's "delete application data" box removes the folder of that name.
const FOLDER: &str = "com.foxrgb.desktop";

/// Where everything the app keeps goes: `FOXRGB_CONFIG_DIR` if set, otherwise `%APPDATA%\com.foxrgb.desktop`.
pub fn dir() -> PathBuf {
    match std::env::var_os("FOXRGB_CONFIG_DIR") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default()).join(FOLDER),
    }
}

/// The web view's own data. Left to itself it would go to a second folder that `FOXRGB_CONFIG_DIR` does not move.
pub fn webview_dir() -> PathBuf {
    dir().join("webview")
}

/// Never fails: a missing file gives the defaults, a corrupt one is renamed aside first.
pub fn load(dir: &Path) -> Config {
    let path = dir.join(FILE);
    let Ok(bytes) = fs::read(&path) else {
        return Config::default();
    };
    let mut config: Config = serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        eprintln!("{}: {e}", path.display());
        let _ = fs::rename(&path, dir.join(CORRUPT));
        Config::default()
    });
    config.settings.fps = config.settings.fps.clamp(10, 60);
    config.brightness = config.brightness.clamp(0.0, 1.0);
    config
}

/// Writes a temporary file and renames it over the old one.
pub fn save(dir: &Path, config: &Config) -> Result<(), String> {
    let text = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    let tmp = dir.join("config.json.tmp");
    fs::create_dir_all(dir)
        .and_then(|_| fs::write(&tmp, text))
        .and_then(|_| fs::rename(&tmp, dir.join(FILE)))
        .map_err(|e| format!("{}: {e}", dir.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::current_exe().unwrap().with_file_name(format!("foxrgb-test-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_folder_has_the_name_the_uninstaller_removes() {
        let conf: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(conf["identifier"], FOLDER);
        // The window is made in main.rs, which gives the web view its folder.
        assert_eq!(conf["app"]["windows"][0]["create"], false);
        assert!(webview_dir().starts_with(dir()));
    }

    // The setup program takes its version from tauri.conf.json, the binaries from Cargo.toml.
    #[test]
    fn the_setup_program_and_the_binaries_carry_one_version() {
        let conf: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(conf["version"], env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn saves_and_loads() {
        let dir = scratch("roundtrip");
        let device = DeviceConfig { enabled: false, brightness: 0.5, rect: Some([0.1, 0.2, 0.3, 0.4]) };
        let profile = Profile { effect: Effect::Cycle { speed: 2.0 }, brightness: 0.2 };
        let config = Config {
            brightness: 0.4,
            settings: Settings { fps: 48, openrgb_enabled: true, ..Default::default() },
            devices: BTreeMap::from([("virtual:gpu".to_string(), device)]),
            profiles: BTreeMap::from([("Night".to_string(), profile)]),
            active_profile: Some("Night".into()),
            ..Default::default()
        };
        save(&dir, &config).unwrap();
        assert_eq!(load(&dir), config);
        assert!(!dir.join("config.json.tmp").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_file_gives_defaults() {
        let dir = scratch("missing");
        assert_eq!(load(&dir), Config::default());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_and_unknown_fields_fall_back_to_defaults() {
        let dir = scratch("partial");
        let text = r#"{"brightness":0.5,"later":1,"effect":{"kind":"not_yet"},
            "settings":{"fps":200,"later":true},"devices":{"a":{"enabled":false}},
            "profiles":{"p":{"effect":{"kind":"gone"}}}}"#;
        fs::write(dir.join(FILE), text).unwrap();
        let config = load(&dir);
        assert_eq!(config.brightness, 0.5);
        assert_eq!(config.effect, Effect::default());
        assert_eq!(config.settings, Settings { fps: 60, ..Default::default() });
        assert_eq!(config.devices["a"], DeviceConfig { enabled: false, ..Default::default() });
        assert_eq!(config.profiles["p"], Profile { effect: Effect::default(), brightness: 1.0 });
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn settings_saved_before_auto_detect_still_load() {
        let dir = scratch("older");
        let text = r#"{"settings":{"fps":48,"openrgb_enabled":true,"openrgb_host":"10.0.0.2","openrgb_port":6743,
            "native_drivers":false,"show_virtual":false,"close_to_tray":false,"start_minimized":true,
            "autostart":false}}"#;
        fs::write(dir.join(FILE), text).unwrap();
        let settings = load(&dir).settings;
        assert!(settings.auto_detect);
        assert_eq!((settings.fps, settings.openrgb_port, settings.show_virtual), (48, 6743, false));
        assert!(settings.openrgb_enabled && settings.start_minimized && !settings.close_to_tray);
        // and the switch survives a round trip once it is off
        let off = Config { settings: Settings { auto_detect: false, ..settings }, ..Default::default() };
        save(&dir, &off).unwrap();
        assert!(!load(&dir).settings.auto_detect);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn corrupt_file_is_moved_aside() {
        let dir = scratch("corrupt");
        for bad in [&b"{ nope"[..], &[0xff, 0xfe, b'{', b'}']] {
            fs::write(dir.join(FILE), bad).unwrap();
            assert_eq!(load(&dir), Config::default());
            assert!(!dir.join(FILE).exists());
            assert_eq!(fs::read(dir.join(CORRUPT)).unwrap(), bad);
        }
        fs::remove_dir_all(&dir).unwrap();
    }
}
