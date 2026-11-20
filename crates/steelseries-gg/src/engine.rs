//! Client for SteelSeries Engine's local HTTPS API (the routes GG's own Engine pages use).

use crate::discovery::DEFAULT_TIMEOUT;
use crate::error::{Error, Result};
use crate::http::Http;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const STEELSERIES_VID: u16 = 0x1038;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Firmware {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub incompatible: bool,
    #[serde(default)]
    pub should_update: bool,
    #[serde(default)]
    pub update_in_progress: bool,
    #[serde(default)]
    pub re_pair_required: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PhysicalDevice {
    #[serde(default)]
    pub id: Option<u32>,
    pub product_id: u32,
    #[serde(default, rename = "hexProductId")]
    pub hex_product_id: String,
    pub name: String,
    #[serde(default)]
    pub full_name: String,
    #[serde(default)]
    pub firmware: Firmware,
    #[serde(default)]
    pub bootloader_mode: bool,
}

impl PhysicalDevice {
    /// Engine packs the USB vendor id in the top 16 bits (`0x1038227e`).
    pub fn usb_ids(&self) -> (u16, u16) {
        ((self.product_id >> 16) as u16, (self.product_id & 0xffff) as u16)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConfigurationMeta {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub active: u8,
    #[serde(default)]
    pub last_deployed: u8,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EngineDevice {
    pub id: u32,
    pub product_id: u32,
    #[serde(default)]
    pub connected_product_id: u32,
    pub name: String,
    #[serde(default)]
    pub full_name: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default, rename = "type")]
    pub kind: u32,
    #[serde(default, rename = "deviceTypeName")]
    pub device_type_name: String,
    #[serde(default)]
    pub connected: u8,
    #[serde(default)]
    pub configuration_metadata: Vec<ConfigurationMeta>,
    #[serde(default)]
    pub physical_devices: Vec<PhysicalDevice>,
}

impl EngineDevice {
    pub fn is_connected(&self) -> bool {
        self.connected != 0
    }

    pub fn title(&self) -> &str {
        if !self.display_name.is_empty() { &self.display_name } else if !self.full_name.is_empty() { &self.full_name } else { &self.name }
    }

    pub fn firmware_update_available(&self) -> bool {
        self.physical_devices.iter().any(|p| p.firmware.should_update && !p.firmware.incompatible)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Configuration {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub active: u8,
    /// On-board settings as Engine stores them: a JSON document in a string.
    #[serde(default)]
    pub settings: String,
}

impl Configuration {
    pub fn settings_json(&self) -> Value {
        serde_json::from_str(&self.settings).unwrap_or(Value::Null)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MmDeviceMetadata {
    pub device_id: u32,
    pub mm_device_id: String,
    pub device_name: String,
    #[serde(default)]
    pub supported_features: Vec<String>,
}

impl MmDeviceMetadata {
    /// Headsets that turn Sonar's software chat mix on (`SOFTWARE_CHATMIX_FINITE` / `_INFINITE`).
    pub fn has_chat_mix(&self) -> bool {
        self.supported_features.iter().any(|f| f.starts_with("SOFTWARE_CHATMIX"))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WirelessStatus {
    pub device_id: u32,
    #[serde(default)]
    pub product_id: u32,
    pub device_name: String,
    #[serde(default)]
    pub mm_devices: Vec<String>,
    pub is_connected: bool,
}

#[derive(Deserialize)]
struct Devices<T> {
    devices: Vec<T>,
}

#[derive(Deserialize)]
struct Configurations {
    configurations: Vec<Configuration>,
}

/// Proof that the caller asked the person before a firmware write. Build it only from a confirmation in your UI.
pub struct FirmwareConsent(());

impl FirmwareConsent {
    pub fn confirmed_by_user() -> Self {
        FirmwareConsent(())
    }
}

#[derive(Clone)]
pub struct EngineClient {
    base: String,
    http: Http,
}

impl EngineClient {
    /// `base` is `https://127.0.0.1:<port>`; `cert_der` pins the certificate GG publishes for Engine.
    pub fn new(base: &str, cert_der: Option<Vec<u8>>) -> Self {
        Self { base: base.trim_end_matches('/').to_string(), http: Http::new(cert_der, DEFAULT_TIMEOUT) }
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

    /// Every device Engine knows, connected or not.
    pub fn devices(&self) -> Result<Vec<EngineDevice>> {
        Ok(self.http.get_json::<Devices<EngineDevice>>(&self.url("devices"))?.devices)
    }

    pub fn connected_devices(&self) -> Result<Vec<EngineDevice>> {
        Ok(self.http.get_json::<Devices<EngineDevice>>(&self.url("v1/connectedDevices"))?.devices)
    }

    pub fn device(&self, id: u32) -> Result<Value> {
        self.http.get_json(&self.url(&format!("device/{id}")))
    }

    pub fn mm_devices_metadata(&self) -> Result<Vec<MmDeviceMetadata>> {
        Ok(self.http.get_json::<Devices<MmDeviceMetadata>>(&self.url("v1/mmDevicesMetadata"))?.devices)
    }

    pub fn wireless_status(&self) -> Result<Vec<WirelessStatus>> {
        Ok(self.http.get_json::<Devices<WirelessStatus>>(&self.url("v1/wirelessDeviceConnectionStatus"))?.devices)
    }

    pub fn configurations(&self, device_id: u32) -> Result<Vec<Configuration>> {
        Ok(self.http.get_json::<Configurations>(&self.url(&format!("device/{device_id}/configurations")))?.configurations)
    }

    pub fn configuration(&self, config_id: &str) -> Result<Value> {
        self.http.get_json(&self.url(&format!("configuration/{config_id}")))
    }

    /// Saves new on-board settings into a configuration without sending them to the device.
    pub fn save_configuration(&self, config_id: &str, name: &str, settings: &Value) -> Result<String> {
        let body = json!({ "name": name, "settings": settings.to_string() });
        self.http.send("PUT", &self.url(&format!("configuration/{config_id}/save")), Some(&body))
    }

    /// Writes a saved configuration to the device (lighting, DPI, EQ, sidetone…).
    pub fn deploy_configuration(&self, config_id: &str) -> Result<String> {
        self.http.send("POST", &self.url(&format!("configuration/{config_id}/deploy")), None)
    }

    pub fn global_device_settings(&self, device_id: u32) -> Result<Value> {
        self.http.get_json(&self.url(&format!("device/{device_id}/globalDeviceSettings")))
    }

    /// Reads one value straight from the device (`layout`, `fw_version_stm32`, `get_audio_settings`…).
    pub fn read_from_device(&self, device_id: u32, what: &str) -> Result<Value> {
        let v: Value = self.http.get_json(&self.url(&format!("device/{device_id}/readFromDevice/{what}")))?;
        Ok(inner_json(&v, "read_data"))
    }

    /// Runs a named device function (`get_fw_version`, `read_battery_status`, `engine_fw_versions`…).
    pub fn function(&self, device_id: u32, name: &str) -> Result<Value> {
        if is_destructive_function(name) {
            return Err(Error::Invalid(format!("{name} changes the device; use function_confirmed")));
        }
        self.function_confirmed(device_id, name)
    }

    pub fn function_confirmed(&self, device_id: u32, name: &str) -> Result<Value> {
        let v: Value = self.http.send_json("POST", &self.url(&format!("device/{device_id}/function/{name}")), None)?;
        Ok(v.get("function_data").cloned().unwrap_or(v))
    }

    /// Battery, as the headset pages ask for it.
    pub fn battery(&self, device_id: u32) -> Result<Value> {
        self.function(device_id, "read_battery_status")
            .or_else(|_| self.function(device_id, "get_battery_status"))
    }

    /// Sends a device command document, e.g. `{"charge_status":{}}`.
    pub fn command(&self, device_id: u32, command: &Value) -> Result<Value> {
        let body = json!({ "id": device_id, "command": command.to_string() });
        let v: Value = self.http.send_json("POST", &self.url("device/command"), Some(&body))?;
        Ok(inner_json(&v, "command_data"))
    }

    /// Asks Engine to download and flash the official firmware for this device.
    pub fn start_firmware_update(&self, device_id: u32, physical_device: Option<u32>, _consent: FirmwareConsent) -> Result<String> {
        let path = match physical_device {
            Some(p) => format!("device/{device_id}/physicalDevice/{p}/firmware"),
            None => format!("device/{device_id}/firmware"),
        };
        self.http.send("PUT", &self.url(&path), None)
    }
}

fn inner_json(v: &Value, key: &str) -> Value {
    match v.get(key) {
        Some(Value::String(s)) if !s.is_empty() => serde_json::from_str(s).unwrap_or(Value::String(s.clone())),
        Some(other) => other.clone(),
        None => v.clone(),
    }
}

pub fn is_destructive_function(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    ["restore", "factory", "reset", "save_", "fw_update", "pairing", "calibration", "write"].iter().any(|k| n.contains(k))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_live_device() {
        let j = r#"{"devices":[{"id":358,"product_id":272114302,"connected_product_id":272114337,"name":"arctis_nova_7_gen2_tx","full_name":"Arctis Nova 7 Gen 2","display_name":"Arctis Nova 7","type":3,"deviceTypeName":"Headset","connected":1,"configuration_metadata":[{"id":"8d77","name":"Default","device_id":358,"last_deployed":1,"active":1}],"physical_devices":[{"product_id":272114302,"hexProductId":"0x1038227e","name":"arctis_nova_7_gen2_tx","full_name":"Arctis Nova 7 Gen 2","firmware":{"version":"","incompatible":false,"should_update":false,"update_in_progress":false,"re_pair_required":false}}]}]}"#;
        let d: Devices<EngineDevice> = serde_json::from_str(j).unwrap();
        let dev = &d.devices[0];
        assert_eq!(dev.physical_devices[0].usb_ids(), (0x1038, 0x227e));
        assert!(dev.is_connected());
        assert_eq!(dev.title(), "Arctis Nova 7");
    }

    #[test]
    fn guards_destructive() {
        assert!(is_destructive_function("restore_factory_settings"));
        assert!(!is_destructive_function("read_battery_status"));
    }
}
