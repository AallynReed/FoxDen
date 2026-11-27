//! The tools offered to AI clients. The app checks every argument again.

use serde_json::{Value, json};

const CHANNELS: [&str; 5] = ["game", "chat", "media", "aux", "mic"];

fn tool(name: &str, description: &str, properties: Value, required: &[&str], read_only: bool) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": {"type": "object", "properties": properties, "required": required},
        "annotations": {"readOnlyHint": read_only},
    })
}

fn none() -> Value {
    json!({})
}

pub fn list() -> Vec<Value> {
    let channel = |all: &[&str]| json!({"type": "string", "enum": all, "description": "Mixer channel"});
    vec![
        tool(
            "get_status",
            "FoxDen's version, lighting devices (with their own effects, onboard modes, zones, brightness and layout), the main effect, a summary of the layers over it, brightness, active profile and whether lighting is paused.",
            none(),
            &[],
            true,
        ),
        tool(
            "list_effects",
            "Every main effect kind and every layer effect kind with its parameters: name, type and range or options.",
            none(),
            &[],
            true,
        ),
        tool(
            "set_effect",
            "Sets the lighting effect, for all devices or one device. Parameters left out keep their current or default value; colours are \"#rrggbb\" or [r,g,b].",
            json!({
                "kind": {"type": "string", "description": "Effect kind from list_effects, such as \"static\" or \"rainbow\""},
                "params": {"type": "object", "description": "Parameter values by name", "additionalProperties": true},
                "device": {"type": "string", "description": "Device id or name from get_status; leave out for the main effect"},
            }),
            &["kind"],
            false,
        ),
        tool(
            "set_brightness",
            "Sets the overall lighting brightness.",
            json!({"value": {"type": "number", "minimum": 0, "maximum": 100, "description": "Percent"}}),
            &["value"],
            false,
        ),
        tool("pause", "Pauses the lighting.", none(), &[], false),
        tool("resume", "Resumes the lighting.", none(), &[], false),
        tool("list_profiles", "Saved lighting profiles and the active one.", none(), &[], true),
        tool(
            "apply_profile",
            "Applies a saved lighting profile.",
            json!({"name": {"type": "string"}}),
            &["name"],
            false,
        ),
        tool(
            "save_profile",
            "Saves the current lighting as a profile.",
            json!({
                "name": {"type": "string", "maxLength": 40},
                "replace": {"type": "boolean", "description": "Overwrite a profile with this name"},
            }),
            &["name"],
            false,
        ),
        tool(
            "get_profile",
            "A saved profile in full: main effect, layers, per-device effects and brightness.",
            json!({"name": {"type": "string"}}),
            &["name"],
            true,
        ),
        tool("delete_profile", "Deletes a saved profile.", json!({"name": {"type": "string"}}), &["name"], false),
        tool(
            "rename_profile",
            "Renames a saved profile.",
            json!({
                "name": {"type": "string"},
                "to": {"type": "string", "maxLength": 40},
                "replace": {"type": "boolean", "description": "Overwrite a profile already named `to`"},
            }),
            &["name", "to"],
            false,
        ),
        tool(
            "list_layers",
            "The layers drawn over the main effect, in order with the last on top: index, effect and its parameters, opacity, blend, which devices and when it shows.",
            none(),
            &[],
            true,
        ),
        tool(
            "add_layer",
            "Adds a layer over the main effect, on top unless position says otherwise. For keys lit while a game is in front, use kind \"key_lights\" or \"key_colours\" with when \"front\" and apps [\"Game.exe\"]; the layer then shows only while that app is in front and goes away after.",
            layer_schema(true),
            &["kind"],
            false,
        ),
        tool(
            "update_layer",
            "Changes a layer. Fields left out stay as they are; params are laid over the layer's current ones. Giving a different kind replaces its effect.",
            layer_schema(false),
            &["index"],
            false,
        ),
        tool(
            "remove_layer",
            "Removes a layer.",
            json!({"index": {"type": "integer", "minimum": 0}}),
            &["index"],
            false,
        ),
        tool(
            "move_layer",
            "Moves a layer to another place in the stack; the last is drawn on top.",
            json!({"index": {"type": "integer", "minimum": 0}, "to": {"type": "integer", "minimum": 0}}),
            &["index", "to"],
            false,
        ),
        tool(
            "toggle_layer",
            "Turns a layer on or off; without enabled it flips.",
            json!({"index": {"type": "integer", "minimum": 0}, "enabled": {"type": "boolean"}}),
            &["index"],
            false,
        ),
        tool(
            "clear_device_effect",
            "Has a device follow the main effect again, removing the effect set_effect gave it.",
            json!({"device": {"type": "string", "description": "Device id or name from get_status"}}),
            &["device"],
            false,
        ),
        tool(
            "set_device",
            "Turns a lighting device on or off, sets its own brightness, or moves it on the effect canvas. Fields left out stay as they are.",
            json!({
                "device": {"type": "string", "description": "Device id or name from get_status"},
                "enabled": {"type": "boolean"},
                "brightness": {"type": "number", "minimum": 0, "maximum": 100, "description": "Percent"},
                "layout": {"type": "array", "items": {"type": "number"}, "minItems": 4, "maxItems": 4,
                    "description": "[x, y, width, height] on the effect canvas, each 0 to 1"},
            }),
            &["device"],
            false,
        ),
        tool("rescan_devices", "Reconnects the lighting drivers and looks for devices again.", none(), &[], false),
        tool(
            "list_keys",
            "Every keyboard key by the name the key tools take, with its code and place.",
            none(),
            &[],
            true,
        ),
        tool(
            "get_key_map",
            "A device's LEDs: each LED's index, position, and the key (or, on zone boards, the keys) it lights.",
            json!({"device": {"type": "string", "description": "Device id or name from get_status"}}),
            &["device"],
            true,
        ),
        tool("list_key_maps", "Saved key colour maps.", none(), &[], true),
        tool(
            "save_key_map",
            "Saves a key colour map, which the Effects tab can load into a key colours layer.",
            json!({
                "name": {"type": "string", "maxLength": 40},
                "keys": {"type": "object", "additionalProperties": {"type": "string"},
                    "description": "Key name from list_keys to colour \"#rrggbb\""},
                "replace": {"type": "boolean"},
            }),
            &["name", "keys"],
            false,
        ),
        tool("delete_key_map", "Deletes a saved key colour map.", json!({"name": {"type": "string"}}), &["name"], false),
        tool(
            "list_apps",
            "Executables with a window open now, for a layer's apps.",
            none(),
            &[],
            true,
        ),
        tool(
            "get_settings",
            "The app settings set_settings can change, those that need approval on screen, the languages, and how the last request for approval ended.",
            none(),
            &[],
            true,
        ),
        tool(
            "set_settings",
            "Changes app settings: fps, language, close_to_tray, start_minimized, battery_tray, battery_alerts and auto_detect at once. Changing autostart, lighting_module or native_drivers asks the person at this PC on screen first and needs a reason; nothing in the call changes unless they approve. The call waits up to 50 seconds; get_settings shows the outcome later under last_request.",
            json!({
                "autostart": {"type": "boolean", "description": "Start FoxDen with Windows; needs approval"},
                "lighting_module": {"type": "boolean", "description": "FoxDen drives lighting at all; needs approval"},
                "native_drivers": {"type": "boolean", "description": "FoxDen's own device drivers; needs approval"},
                "reason": {"type": "string", "maxLength": 300, "description": "For settings that need approval: one or two sentences for the person approving"},
                "fps": {"type": "integer", "minimum": 10, "maximum": 60},
                "language": {"type": "string", "description": "\"auto\" or a code from get_settings"},
                "close_to_tray": {"type": "boolean"},
                "start_minimized": {"type": "boolean"},
                "battery_tray": {"type": "boolean"},
                "battery_alerts": {"type": "boolean"},
                "auto_detect": {"type": "boolean", "description": "Watch for devices being plugged in"},
            }),
            &[],
            false,
        ),
        tool(
            "get_sensors",
            "Temperatures and fan speeds and duty from FoxDen's fan service. Read only.",
            none(),
            &[],
            true,
        ),
        tool(
            "get_audio",
            "The audio mixer: output, master volume, and each channel's volume, mute and apps.",
            none(),
            &[],
            true,
        ),
        tool(
            "set_channel_volume",
            "Sets a mixer channel's volume.",
            json!({
                "channel": channel(&CHANNELS),
                "volume": {"type": "number", "minimum": 0, "maximum": 100, "description": "Percent"},
            }),
            &["channel", "volume"],
            false,
        ),
        tool(
            "set_channel_mute",
            "Mutes or unmutes a mixer channel.",
            json!({"channel": channel(&CHANNELS), "muted": {"type": "boolean"}}),
            &["channel", "muted"],
            false,
        ),
        tool(
            "assign_app",
            "Moves an app's sound to a mixer channel.",
            json!({
                "app": {"type": "string", "description": "Executable name, such as \"game.exe\""},
                "channel": channel(&CHANNELS[..4]),
            }),
            &["app", "channel"],
            false,
        ),
        tool(
            "get_batteries",
            "Battery levels of wireless devices.",
            none(),
            &[],
            true,
        ),
        tool(
            "get_fan_config",
            "The fan curves and fan rules in use, the fans that can be controlled and the temperature sensors, in the shape propose_fan_curves takes. Duties are percent, temperatures °C, ramp is percent per second. last_proposal tells how the latest proposal ended.",
            none(),
            &[],
            true,
        ),
        tool(
            "propose_fan_curves",
            "Proposes a whole new fan config. Nothing changes until the person at this PC approves it on screen in FoxDen; the call waits up to 50 seconds for their answer, and the request stays on screen for 3 minutes. Curves and rules left out are removed; fans without a rule go back to their firmware. Refused without asking: a maximum duty below 20%, a fan whose curve and minimum never reach 20%, and a pump capped below 50%, which could stop it. If the reply says it is still waiting, get_fan_config shows the outcome under last_proposal later.",
            proposal_schema(),
            &["curves", "rules", "reason"],
            false,
        ),
    ]
}

fn layer_schema(new: bool) -> Value {
    let mut props = json!({
        "kind": {"type": "string", "description": "A layer effect kind from list_effects' layer_effects, or any main effect kind to fill the layer"},
        "params": {"type": "object", "additionalProperties": true,
            "description": "Parameter values by name; keys as names from list_keys, key colours as {\"Q\": \"#ff0000\"}"},
        "enabled": {"type": "boolean"},
        "opacity": {"type": "number", "minimum": 0, "maximum": 100, "description": "Percent"},
        "blend": {"type": "string", "enum": ["normal", "add", "screen", "multiply", "tint"]},
        "scope": {"type": "string", "enum": ["all", "keyboards", "chosen"], "description": "Which devices; chosen takes devices"},
        "devices": {"type": "array", "items": {"type": "string"}, "description": "Device ids or names; sets scope to chosen"},
        "when": {"type": "string", "enum": ["always", "running", "front", "battery_low"],
            "description": "running or front: while one of apps runs or is in front"},
        "apps": {"type": "array", "items": {"type": "string"}, "description": "Executable names, such as \"League of Legends.exe\""},
        "battery": {"type": "string", "description": "For battery_low: a battery key from get_batteries, empty for any"},
        "low": {"type": "number", "minimum": 0, "maximum": 100, "description": "For battery_low: percent counted as low"},
    });
    if new {
        props["position"] = json!({"type": "integer", "minimum": 0, "description": "Index to insert at; on top if left out"});
    } else {
        props["index"] = json!({"type": "integer", "minimum": 0, "description": "Layer index from list_layers"});
    }
    props
}

fn proposal_schema() -> Value {
    json!({
        "curves": {"type": "array", "items": {
            "type": "object",
            "properties": {
                "name": {"type": "string", "maxLength": 40},
                "sensors": {"type": "array", "items": {"type": "string"}, "minItems": 1,
                    "description": "Temperature sensor ids from get_fan_config; the hottest drives the curve"},
                "points": {"type": "array", "minItems": 1, "items": {
                    "type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2,
                    "description": "[temperature °C from 20 to 100, duty percent from 0 to 100]"},
                    "description": "Rising temperatures, at least 1 °C apart"},
                "hysteresis": {"type": "number", "minimum": 0, "maximum": 30, "description": "°C the temperature must fall before duty drops; 2 if left out"},
            },
            "required": ["name", "sensors", "points"],
        }},
        "rules": {"type": "array", "items": {
            "type": "object",
            "properties": {
                "fan": {"type": "string", "description": "Fan id from get_fan_config"},
                "curve": {"type": "string", "description": "Name of a curve in this proposal"},
                "min_duty": {"type": "number", "minimum": 0, "maximum": 100, "description": "Percent; 0 if left out"},
                "max_duty": {"type": "number", "minimum": 20, "maximum": 100, "description": "Percent; 100 if left out"},
                "ramp": {"type": "number", "minimum": 1, "maximum": 100, "description": "Largest duty change in percent per second; 20 if left out"},
            },
            "required": ["fan", "curve"],
        }},
        "reason": {"type": "string", "maxLength": 300, "description": "One or two sentences for the person approving: why this change"},
    })
}

pub fn known(name: &str) -> bool {
    list().iter().any(|t| t["name"] == name)
}
