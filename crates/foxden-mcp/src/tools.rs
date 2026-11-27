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
            "key_map_code",
            "A saved key colour map as a share code.",
            json!({"name": {"type": "string"}}),
            &["name"],
            true,
        ),
        tool(
            "import_key_map_code",
            "Saves a key colour map from a share code.",
            json!({"code": {"type": "string"}}),
            &["code"],
            false,
        ),
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
            "Changes app settings: fps, language, close_to_tray, start_minimized, battery_tray, battery_alerts and auto_detect at once. Changing autostart, lighting_module, native_drivers, fans_module, network_enabled, openrgb_enabled, auto_update, close_conflicts or send_reports asks the person at this PC on screen first and needs a reason; nothing in the call changes unless they approve. The call waits up to 50 seconds; get_settings shows the outcome later under last_request.",
            json!({
                "autostart": {"type": "boolean", "description": "Start FoxDen with Windows; needs approval"},
                "lighting_module": {"type": "boolean", "description": "FoxDen drives lighting at all; needs approval"},
                "native_drivers": {"type": "boolean", "description": "FoxDen's own device drivers; needs approval"},
                "fans_module": {"type": "boolean", "description": "FoxDen drives fans; off hands them to their firmware; needs approval"},
                "network_enabled": {"type": "boolean", "description": "Lights on the local network; needs approval"},
                "openrgb_enabled": {"type": "boolean", "description": "Use an OpenRGB server; needs approval"},
                "auto_update": {"type": "boolean", "description": "Download and install updates; needs approval"},
                "close_conflicts": {"type": "boolean", "description": "Close other lighting and fan software; needs approval"},
                "send_reports": {"type": "boolean", "description": "Send anonymous error reports; needs approval"},
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
            "list_macros",
            "Key and mouse macros: id, name, hotkey, whether the hotkey is hidden from other apps, mode and steps, in the shape save_macro takes. Also whether macro hotkeys are on, which macros are playing, the key names steps and hotkeys use, and how the last request for approval ended.",
            none(),
            &[],
            true,
        ),
        tool(
            "save_macro",
            "Creates a macro, or replaces the one with this id. A macro sends keys, mouse buttons, wheel turns, typed text and media keys when its hotkey is pressed, in any app. The person at this PC approves it on screen first, so give a reason; the call waits up to 50 seconds.",
            json!({
                "id": {"type": "string", "description": "An id from list_macros to replace that macro; leave out to create one"},
                "name": {"type": "string", "maxLength": 40},
                "mode": {"type": "string", "enum": ["once", "press_hold_release", "repeat_while_held", "toggle"],
                    "description": "once plays press; press_hold_release plays press, loops hold while held, then release; repeat_while_held loops press while held; toggle loops press until pressed again. Pause stops every macro."},
                "press": macro_steps(),
                "hold": macro_steps(),
                "release": macro_steps(),
                "default_delay_ms": {"type": "integer", "minimum": 0, "maximum": 60000, "description": "Wait between steps that don't wait themselves"},
                "hotkey": {"type": ["string", "null"], "description": "Such as \"F13\", \"Ctrl+Shift+F5\" or \"Back button\"; null for none. Left out: an existing macro keeps its hotkey."},
                "block": {"type": "boolean", "description": "Hide the hotkey from the app in front"},
                "reason": {"type": "string", "maxLength": 300, "description": "One or two sentences for the person approving"},
            }),
            &["name", "press", "reason"],
            false,
        ),
        tool("delete_macro", "Deletes a macro by id or name.", json!({"id": {"type": "string"}}), &["id"], false),
        tool(
            "play_macro",
            "Plays a macro once after a delay, whatever its hotkey and mode. The person at this PC approves it on screen first.",
            json!({
                "id": {"type": "string", "description": "Id or name from list_macros"},
                "delay_s": {"type": "number", "minimum": 0, "maximum": 30, "description": "Seconds before it starts; 3 if left out"},
                "reason": {"type": "string", "maxLength": 300},
            }),
            &["id", "reason"],
            false,
        ),
        tool("stop_macros", "Stops every macro that is playing.", none(), &[], false),
        tool(
            "list_macro_presets",
            "Ready-made macros most people want (copy, paste, screenshot, media keys, rapid click and more): id, group, name and what it does.",
            none(),
            &[],
            true,
        ),
        tool(
            "add_macro_preset",
            "Adds a ready-made macro from list_macro_presets, optionally with a hotkey. The person at this PC approves it on screen first.",
            json!({
                "preset": {"type": "string", "description": "An id from list_macro_presets"},
                "hotkey": {"type": ["string", "null"], "description": "Such as \"F13\" or \"Back button\"; leave out for none"},
                "reason": {"type": "string", "maxLength": 300},
            }),
            &["preset", "reason"],
            false,
        ),
        tool(
            "list_mouse_macros",
            "Mice that keep macros in their own memory: each button, what it does from the factory, and which macro it holds. Reading takes a few seconds.",
            none(),
            &[],
            true,
        ),
        tool(
            "store_macro_on_mouse",
            "Writes a saved macro into the mouse's memory and puts it on one button, in every profile that is on. The mouse keeps keys, typed plain English text and waits, played once per press. The person at this PC approves it on screen first.",
            json!({
                "id": {"type": "string", "description": "Id or name from list_macros"},
                "button": {"type": "integer", "minimum": 1, "maximum": 16, "description": "Button number from list_mouse_macros"},
                "reason": {"type": "string", "maxLength": 300},
            }),
            &["id", "button", "reason"],
            false,
        ),
        tool(
            "reset_mouse_button",
            "Puts one mouse button back to what it does from the factory, in every profile that is on. The person at this PC approves it on screen first.",
            json!({
                "button": {"type": "integer", "minimum": 1, "maximum": 16},
                "reason": {"type": "string", "maxLength": 300},
            }),
            &["button", "reason"],
            false,
        ),
        tool(
            "set_macros_enabled",
            "Turns macro hotkeys on or off. Turning them on needs the approval of the person at this PC and a reason.",
            json!({"on": {"type": "boolean"}, "reason": {"type": "string", "maxLength": 300}}),
            &["on"],
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
            "The audio mixer in full: output, master, chat mix, monitor, virtual mic, each channel's volume, mute, source, apps, EQ and processing (the mic's with its gate and block order), sound profiles, EQ filters and presets, the inputs and outputs to choose from, what noise removal is running with its delay and CPU use, and how the last recording request ended.",
            json!({"rescan": {"type": "boolean", "description": "List the audio devices again first"}}),
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
            "set_audio",
            "Changes mixer-wide audio settings; fields left out stay as they are. Volumes are percent, chat_mix runs from -100 (all game side) to 100 (all chat).",
            json!({
                "output": {"type": "string", "description": "Output from get_audio's outputs that the mix plays on"},
                "park": {"type": "string", "description": "Output assigned apps are parked on, muted; empty lets FoxDen pick"},
                "master": {"type": "number", "minimum": 0, "maximum": 100},
                "master_muted": {"type": "boolean"},
                "chat_mix": {"type": "number", "minimum": -100, "maximum": 100},
                "game_side": {"type": "array", "items": {"type": "string", "enum": ["game", "media", "aux"]}, "description": "Channels on the game side of the chat mix"},
                "running": {"type": "boolean", "description": "Start or stop the mixer"},
                "auto_assign": {"type": "boolean", "description": "Move apps that start playing to their usual channel"},
                "allow_auto_assign": {"type": "string", "description": "An executable auto-assign skips, to let it place that app again"},
                "monitor": {"type": "boolean", "description": "Hear the mic on the output"},
                "monitor_volume": {"type": "number", "minimum": 0, "maximum": 100},
                "virtual_mic": {"type": "boolean", "description": "Play the processed mic into VB-CABLE for other apps"},
            }),
            &[],
            false,
        ),
        tool(
            "route_channel",
            "Sets where a channel takes sound from: an input such as a microphone, or everything playing on an output. An empty source leaves only its apps. Also turns the channel on or off.",
            json!({
                "channel": channel(&CHANNELS),
                "source": {"type": "string", "description": "A name from get_audio's inputs or outputs; empty for none"},
                "source_is_output": {"type": "boolean", "description": "Record what plays on that output; worked out from the name if left out"},
                "on": {"type": "boolean"},
            }),
            &["channel"],
            false,
        ),
        tool(
            "unassign_app",
            "Takes an app off its mixer channel, so it plays as Windows sends it.",
            json!({"app": {"type": "string", "description": "Executable name"}}),
            &["app"],
            false,
        ),
        tool(
            "set_eq",
            "Changes a channel's EQ of up to 10 bands. A high pass is a band with filter HighPass. preset replaces every band with one of the presets; bands replaces them all; band_changes edits single bands by index.",
            json!({
                "channel": channel(&CHANNELS),
                "enabled": {"type": "boolean"},
                "preset": {"type": "string", "enum": ["flat", "bass_boost", "voice_clarity", "treble"]},
                "bands": {"type": "array", "maxItems": 10, "items": eq_band(false)},
                "band_changes": {"type": "array", "items": eq_band(true)},
            }),
            &["channel"],
            false,
        ),
        tool(
            "set_processing",
            "Changes a channel's processing, or the master's with channel \"master\". Blocks left out stay as they are, and fields of a block left out keep their values; get_audio shows them all. The mic runs noise removal (denoise), gate, then agc (level stabilizer), comp, deesser and eq in `order`, then the limiter. Outputs and the master take comp and limiter only.",
            json!({
                "channel": {"type": "string", "enum": ["game", "chat", "media", "aux", "mic", "master"]},
                "denoise": {"type": "object", "description": "on, strength 0 to 1, engine light (RNNoise, about 20 ms in all), strong (DeepFilterNet 3, about 40 ms), strong_ll (DeepFilterNet 3 low latency, about 20 ms) or auto (strong while it keeps up); strong and strong_ll fall back to light when they cannot keep up"},
                "gate": {"type": "object", "description": "on, threshold_db -90 to 0, attack_ms 0.1 to 50, hold_ms 0 to 1000, release_ms 5 to 1000"},
                "agc": {"type": "object", "description": "on, target_db -40 to -6, max_gain_db, max_cut_db 0 to 30, speed_db dB/s, floor_db (hold below)"},
                "comp": {"type": "object", "description": "on, threshold_db -60 to 0, ratio 1 to 20, attack_ms, release_ms, knee_db, makeup_db, auto_makeup"},
                "deesser": {"type": "object", "description": "on, frequency 3000 to 10000 Hz, threshold_db -30 to 0 (band over the whole), amount_db 0 to 24"},
                "limiter": {"type": "object", "description": "on, ceiling_db -24 to 0, release_ms 5 to 2000, lookahead_ms 0 to 5"},
                "order": {"type": "array", "items": {"type": "string", "enum": ["agc", "comp", "deesser", "eq"]}, "minItems": 4, "maxItems": 4},
            }),
            &["channel"],
            false,
        ),
        tool(
            "apply_sound_profile",
            "Applies a saved or built-in sound profile from get_audio's sound_profiles to a channel; mic profiles go on the mic, output profiles on the others.",
            json!({"channel": channel(&CHANNELS), "name": {"type": "string"}}),
            &["channel", "name"],
            false,
        ),
        tool(
            "save_sound_profile",
            "Saves a channel's EQ, volume and processing as a sound profile, replacing one of the same name.",
            json!({"channel": channel(&CHANNELS), "name": {"type": "string", "maxLength": 40}}),
            &["channel", "name"],
            false,
        ),
        tool(
            "rename_sound_profile",
            "Renames a saved sound profile.",
            json!({"name": {"type": "string"}, "to": {"type": "string", "maxLength": 40}}),
            &["name", "to"],
            false,
        ),
        tool("delete_sound_profile", "Deletes a saved sound profile.", json!({"name": {"type": "string"}}), &["name"], false),
        tool(
            "sound_profile_code",
            "A sound profile as a share code another FoxDen can import.",
            json!({"name": {"type": "string"}}),
            &["name"],
            true,
        ),
        tool(
            "import_sound_profile",
            "Adds a sound profile from a share code.",
            json!({"code": {"type": "string", "maxLength": 2000}}),
            &["code"],
            false,
        ),
        tool(
            "get_audio_priority",
            "The output and input priority lists: devices in the order FoxDen picks them, their state, and devices that could be added.",
            none(),
            &[],
            true,
        ),
        tool(
            "set_audio_priority",
            "Changes the device priority. Lists replace the whole list, in order, by device id or name.",
            json!({
                "on": {"type": "boolean"},
                "mixer": {"type": "boolean", "description": "Move the mixer's output to the first device available"},
                "windows": {"type": "boolean", "description": "Make it Windows' default device too"},
                "outputs": {"type": "array", "items": {"type": "string"}},
                "inputs": {"type": "array", "items": {"type": "string"}},
            }),
            &[],
            false,
        ),
        tool(
            "get_app_volumes",
            "Windows' own volume and mute of each app playing sound and of each audio device.",
            none(),
            &[],
            true,
        ),
        tool(
            "set_app_volume",
            "Sets an app's Windows volume or mute.",
            json!({
                "app": {"type": "string", "description": "Executable from get_app_volumes"},
                "volume": {"type": "number", "minimum": 0, "maximum": 100},
                "muted": {"type": "boolean"},
            }),
            &["app"],
            false,
        ),
        tool(
            "set_device_volume",
            "Sets an audio device's Windows volume or mute.",
            json!({
                "device": {"type": "string", "description": "Device id or name from get_app_volumes"},
                "volume": {"type": "number", "minimum": 0, "maximum": 100},
                "muted": {"type": "boolean"},
            }),
            &["device"],
            false,
        ),
        tool(
            "set_default_device",
            "Makes an audio device Windows' default for playback or recording.",
            json!({"device": {"type": "string", "description": "Device id or name from get_app_volumes"}}),
            &["device"],
            false,
        ),
        tool(
            "analyze_mic",
            "Records a few seconds of the Mic channel's microphone and measures it raw and through the current mic chain: peak, RMS, LUFS, noise floor, clipping, third-octave spectrum, sibilance at 4 to 10 kHz, rumble, plosives, clicks between words, and the difference the chain makes. The person at this PC approves every recording on screen; nothing is recorded otherwise. The sound stays in memory and is deleted at once. Ask the person to speak normally while it records. The call waits up to 50 seconds for their answer.",
            json!({
                "seconds": {"type": "number", "minimum": 1, "maximum": 10, "description": "5 if left out"},
                "reason": {"type": "string", "maxLength": 300},
            }),
            &["reason"],
            false,
        ),
        tool(
            "auto_tune_mic",
            "Records one approved sample, as analyze_mic does, and works out and applies the whole mic chain for one use from it: noise removal and its engine, gate, high pass, EQ, de-esser, stabilizer, compressor and limiter. Returns the settings, the reasons, and the analysis before (current chain) and after (new chain, checked on the same sample). undo_auto_tune puts the old chain back. discord: clear steady speech; streaming: about -16 LUFS, -1 dBFS ceiling; podcast: warm and natural, -16 LUFS stereo or -19 mono; gaming: against clicks and shouting, little delay; meeting: Teams, Zoom or Meet; music: singing or an instrument, kept natural.",
            json!({
                "target": {"type": "string", "enum": ["discord", "streaming", "podcast", "gaming", "meeting", "music"]},
                "delivery": {"type": "string", "enum": ["stereo", "mono"], "description": "For podcast: the loudness aimed at"},
                "seconds": {"type": "number", "minimum": 1, "maximum": 10, "description": "6 if left out"},
                "reason": {"type": "string", "maxLength": 300},
            }),
            &["target", "reason"],
            false,
        ),
        tool("undo_auto_tune", "Puts back the mic chain the last auto-tune replaced.", none(), &[], false),
        tool(
            "set_fan_curve",
            "Adds a fan curve or changes one, keeping the rest of the fan config. Fields left out of an existing curve keep their values. Approved on screen like propose_fan_curves, with the same limits.",
            json!({
                "name": {"type": "string", "maxLength": 40},
                "sensors": {"type": "array", "items": {"type": "string"}, "minItems": 1},
                "points": {"type": "array", "items": {"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2}, "description": "[°C, duty %], rising"},
                "hysteresis": {"type": "number", "minimum": 0, "maximum": 30},
                "reason": {"type": "string", "maxLength": 300},
            }),
            &["name", "reason"],
            false,
        ),
        tool(
            "remove_fan_curve",
            "Removes a fan curve no fan follows. Approved on screen.",
            json!({"name": {"type": "string"}, "reason": {"type": "string", "maxLength": 300}}),
            &["name", "reason"],
            false,
        ),
        tool(
            "set_fan_rule",
            "Has one fan follow a curve with its limits, or with curve null go back to its firmware. Fields left out keep their values. Approved on screen, with the 20% floor and pump limits of propose_fan_curves.",
            json!({
                "fan": {"type": "string", "description": "Fan id from get_fan_config"},
                "curve": {"type": ["string", "null"]},
                "min_duty": {"type": "number", "minimum": 0, "maximum": 100},
                "max_duty": {"type": "number", "minimum": 20, "maximum": 100},
                "ramp": {"type": "number", "minimum": 1, "maximum": 100},
                "reason": {"type": "string", "maxLength": 300},
            }),
            &["fan", "reason"],
            false,
        ),
        tool(
            "set_fan_service",
            "Starts or stops FoxDen's fan service; stopped, every fan runs on its firmware.",
            json!({"running": {"type": "boolean"}}),
            &["running"],
            false,
        ),
        tool(
            "set_hardware_mode",
            "Has a device run one of its own effects (get_status shows hardware_modes), with speed, brightness, direction, random and colours in its ranges, or with mode null follow FoxDen again.",
            json!({
                "device": {"type": "string"},
                "mode": {"type": ["string", "null"]},
                "speed": {"type": "integer"},
                "brightness": {"type": "integer"},
                "direction": {"type": "string"},
                "random": {"type": "boolean"},
                "colors": {"type": "array", "items": {"type": "string"}, "description": "\"#rrggbb\" each"},
            }),
            &["device"],
            false,
        ),
        tool("reset_layout", "Puts every device back in its first place on the effect canvas.", none(), &[], false),
        tool("detect_devices", "Looks for hardware and conflicting software again.", none(), &[], false),
        tool(
            "set_zone_leds",
            "Sets how many LEDs a resizable zone has, such as a strip on an ARGB header.",
            json!({"zone": {"type": "string", "description": "Zone key from get_status"}, "leds": {"type": "integer", "minimum": 0}}),
            &["zone", "leds"],
            false,
        ),
        tool(
            "get_headsets",
            "Headsets FoxDen drives: battery, ChatMix dial, each control (sidetone, mic volume, noise gate, ANC and more) with its values, and the EQ.",
            none(),
            &[],
            true,
        ),
        tool(
            "set_headset",
            "Sets one headset control to a value its shape in get_headsets allows.",
            json!({"headset": {"type": "string", "description": "Key or name"}, "control": {"type": "string"}, "value": {"type": "integer"}}),
            &["headset", "control", "value"],
            false,
        ),
        tool(
            "set_headset_eq",
            "Sets a headset's own EQ bands.",
            json!({"headset": {"type": "string"}, "bands": {"type": "array", "items": {"type": "integer"}}}),
            &["headset", "bands"],
            false,
        ),
        tool(
            "get_device_info",
            "A device's technical details as text, as the Devices tab shows for a device report. Read only; nothing is sent.",
            json!({"device": {"type": "string"}}),
            &["device"],
            true,
        ),
        tool(
            "get_logs",
            "FoxDen's log, newest last.",
            json!({
                "level": {"type": "string", "enum": ["info", "warn", "error"], "description": "This level and worse"},
                "source": {"type": "string"},
                "limit": {"type": "integer", "minimum": 1, "maximum": 1000, "description": "100 if left out"},
            }),
            &[],
            true,
        ),
        tool(
            "get_conflicts",
            "Other lighting and fan software FoxDen knows, whether it runs and whether FoxDen would close it.",
            none(),
            &[],
            true,
        ),
        tool(
            "close_conflicts",
            "Closes the conflicting software get_conflicts marks to close. The person at this PC approves it on screen first.",
            json!({"reason": {"type": "string", "maxLength": 300}}),
            &["reason"],
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

fn macro_steps() -> Value {
    json!({"type": "array", "items": {"type": "object", "properties": {
        "type": {"type": "string", "enum": ["key", "button", "wheel", "delay", "text", "media"]},
        "key": {"type": "string", "description": "key: a key name from list_macros' key_names; media: play_pause, stop, next, previous, volume_up, volume_down or mute"},
        "action": {"type": "string", "enum": ["tap", "down", "up"], "description": "For key and button; tap if left out"},
        "button": {"type": "string", "enum": ["left", "right", "middle", "back", "forward"]},
        "ticks": {"type": "integer", "description": "Wheel: positive up or right, negative down or left"},
        "horizontal": {"type": "boolean"},
        "ms": {"type": "integer", "minimum": 0, "maximum": 60000},
        "text": {"type": "string", "maxLength": 1000, "description": "Typed as it is"},
    }, "required": ["type"]}})
}

fn eq_band(edit: bool) -> Value {
    let mut props = json!({
        "enabled": {"type": "boolean"},
        "filter": {"type": "string", "description": "A filter from get_audio's eq_filters, such as PeakingEQ or HighPass"},
        "frequency": {"type": "number", "minimum": 20, "maximum": 20000},
        "gain_db": {"type": "number", "minimum": -24, "maximum": 24},
        "q": {"type": "number", "minimum": 0.1, "maximum": 10},
    });
    if edit {
        props["index"] = json!({"type": "integer", "minimum": 0});
    }
    json!({"type": "object", "properties": props, "required": if edit { vec!["index"] } else { vec![] }})
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
