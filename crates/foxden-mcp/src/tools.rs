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
            "FoxDen's version, lighting devices, current effect, brightness, active profile and whether lighting is paused.",
            none(),
            &[],
            true,
        ),
        tool(
            "list_effects",
            "Every lighting effect kind with its parameters: name, type and range or options.",
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
