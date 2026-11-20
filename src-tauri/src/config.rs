use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use foxden_core::{Effect, Layer, Post};
use foxden_fan::FanConfig;
use foxden_net::NetConfig;
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
    /// Lights on the local network and DMX, off until the user turns them on.
    pub network_enabled: bool,
    pub network: NetConfig,
    /// Serial LED strips the user added.
    pub led_strips: Vec<LedStripConfig>,
    /// LED counts of resizable zones, keyed `device#zone`.
    pub zone_leds: BTreeMap<String, usize>,
    pub close_to_tray: bool,
    pub start_minimized: bool,
    pub autostart: bool,
    /// Watch for hardware being plugged in or removed and for the OpenRGB server.
    pub auto_detect: bool,
    /// Download and install new releases from GitHub.
    pub auto_update: bool,
    pub lighting_module: bool,
    pub fans_module: bool,
    /// Close other lighting and fan software that holds the same devices.
    pub close_conflicts: bool,
    /// Executables of conflicting software the user keeps running, such as SteelSeriesSonar.exe.
    pub keep_running: Vec<String>,
    /// Send anonymous error reports.
    pub send_reports: bool,
    /// A tray icon with the battery level of each wireless device.
    pub battery_tray: bool,
    /// Sonar, Engine and GameSense through SteelSeries GG, off until the user turns it on.
    pub steelseries_gg: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            fps: 30,
            openrgb_enabled: false,
            openrgb_host: "127.0.0.1".into(),
            openrgb_port: 6742,
            native_drivers: true,
            network_enabled: false,
            network: NetConfig::default(),
            led_strips: Vec::new(),
            zone_leds: BTreeMap::new(),
            close_to_tray: true,
            start_minimized: false,
            autostart: true,
            auto_detect: true,
            auto_update: true,
            lighting_module: true,
            fans_module: true,
            close_conflicts: true,
            keep_running: Vec::new(),
            send_reports: false,
            battery_tray: true,
            steelseries_gg: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LedStripConfig {
    pub name: String,
    pub port: String,
    pub baud: u32,
    pub leds: usize,
    /// `adalight`, `tpm2` or `keyboard_visualizer`.
    pub protocol: String,
}

impl Default for LedStripConfig {
    fn default() -> Self {
        LedStripConfig {
            name: "LED strip".into(),
            port: String::new(),
            baud: 115_200,
            leds: 60,
            protocol: "adalight".into(),
        }
    }
}

impl LedStripConfig {
    /// The strip for foxden-hid, if every field is usable.
    pub fn strip(&self) -> Option<foxden_hid::LedStrip> {
        let port = self.port.trim();
        if port.is_empty() || self.baud == 0 || self.leds == 0 {
            return None;
        }
        Some(foxden_hid::LedStrip {
            name: self.name.trim().to_string(),
            port: port.to_string(),
            baud: self.baud,
            leds: self.leds,
            protocol: foxden_hid::LedProtocol::from_name(&self.protocol)?,
        })
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
        DeviceConfig {
            enabled: true,
            brightness: 1.0,
            rect: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    #[serde(default, deserialize_with = "lenient")]
    pub effect: Effect,
    #[serde(default, deserialize_with = "lenient_list")]
    pub layers: Vec<Layer>,
    #[serde(default, deserialize_with = "lenient_map")]
    pub overrides: BTreeMap<String, Effect>,
    #[serde(default = "full")]
    pub brightness: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    #[serde(deserialize_with = "lenient")]
    pub effect: Effect,
    #[serde(deserialize_with = "lenient_list")]
    pub layers: Vec<Layer>,
    /// Devices with an effect of their own.
    #[serde(deserialize_with = "lenient_map")]
    pub overrides: BTreeMap<String, Effect>,
    pub brightness: f32,
    pub settings: Settings,
    pub devices: BTreeMap<String, DeviceConfig>,
    pub profiles: BTreeMap<String, Profile>,
    pub active_profile: Option<String>,
    /// The user's fan config, kept while the Fans module is off.
    pub fan_backup: Option<FanConfig>,
    /// Conflicting executables seen so far, so they can be kept running when they are not.
    #[serde(deserialize_with = "lenient_list")]
    pub known_conflicts: Vec<crate::conflicts::Known>,
    /// The standalone audio mixer's routes, volumes and EQ.
    #[serde(deserialize_with = "lenient")]
    pub audio: crate::mixer::AudioConfig,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            effect: Effect::default(),
            layers: Vec::new(),
            overrides: BTreeMap::new(),
            brightness: 1.0,
            settings: Settings::default(),
            devices: BTreeMap::new(),
            profiles: BTreeMap::new(),
            active_profile: None,
            fan_backup: None,
            known_conflicts: Vec::new(),
            audio: Default::default(),
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

/// Keeps every entry that reads, dropping the rest.
fn lenient_list<'de, D: Deserializer<'de>, T: DeserializeOwned>(d: D) -> Result<Vec<T>, D::Error> {
    let values = Vec::<serde_json::Value>::deserialize(d).unwrap_or_default();
    Ok(values
        .into_iter()
        .filter_map(|v| serde_json::from_value(v).ok())
        .collect())
}

fn lenient_map<'de, D: Deserializer<'de>, T: DeserializeOwned>(
    d: D,
) -> Result<BTreeMap<String, T>, D::Error> {
    let values = BTreeMap::<String, serde_json::Value>::deserialize(d).unwrap_or_default();
    Ok(values
        .into_iter()
        .filter_map(|(k, v)| Some((k, serde_json::from_value(v).ok()?)))
        .collect())
}

/// The app's identifier. The uninstaller's "delete application data" box removes the folder of that name.
const FOLDER: &str = "com.foxden.desktop";

/// Where everything the app keeps goes: `FOXDEN_CONFIG_DIR` if set, otherwise `%APPDATA%\com.foxden.desktop`.
pub fn dir() -> PathBuf {
    match std::env::var_os("FOXDEN_CONFIG_DIR") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => {
            let appdata = PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default());
            let dir = appdata.join(FOLDER);
            adopt(&appdata.join(OLD_FOLDER), &dir);
            dir
        }
    }
}

/// The folder the app used while it was called FoxRGB.
const OLD_FOLDER: &str = "com.foxrgb.desktop";

/// Copies an older install's folder over once, the first time the new one is missing.
fn adopt(old: &Path, new: &Path) {
    if new.exists() || !old.is_dir() {
        return;
    }
    if copy_tree(old, new).is_err() {
        let _ = fs::remove_dir_all(new);
    }
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// The web view's own data. Left to itself it would go to a second folder that `FOXDEN_CONFIG_DIR` does not move.
pub fn webview_dir() -> PathBuf {
    dir().join("webview")
}

/// Never fails: a missing file gives the defaults, a corrupt one is renamed aside first.
pub fn load(dir: &Path) -> Config {
    let path = dir.join(FILE);
    let Ok(bytes) = fs::read(&path) else {
        return Config::default();
    };
    let read = serde_json::from_slice::<serde_json::Value>(&bytes).and_then(|mut value| {
        upgrade(&mut value);
        serde_json::from_value::<Config>(value)
    });
    let mut config: Config = read.unwrap_or_else(|e| {
        crate::log::warn(
            "config",
            format!("{FILE} could not be read and was set aside: {e}"),
        );
        let _ = fs::rename(&path, dir.join(CORRUPT));
        Config::default()
    });
    config.settings.fps = config.settings.fps.clamp(10, 60);
    config.brightness = config.brightness.clamp(0.0, 1.0);
    config
}

/// Brings older settings forward. Key ripple used to be a base effect, and 0.6 had one post effect:
/// both become layers. Layers missing newer fields get their kind's defaults.
fn upgrade(config: &mut serde_json::Value) {
    use serde_json::{Value, json};
    let layer = |mut effect: Value, opacity: Value, blend: Value| {
        if effect["rainbow"] == true {
            effect["colors"] = json!("rainbow");
        }
        json!({"enabled": true, "opacity": opacity, "blend": blend, "scope": "all", "devices": [], "effect": effect})
    };
    let bring = |holder: &mut Value| {
        let e = holder["effect"].clone();
        if e["kind"] == "key_ripple" {
            let ripple = json!({"kind": "press_ripple", "color": e["color"],
                "speed": e["speed"], "width": e["width"], "fade": e["fade"]});
            holder["effect"] = json!({"kind": "static", "color": e["background"]});
            holder["layers"] = json!([layer(ripple, json!(1.0), json!("normal"))]);
        }
        if let Some(post) = holder.as_object_mut().and_then(|h| h.remove("post"))
            && post.get("kind").is_some_and(|k| k != "none")
            && holder.get("layers").is_none()
        {
            let opacity = post.get("strength").cloned().unwrap_or(json!(1.0));
            let blend = match post.get("blend").and_then(Value::as_str) {
                Some("add") => "add",
                Some("tint") => "tint",
                _ => "normal",
            };
            holder["layers"] = json!([layer(post, opacity, json!(blend))]);
        }
        if let Some(layers) = holder.get_mut("layers").and_then(Value::as_array_mut) {
            for l in layers {
                complete(&mut l["effect"]);
            }
        }
    };
    if !config.is_object() {
        return;
    }
    bring(config);
    if let Some(profiles) = config.get_mut("profiles").and_then(|p| p.as_object_mut()) {
        profiles
            .values_mut()
            .filter(|p| p.is_object())
            .for_each(bring);
    }
}

/// Fills the fields a layer effect lacks from the defaults of its kind, dropping ones it no longer has.
fn complete(effect: &mut serde_json::Value) {
    let kind = effect.get("kind").cloned();
    let Some(default) = Post::catalog()
        .into_iter()
        .filter_map(|i| serde_json::to_value(i.default).ok())
        .find(|d| d.get("kind") == kind.as_ref())
    else {
        return;
    };
    let (Some(defaults), Some(fields)) = (default.as_object(), effect.as_object_mut()) else {
        return;
    };
    fields.retain(|name, _| defaults.contains_key(name));
    for (name, v) in defaults {
        fields.entry(name.clone()).or_insert_with(|| v.clone());
    }
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
    use foxden_core::{Blend, Color, Colors, LayerEffect, Scope};

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::current_exe()
            .unwrap()
            .with_file_name(format!("foxden-test-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_folder_has_the_name_the_uninstaller_removes() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(conf["identifier"], FOLDER);
        // The window is made in main.rs, which gives the web view its folder.
        assert_eq!(conf["app"]["windows"][0]["create"], false);
        assert!(webview_dir().starts_with(dir()));
    }

    // The setup program takes its version from tauri.conf.json, the binaries from Cargo.toml.
    #[test]
    fn the_setup_program_and_the_binaries_carry_one_version() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(conf["version"], env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn saves_and_loads() {
        let dir = scratch("roundtrip");
        let device = DeviceConfig {
            enabled: false,
            brightness: 0.5,
            rect: Some([0.1, 0.2, 0.3, 0.4]),
        };
        let profile = Profile {
            effect: Effect::Cycle { speed: 2.0 },
            layers: Vec::new(),
            overrides: BTreeMap::new(),
            brightness: 0.2,
        };
        let config = Config {
            brightness: 0.4,
            settings: Settings {
                fps: 48,
                openrgb_enabled: true,
                ..Default::default()
            },
            devices: BTreeMap::from([("hid:gpu".to_string(), device)]),
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
    fn older_effects_become_layers() {
        let dir = scratch("ripple");
        let ripple = r#"{"kind":"key_ripple","color":[1,2,3],"background":[4,5,6],"speed":1.5,"width":0.05,"fade":1.0}"#;
        let ghost = r#"{"kind":"press_ghost","color":[9,9,9],"rainbow":true,"hold":0.2,"fade":1.0,"size":1.0,"strength":0.5,"blend":"add"}"#;
        let text = format!(
            r#"{{"effect":{ripple},"profiles":{{"p":{{"effect":{{"kind":"cycle","speed":1}},"post":{ghost}}}}}}}"#
        );
        fs::write(dir.join(FILE), text).unwrap();
        let config = load(&dir);
        assert_eq!(
            config.effect,
            Effect::Static {
                color: Color(4, 5, 6)
            }
        );
        let [layer] = config.layers.as_slice() else {
            panic!("{:?}", config.layers)
        };
        let LayerEffect::Post(Post::PressRipple {
            color,
            speed,
            rings,
            ..
        }) = layer.effect
        else {
            panic!()
        };
        assert_eq!((color, speed, rings), (Color(1, 2, 3), 1.5, 1.0));
        let [layer] = config.profiles["p"].layers.as_slice() else {
            panic!()
        };
        assert_eq!((layer.opacity, layer.blend), (0.5, Blend::Add));
        let LayerEffect::Post(Post::PressGhost {
            color,
            colors,
            hold,
            ..
        }) = layer.effect
        else {
            panic!()
        };
        assert_eq!(
            (color, colors, hold),
            (Color(9, 9, 9), Colors::Rainbow, 0.2)
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_layer_that_does_not_read_is_dropped_alone() {
        let dir = scratch("layers");
        let text = r#"{"layers":[{"kind":"what"},{"enabled":true,"opacity":1,"blend":"screen","scope":"keyboards","effect":{"kind":"rainbow","speed":1,"scale":1,"angle":0}}],
            "overrides":{"a":{"kind":"gone"},"b":{"kind":"static","color":[1,1,1]}}}"#;
        fs::write(dir.join(FILE), text).unwrap();
        let config = load(&dir);
        assert_eq!(config.layers.len(), 1);
        assert_eq!(config.layers[0].scope, Scope::Keyboards);
        assert_eq!(config.overrides.keys().collect::<Vec<_>>(), ["b"]);
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
        assert_eq!(
            config.settings,
            Settings {
                fps: 60,
                ..Default::default()
            }
        );
        assert_eq!(
            config.devices["a"],
            DeviceConfig {
                enabled: false,
                ..Default::default()
            }
        );
        assert_eq!(
            config.profiles["p"],
            Profile {
                effect: Effect::default(),
                layers: Vec::new(),
                overrides: BTreeMap::new(),
                brightness: 1.0
            }
        );
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
        assert!(settings.lighting_module && settings.fans_module && settings.close_conflicts);
        assert_eq!(
            (settings.fps, settings.openrgb_port, settings.native_drivers),
            (48, 6743, false)
        );
        assert!(settings.openrgb_enabled && settings.start_minimized && !settings.close_to_tray);
        // and the switch survives a round trip once it is off
        let off = Config {
            settings: Settings {
                auto_detect: false,
                ..settings
            },
            ..Default::default()
        };
        save(&dir, &off).unwrap();
        assert!(!load(&dir).settings.auto_detect);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn network_lights_are_off_and_keep_their_devices() {
        let dir = scratch("network");
        assert!(!Settings::default().network_enabled);
        let text = r#"{"settings":{"network_enabled":true,"network":{"hue":[{"ip":"192.168.1.40","username":"u"}],"e131":[{"num_leds":30}]}}}"#;
        fs::write(dir.join(FILE), text).unwrap();
        let settings = load(&dir).settings;
        assert!(settings.network_enabled);
        assert_eq!(settings.network.hue[0].username, "u");
        assert_eq!(settings.network.e131[0].universe_size, 512);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn strips_and_zone_lengths_round_trip() {
        let dir = scratch("strips");
        fs::write(dir.join(FILE), r#"{"settings":{"fps":40}}"#).unwrap();
        let old = load(&dir).settings;
        assert!(old.led_strips.is_empty() && old.zone_leds.is_empty());
        let text = r#"{"settings":{"led_strips":[{"port":"COM4"},
            {"name":"Desk","port":"COM5","baud":9600,"leds":30,"protocol":"tpm2"}],
            "zone_leds":{"hid:asus-aura-usb-x#1":24}}}"#;
        fs::write(dir.join(FILE), text).unwrap();
        let config = load(&dir);
        let s = &config.settings;
        assert_eq!(
            s.led_strips[0],
            LedStripConfig {
                port: "COM4".into(),
                ..Default::default()
            }
        );
        let desk = s.led_strips[1].strip().unwrap();
        assert_eq!((desk.baud, desk.leds), (9600, 30));
        assert_eq!(desk.protocol, foxden_hid::LedProtocol::Tpm2);
        assert_eq!(s.zone_leds["hid:asus-aura-usb-x#1"], 24);
        save(&dir, &config).unwrap();
        assert_eq!(load(&dir), config);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unusable_strips_are_left_out() {
        let ok = LedStripConfig {
            port: "COM3".into(),
            ..Default::default()
        };
        assert_eq!(ok.strip().unwrap().leds, 60);
        let bad = [
            LedStripConfig::default(),
            LedStripConfig {
                leds: 0,
                ..ok.clone()
            },
            LedStripConfig {
                protocol: "basic_i2c".into(),
                ..ok.clone()
            },
        ];
        assert!(bad.iter().all(|s| s.strip().is_none()));
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

    #[test]
    fn the_audio_mixer_survives_a_restart_and_a_bad_block() {
        let dir = scratch("audio");
        let mut config = Config::default();
        config.audio.output = "Speakers".into();
        config.audio.chat_mix = -0.4;
        config.audio.running = true;
        config.audio.channels[0].eq[2].gain_db = 4.5;
        save(&dir, &config).unwrap();
        assert_eq!(load(&dir), config);
        fs::write(dir.join(FILE), r#"{"brightness":0.5,"audio":{"channels":"nope"}}"#).unwrap();
        let read = load(&dir);
        assert_eq!((read.brightness, read.audio), (0.5, Default::default()));
        assert!(!dir.join(CORRUPT).exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
