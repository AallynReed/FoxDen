//! Client for Sonar's local HTTP API (SoundStage.Api controllers).

use crate::discovery::DEFAULT_TIMEOUT;
use crate::error::{Error, Result};
use crate::http::Http;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const VOLUME_MAX: f32 = 1.0;
/// Step Sonar uses for volume up/down shortcuts.
pub const VOLUME_STEP: f32 = 0.05;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    Classic,
    Stream,
}

impl Mode {
    pub fn api(self) -> &'static str {
        match self {
            Mode::Classic => "classic",
            Mode::Stream => "stream",
        }
    }
}

/// Sonar's virtual devices (DeviceRole without None/Stream).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Channel {
    Game,
    ChatRender,
    ChatCapture,
    Media,
    Aux,
}

impl Channel {
    pub const ALL: [Channel; 5] = [Channel::Game, Channel::ChatRender, Channel::Media, Channel::Aux, Channel::ChatCapture];

    pub fn api(self) -> &'static str {
        match self {
            Channel::Game => "game",
            Channel::ChatRender => "chatRender",
            Channel::ChatCapture => "chatCapture",
            Channel::Media => "media",
            Channel::Aux => "aux",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Channel::Game => "Game",
            Channel::ChatRender => "Chat",
            Channel::ChatCapture => "Mic",
            Channel::Media => "Media",
            Channel::Aux => "Aux",
        }
    }

    /// Id used by `/classicRedirections` (RedirectionId).
    pub fn redirection_id(self) -> &'static str {
        match self {
            Channel::Game => "game",
            Channel::ChatRender => "chat",
            Channel::ChatCapture => "mic",
            Channel::Media => "media",
            Channel::Aux => "aux",
        }
    }

    pub fn is_capture(self) -> bool {
        self == Channel::ChatCapture
    }
}

/// The two mixes of Stream mode (StreamRedirectionId without Mic).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StreamMix {
    Streaming,
    Monitoring,
}

impl StreamMix {
    pub fn api(self) -> &'static str {
        match self {
            StreamMix::Streaming => "streaming",
            StreamMix::Monitoring => "monitoring",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DataFlow {
    Render,
    Capture,
    All,
}

impl DataFlow {
    pub fn api(self) -> &'static str {
        match self {
            DataFlow::Render => "render",
            DataFlow::Capture => "capture",
            DataFlow::All => "all",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct VolumeMute {
    #[serde(default)]
    pub volume: f32,
    #[serde(default)]
    pub muted: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VolumeSlot {
    #[serde(default)]
    pub classic: Option<VolumeMute>,
    #[serde(default)]
    pub stream: BTreeMap<String, VolumeMute>,
}

impl VolumeSlot {
    pub fn stream(&self, mix: StreamMix) -> VolumeMute {
        self.stream.get(mix.api()).copied().unwrap_or_default()
    }
}

/// `GET /volumeSettings/classic` and `/volumeSettings/streamer`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VolumeSettings {
    #[serde(default)]
    pub masters: VolumeSlot,
    #[serde(default)]
    pub devices: BTreeMap<String, VolumeSlot>,
}

impl VolumeSettings {
    pub fn channel(&self, c: Channel) -> VolumeSlot {
        self.devices.get(c.api()).cloned().unwrap_or_default()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatMix {
    pub balance: f32,
    #[serde(default)]
    pub state: String,
}

impl ChatMix {
    /// Only "enabled" accepts changes; otherwise the PUT answers 403.
    pub fn is_writable(&self) -> bool {
        self.state.eq_ignore_ascii_case("enabled")
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassicRedirection {
    pub id: String,
    #[serde(default)]
    pub device_id: String,
    #[serde(default)]
    pub is_running: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamRedirectionStatus {
    pub role: String,
    pub is_enabled: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamRedirection {
    pub stream_redirection_id: String,
    #[serde(default)]
    pub device_id: String,
    #[serde(default)]
    pub status: Vec<StreamRedirectionStatus>,
    #[serde(default)]
    pub is_running: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDevice {
    pub friendly_name: String,
    pub id: String,
    pub data_flow: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub channels: u32,
    #[serde(default)]
    pub default_role: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub is_vad: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceOutSetting {
    pub volume: f32,
    pub mute: bool,
    pub feature: String,
}

/// Headset/line output of devices that have both (Feature HeadphoneOut, LineOut).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceOut {
    #[serde(default)]
    pub selected_device_out: String,
    #[serde(default)]
    pub device_out_settings: Vec<DeviceOutSetting>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusConfig {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusDevice {
    pub id: String,
    pub data_flow: String,
    pub virtual_audio_device: String,
    #[serde(default)]
    pub is_channel_enabled: bool,
    #[serde(default)]
    pub stream_redirection_device: Option<String>,
    pub config: Option<StatusConfig>,
}

/// `GET /v1/status`, the public summary.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub is_enabled: bool,
    pub is_onboarding_done: bool,
    pub mode: String,
    pub are_endpoints_configured: bool,
    #[serde(default)]
    pub devices: Vec<StatusDevice>,
}

#[derive(Clone)]
pub struct SonarClient {
    base: String,
    http: Http,
}

impl SonarClient {
    /// `base` is Sonar's `webServerAddress`, e.g. `http://127.0.0.1:54578`.
    pub fn new(base: &str) -> Self {
        Self { base: base.trim_end_matches('/').to_string(), http: Http::new(None, DEFAULT_TIMEOUT) }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.base, path.trim_start_matches('/'))
    }

    pub fn get_raw(&self, path: &str) -> Result<Value> {
        self.http.get_json(&self.url(path))
    }

    pub fn send_raw(&self, method: &str, path: &str, body: Option<&Value>) -> Result<String> {
        self.http.send(method, &self.url(path), body)
    }

    fn put(&self, path: &str) -> Result<()> {
        self.http.send("PUT", &self.url(path), None).map(|_| ())
    }

    pub fn status(&self) -> Result<Status> {
        self.http.get_json(&self.url("v1/status"))
    }

    pub fn mode(&self) -> Result<Mode> {
        self.http.get_json(&self.url("mode"))
    }

    pub fn set_mode(&self, mode: Mode) -> Result<()> {
        self.put(&format!("mode/{}", mode.api()))
    }

    pub fn classic_volumes(&self) -> Result<VolumeSettings> {
        self.http.get_json(&self.url("volumeSettings/classic"))
    }

    pub fn streamer_volumes(&self) -> Result<VolumeSettings> {
        self.http.get_json(&self.url("volumeSettings/streamer"))
    }

    pub fn set_volume(&self, channel: Channel, volume: f32) -> Result<()> {
        self.put(&format!("volumeSettings/classic/{}/Volume/{}", channel.api(), vol(volume)?))
    }

    pub fn set_mute(&self, channel: Channel, muted: bool) -> Result<()> {
        self.put(&format!("volumeSettings/classic/{}/Mute/{muted}", channel.api()))
    }

    pub fn set_master_volume(&self, volume: f32) -> Result<()> {
        self.put(&format!("volumeSettings/classic/Master/Volume/{}", vol(volume)?))
    }

    pub fn set_master_mute(&self, muted: bool) -> Result<()> {
        self.put(&format!("volumeSettings/classic/Master/Mute/{muted}"))
    }

    pub fn set_stream_volume(&self, mix: StreamMix, channel: Channel, volume: f32) -> Result<()> {
        self.put(&format!("volumeSettings/streamer/{}/{}/volume/{}", mix.api(), channel.api(), vol(volume)?))
    }

    pub fn set_stream_mute(&self, mix: StreamMix, channel: Channel, muted: bool) -> Result<()> {
        self.put(&format!("volumeSettings/streamer/{}/{}/isMuted/{muted}", mix.api(), channel.api()))
    }

    pub fn set_stream_master_volume(&self, mix: StreamMix, volume: f32) -> Result<()> {
        self.put(&format!("volumeSettings/streamer/{}/master/volume/{}", mix.api(), vol(volume)?))
    }

    pub fn set_stream_master_mute(&self, mix: StreamMix, muted: bool) -> Result<()> {
        self.put(&format!("volumeSettings/streamer/{}/master/isMuted/{muted}", mix.api()))
    }

    pub fn chat_mix(&self) -> Result<ChatMix> {
        self.http.get_json(&self.url("v1/chatMix"))
    }

    /// `PUT /v1/chatMix?balance=` (-1 game … 1 chat). 403 means the chat mix is read-only right now.
    pub fn set_chat_mix(&self, balance: f32) -> Result<ChatMix> {
        let b = crate::chatmix::validate(balance).map_err(Error::Invalid)?;
        self.http.send_json("PUT", &self.url(&format!("v1/chatMix?balance={b}")), None)
    }

    pub fn classic_redirections(&self) -> Result<Vec<ClassicRedirection>> {
        self.http.get_json(&self.url("classicRedirections"))
    }

    /// Sends one Sonar channel to an output (render) or input (capture) device id.
    pub fn set_classic_redirection(&self, channel: Channel, device_id: &str) -> Result<()> {
        self.put(&format!("classicRedirections/{}/deviceId/{}", channel.redirection_id(), enc(device_id)))
    }

    /// Sends every render channel to one output device.
    pub fn set_all_render_redirections(&self, device_id: &str) -> Result<()> {
        self.put(&format!("classicRedirections/render/deviceId/{}", enc(device_id)))
    }

    pub fn stream_redirections(&self) -> Result<Vec<StreamRedirection>> {
        self.http.get_json(&self.url("streamRedirections"))
    }

    pub fn set_stream_redirection_device(&self, mix: &str, device_id: &str) -> Result<()> {
        self.put(&format!("streamRedirections/{mix}/deviceId/{}", enc(device_id)))
    }

    pub fn set_stream_redirection_enabled(&self, mix: StreamMix, channel: Channel, enabled: bool) -> Result<()> {
        self.put(&format!("streamRedirections/{}/redirections/{}/isEnabled/{enabled}", mix.api(), channel.api()))
    }

    pub fn set_stream_monitoring(&self, enabled: bool) -> Result<()> {
        self.put(&format!("streamRedirections/isStreamMonitoringEnabled/{enabled}"))
    }

    pub fn audio_devices(&self, flow: DataFlow) -> Result<Vec<AudioDevice>> {
        self.http.get_json(&self.url(&format!("audioDevices?deviceDataFlow={}", flow.api())))
    }

    pub fn device_out(&self) -> Result<DeviceOut> {
        self.http.get_json(&self.url("deviceOut"))
    }

    pub fn set_device_out_path(&self, feature: &str) -> Result<()> {
        self.put(&format!("deviceOut/audioPath/{feature}"))
    }

    pub fn set_device_out_volume(&self, feature: &str, volume: f32) -> Result<()> {
        self.put(&format!("deviceOut/volume/{feature}/{}", vol(volume)?))
    }

    pub fn set_device_out_mute(&self, feature: &str, muted: bool) -> Result<()> {
        self.put(&format!("deviceOut/mute/{feature}/{muted}"))
    }

    /// Saved EQ/effect presets for one virtual device (`vad` = game, chatRender, chatCapture, media, aux).
    pub fn configs(&self, vad: Channel) -> Result<Value> {
        self.http.get_json(&self.url(&format!("configs?vad={}", vad.api())))
    }

    pub fn selected_configs(&self) -> Result<Value> {
        self.http.get_json(&self.url("configs/selected"))
    }

    pub fn select_config(&self, id: &str) -> Result<()> {
        self.put(&format!("configs/{}/select", enc(id)))
    }

    pub fn fallback_enabled(&self) -> Result<bool> {
        self.http.get_json(&self.url("fallbackSettings/isEnabled"))
    }

    pub fn set_fallback_enabled(&self, enabled: bool) -> Result<()> {
        self.put(&format!("fallbackSettings/isEnabled/{enabled}"))
    }

    pub fn link_all(&self, source: Channel) -> Result<()> {
        self.put(&format!("linkAll/link/{}", source.api()))
    }

    pub fn unlink_all(&self) -> Result<()> {
        self.put("linkAll/unlink")
    }
}

fn vol(v: f32) -> Result<String> {
    if !(0.0..=VOLUME_MAX).contains(&v) || !v.is_finite() {
        return Err(Error::Invalid(format!("volume {v} outside 0..1")));
    }
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    Ok(if s.is_empty() { "0".into() } else { s.to_string() })
}

fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_text() {
        assert_eq!(vol(1.0).unwrap(), "1");
        assert_eq!(vol(0.5).unwrap(), "0.5");
        assert_eq!(vol(0.0).unwrap(), "0");
        assert!(vol(1.2).is_err());
    }

    #[test]
    fn parses_live_shapes() {
        let v: VolumeSettings = serde_json::from_str(r#"{"masters":{"stream":{},"classic":{"volume":1.0,"muted":false}},"devices":{"game":{"stream":{"streaming":{"volume":0.5,"muted":true}},"classic":{"volume":0.25,"muted":false}}}}"#).unwrap();
        assert_eq!(v.channel(Channel::Game).classic.unwrap().volume, 0.25);
        assert!(v.channel(Channel::Game).stream(StreamMix::Streaming).muted);
        let m: Mode = serde_json::from_str("\"classic\"").unwrap();
        assert_eq!(m, Mode::Classic);
    }
}
