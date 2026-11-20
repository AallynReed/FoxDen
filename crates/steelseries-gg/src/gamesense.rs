//! GameSense: the public lighting/OLED/tactile event API GG serves on coreProps.json `address`.

use crate::discovery::DEFAULT_TIMEOUT;
use crate::error::{Error, Result};
use crate::http::Http;
use serde_json::{Value, json};

pub const DEINIT_TIMER_MIN_MS: u32 = 1000;
pub const DEINIT_TIMER_MAX_MS: u32 = 60000;

/// Game and event names: not empty, only `A-Z`, `0-9`, `-` and `_` (GameName.CheckValidity).
pub fn validate_name(name: &str) -> Result<&str> {
    if name.is_empty() {
        return Err(Error::Invalid("name is empty".into()));
    }
    if name.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-' || b == b'_') {
        Ok(name)
    } else {
        Err(Error::Invalid(format!("{name}: only A-Z, 0-9, - and _ are allowed")))
    }
}

#[derive(Debug, Clone)]
pub struct GameMetadata {
    pub game: String,
    pub display_name: String,
    pub developer: String,
    /// How long GG keeps the game's effects after the last event or heartbeat.
    pub deinitialize_timer_length_ms: Option<u32>,
}

#[derive(Clone)]
pub struct GameSenseClient {
    base: String,
    http: Http,
}

impl GameSenseClient {
    pub fn new(base: &str) -> Self {
        Self { base: base.trim_end_matches('/').to_string(), http: Http::new(None, DEFAULT_TIMEOUT) }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn post(&self, route: &str, body: &Value) -> Result<Value> {
        self.http.send_json("POST", &format!("{}/{}", self.base, route.trim_start_matches('/')), Some(body))
    }

    pub fn ping(&self) -> Result<Value> {
        self.http.get_json(&format!("{}/ping", self.base))
    }

    pub fn register_game(&self, m: &GameMetadata) -> Result<Value> {
        validate_name(&m.game)?;
        let mut body = json!({ "game": m.game, "game_display_name": m.display_name, "developer": m.developer });
        if let Some(t) = m.deinitialize_timer_length_ms {
            if !(DEINIT_TIMER_MIN_MS..=DEINIT_TIMER_MAX_MS).contains(&t) {
                return Err(Error::Invalid(format!("deinitialize timer {t} ms outside 1000..60000")));
            }
            body["deinitialize_timer_length_ms"] = json!(t);
        }
        self.post("game_metadata", &body)
    }

    /// An event without handlers, for apps that let the person bind it in GG.
    pub fn register_event(&self, game: &str, event: &str, min: i32, max: i32, icon_id: u32, value_optional: bool) -> Result<Value> {
        validate_name(game)?;
        validate_name(event)?;
        self.post("register_game_event", &json!({
            "game": game, "event": event, "min_value": min, "max_value": max,
            "icon_id": icon_id, "value_optional": value_optional
        }))
    }

    /// An event with handlers (`handlers` is the GameSense handler array: zones, colors, modes).
    pub fn bind_event(&self, game: &str, event: &str, min: i32, max: i32, icon_id: u32, handlers: Value) -> Result<Value> {
        validate_name(game)?;
        validate_name(event)?;
        self.post("bind_game_event", &json!({
            "game": game, "event": event, "min_value": min, "max_value": max,
            "icon_id": icon_id, "handlers": handlers
        }))
    }

    pub fn send_event(&self, game: &str, event: &str, value: i32, frame: Option<Value>) -> Result<Value> {
        let mut data = json!({ "value": value });
        if let Some(f) = frame {
            data["frame"] = f;
        }
        self.post("game_event", &json!({ "game": game, "event": event, "data": data }))
    }

    pub fn send_events(&self, game: &str, events: &[(&str, i32)]) -> Result<Value> {
        let list: Vec<Value> = events.iter().map(|(e, v)| json!({ "event": e, "data": { "value": v } })).collect();
        self.post("multiple_game_events", &json!({ "game": game, "events": list }))
    }

    pub fn heartbeat(&self, game: &str) -> Result<Value> {
        self.post("game_heartbeat", &json!({ "game": game }))
    }

    pub fn stop_game(&self, game: &str) -> Result<Value> {
        self.post("stop_game", &json!({ "game": game }))
    }

    pub fn remove_event(&self, game: &str, event: &str) -> Result<Value> {
        self.post("remove_game_event", &json!({ "game": game, "event": event }))
    }

    pub fn remove_game(&self, game: &str) -> Result<Value> {
        self.post("remove_game", &json!({ "game": game }))
    }
}

/// A full-device solid colour handler that follows the event value as a 0..100 percent.
pub fn color_handler(device_type: &str, zone: &str, rgb: (u8, u8, u8)) -> Value {
    json!([{
        "device-type": device_type,
        "zone": zone,
        "mode": "percent",
        "color": { "red": rgb.0, "green": rgb.1, "blue": rgb.2 }
    }])
}

/// A handler whose colour comes from each event's frame: send `frame = {frame_key: {red, green, blue}}`
/// (mode `context-color`, game-integration/lib/user-config.lsp).
pub fn context_color_handler(device_type: &str, zone: &str, frame_key: &str) -> Value {
    json!([{
        "device-type": device_type,
        "zone": zone,
        "mode": "context-color",
        "context-frame-key": frame_key
    }])
}

pub fn color_frame(frame_key: &str, rgb: (u8, u8, u8)) -> Value {
    json!({ frame_key: { "red": rgb.0, "green": rgb.1, "blue": rgb.2 } })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(validate_name("FOXDEN").is_ok());
        assert!(validate_name("MY-GAME_2").is_ok());
        assert!(validate_name("foxden").is_err());
        assert!(validate_name("").is_err());
        assert!(validate_name("A B").is_err());
    }
}
