use std::collections::HashSet;

use serde_json::{Value, json};
use slint::{ComponentHandle, Model, SharedString};
use tauri::AppHandle;

use super::super::{
    AppWindow, AudBand, AudBlock, AudChip, AudGr, AudOpt, AudPSlider, AudPrio, AudSkip, AudSrc, AudStrip, AudVol,
    AudioPage, fill, tr,
};
use super::{CHANNELS, FREQS, PRESETS, SIDES, St, apps_of, chan_of, f, kind_of, label_of, short, tf, vol_key};

fn opt(value: &str, text: &str, off: bool) -> AudOpt {
    AudOpt { value: value.into(), text: text.into(), off }
}

/// The options with the current value picked, adding it as missing when it is not offered.
fn choose(mut list: Vec<AudOpt>, current: &str, missing: String) -> (Vec<AudOpt>, i32) {
    if !current.is_empty() && !list.iter().any(|o| o.value == current) {
        list.insert(0, opt(current, &missing, false));
    }
    let i = list.iter().position(|o| o.value == current).unwrap_or(0);
    (list, i as i32)
}

fn strs(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|l| l.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

pub(super) fn signed(v: f64) -> String {
    let sign = if v > 0.0 { "+" } else if v < 0.0 { "−" } else { "" };
    format!("{sign}{:.1}", v.abs())
}

fn hz(v: f64) -> String {
    if v >= 1000.0 {
        let k = (v / 100.0).round() / 10.0;
        format!("{k}k")
    } else {
        format!("{}", v.round())
    }
}

fn pct(v: f64) -> String {
    format!("{}%", (v * 100.0).round())
}

pub(super) fn render(w: &AppWindow, app: &AppHandle, s: &St) {
    let g = w.global::<AudioPage>();
    g.set_loaded(s.mix.is_object());
    g.set_problem(s.problem.as_str().into());
    if s.mix.is_object() {
        top(&g, app, s);
        strips(&g, app, s);
        chat(&g, app, s);
        mic(&g, app, s);
        eq(&g, app, s);
        processing(&g, app, s);
        advanced(&g, app, s);
    }
    priority(&g, app, s);
    volumes(&g, app, s);
}

fn top(g: &AudioPage, app: &AppHandle, s: &St) {
    let mix = &s.mix;
    let cfg = s.cfg();
    g.set_running(s.running());
    let following = mix["following"].as_bool().unwrap_or(false);
    let output = cfg["output"].as_str().unwrap_or_default();
    let shown = if s.held("output") {
        output
    } else {
        mix["playing_on"].as_str().filter(|_| following).unwrap_or(output)
    };
    let list = strs(&mix["outputs"]).iter().map(|o| opt(o, o, false)).collect();
    let (list, i) = choose(list, shown, tf(app, "{0} (missing)", &[shown]));
    fill(g.get_outputs(), list, |m| g.set_outputs(m));
    g.set_output_index(i);
    g.set_following(following);
    g.set_following_note(tr(app, "Picked by the output priority").into());
    g.set_master(f(&cfg["master"]));
    g.set_master_muted(cfg["master_muted"].as_bool().unwrap_or(false));
    g.set_master_fx_open(s.proc.as_deref() == Some("master"));
    let m = tr(app, "Master");
    let labels: Vec<SharedString> = [tf(app, "{0} volume", &[&m]), tf(app, "Mute {0}", &[&m]), tf(app, "Edit {0} processing", &[&m])]
        .into_iter()
        .map(SharedString::from)
        .collect();
    fill(g.get_master_labels(), labels, |l| g.set_master_labels(l));
    g.set_plays_on(
        if output.is_empty() {
            tr(app, "Pick an output above.")
        } else {
            tf(app, "Plays on {0}", &[output])
        }
        .into(),
    );
}

/// Where an assigned app stands while the mixer runs: mark, word and note.
fn app_state(app: &AppHandle, s: &St, exe: &str) -> (&'static str, String, String) {
    if !s.running() {
        return ("none", String::new(), String::new());
    }
    let st = &s.mix["app_states"][exe.to_lowercase()];
    let out = st["output"].as_str().unwrap_or_default();
    match st["state"].as_str().unwrap_or("idle") {
        "idle" => ("none", tr(app, "Not playing"), String::new()),
        "playing" => ("on", tr(app, "Mixed"), String::new()),
        "elsewhere" => ("warn", tr(app, "Restart it"), tf(app, "Still on {0}. Restart it to move it here.", &[out])),
        "own_output" => ("warn", tr(app, "Own output"), own_output(app, exe, out)),
        "contains" => {
            let other = short(st["app"].as_str().unwrap_or_default());
            ("warn", tr(app, "Not mixed"), tf(app, "It started {0}, which is in another channel.", &[&other]))
        }
        _ => ("warn", tr(app, "Failed"), st["message"].as_str().unwrap_or_default().to_string()),
    }
}

// Apps that pick a device in their own settings ignore Windows' per-app routing.
fn own_output(app: &AppHandle, exe: &str, output: &str) -> String {
    let name = short(exe);
    if exe.eq_ignore_ascii_case("discord.exe") {
        let text = format!(
            "{name} picks its own output ({output}). Set it to Default in Discord: User Settings → Voice & Video → Output Device → Default."
        );
        return tr(app, &text);
    }
    tf(app, "{0} picks its own output ({1}). Set it to Default in {2}'s settings", &[&name, output, &name]) + "."
}

fn strips(g: &AudioPage, app: &AppHandle, s: &St) {
    let cfg = s.cfg();
    let assigned: HashSet<String> = cfg["channels"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(apps_of)
        .map(|a| a.to_lowercase())
        .collect();
    let auto: HashSet<String> = strs(&cfg["auto"]).into_iter().collect();
    let free: Vec<&Value> = s.mix["apps"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|a| !assigned.contains(&a["exe"].as_str().unwrap_or_default().to_lowercase()))
        .collect();
    let mut add = vec![opt(
        "",
        &tr(app, if free.is_empty() { "No other apps playing sound" } else { "+ Add app" }),
        false,
    )];
    for a in &free {
        let exe = a["exe"].as_str().unwrap_or_default();
        let name = if a["active"].as_bool().unwrap_or(false) {
            tf(app, "{0} (playing)", &[&short(exe)])
        } else {
            short(exe)
        };
        add.push(opt(exe, &name, false));
    }
    fill(g.get_free_apps(), add, |m| g.set_free_apps(m));

    let mut rows = Vec::new();
    let mut chips = Vec::new();
    let null = Value::Null;
    for (n, (id, label)) in CHANNELS.iter().enumerate() {
        let c = chan_of(cfg, id).unwrap_or(&null);
        let apps = apps_of(c);
        let source = c["source"].as_str().unwrap_or_default();
        let on = c["on"].as_bool().unwrap_or(false);
        let l = tr(app, label);
        let before = chips.len();
        if n < 4 {
            for exe in &apps {
                let (mark, word, note) = app_state(app, s, exe);
                let warn = mark == "warn";
                let name = short(exe);
                chips.push(AudChip {
                    strip: n as i32,
                    exe: exe.as_str().into(),
                    name: name.as_str().into(),
                    mark: mark.into(),
                    tag: if warn { word.into() } else { SharedString::new() },
                    warn,
                    auto: !warn && auto.contains(&exe.to_lowercase()),
                    device: false,
                    note: note.into(),
                    remove_label: tf(app, "Remove {0} from {1}", &[&name, &l]).into(),
                });
            }
            if !source.is_empty() {
                let what = if c["loopback"].as_bool().unwrap_or(false) {
                    tf(app, "Everything on {0}", &[source])
                } else {
                    source.to_string()
                };
                chips.push(AudChip {
                    strip: n as i32,
                    exe: SharedString::new(),
                    name: what.into(),
                    mark: "".into(),
                    tag: SharedString::new(),
                    warn: false,
                    auto: false,
                    device: true,
                    note: SharedString::new(),
                    remove_label: tf(app, "Stop recording {0} into {1}", &[source, &l]).into(),
                });
            }
        }
        rows.push(AudStrip {
            id: (*id).into(),
            title: l.as_str().into(),
            off: !on && (!apps.is_empty() || !source.is_empty()),
            turn_on_label: tf(app, "Turn on {0}", &[&l]).into(),
            volume: f(&c["volume"]),
            vol_label: tf(app, "{0} volume", &[&l]).into(),
            muted: c["muted"].as_bool().unwrap_or(false),
            mute_label: tf(app, "Mute {0}", &[&l]).into(),
            eq_on: c["eq_on"].as_bool().unwrap_or(false),
            eq_on_label: tf(app, "{0} EQ on", &[&l]).into(),
            eq_open: s.eq.as_deref() == Some(id),
            eq_label: tf(app, "Edit {0} EQ", &[&l]).into(),
            fx_open: s.proc.as_deref() == Some(id),
            fx_label: tf(app, "Edit {0} processing", &[&l]).into(),
            empty: chips.len() == before,
            add_label: tf(app, "Add an app to {0}", &[&l]).into(),
            chips_label: tf(app, "{0} apps", &[&l]).into(),
        });
    }
    fill(g.get_strips(), rows, |m| g.set_strips(m));
    fill(g.get_chips(), chips, |m| g.set_chips(m));
}

fn chat(g: &AudioPage, app: &AppHandle, s: &St) {
    let cfg = s.cfg();
    let dial = s.dial.is_object();
    let v = if dial {
        s.dial["chat"].as_f64().unwrap_or(0.0) - s.dial["game"].as_f64().unwrap_or(0.0)
    } else {
        (cfg["chat_mix"].as_f64().unwrap_or(0.0) * 100.0).round()
    };
    g.set_dial(dial);
    g.set_chat(v as f32);
    let n = v.abs().round();
    g.set_chat_word(
        if v == 0.0 {
            tr(app, "Even")
        } else if v < 0.0 {
            format!("{}{n}", tr(app, "Game +"))
        } else {
            tf(app, "Chat +{0}", &[&n.to_string()])
        }
        .into(),
    );
    let sides = if cfg["game_side"].is_array() { strs(&cfg["game_side"]) } else { vec!["game".into()] };
    let names: Vec<String> = SIDES.iter().filter(|x| sides.iter().any(|y| y == x.0)).map(|x| tr(app, x.1)).collect();
    g.set_side_end(if names.is_empty() { tr(app, "Nothing") } else { names.join(" + ") }.into());
    let on: Vec<bool> = SIDES.iter().map(|x| sides.iter().any(|y| y == x.0)).collect();
    fill(g.get_sides(), on, |m| g.set_sides(m));
}

fn mic(g: &AudioPage, app: &AppHandle, s: &St) {
    let cfg = s.cfg();
    let null = Value::Null;
    let c = chan_of(cfg, "chatCapture").unwrap_or(&null);
    let source = c["source"].as_str().unwrap_or_default();
    let loopback = c["loopback"].as_bool().unwrap_or(false);
    {
        let mut list = vec![opt("", &tr(app, "None"), false)];
        list.extend(strs(&s.mix["inputs"]).iter().map(|i| opt(&format!("in:{i}"), i, false)));
        if !source.is_empty() && loopback {
            list.push(opt(&format!("out:{source}"), &tf(app, "Everything on {0}", &[source]), false));
        }
        let current = if source.is_empty() {
            String::new()
        } else {
            format!("{}{source}", if loopback { "out:" } else { "in:" })
        };
        let (list, i) = choose(list, &current, tf(app, "{0} (missing)", &[source]));
        fill(g.get_mics(), list, |m| g.set_mics(m));
        g.set_mic_index(i);
    }
    g.set_mic_empty(source.is_empty());
    let mut profiles = vec![opt("", &tr(app, "Mic profile"), false)];
    for p in s.profiles("mic") {
        let name = p["name"].as_str().unwrap_or_default();
        profiles.push(opt(name, name, false));
    }
    fill(g.get_mic_profiles(), profiles, |m| g.set_mic_profiles(m));

    let v = &s.mix["vmic"];
    let installed = v["installed"].as_bool().unwrap_or(false);
    let ready = v["ready"].as_bool().unwrap_or(false);
    let on = cfg["virtual_mic"].as_bool().unwrap_or(false);
    if !g.get_sheet_open() {
        g.set_vmic(on);
    }
    let note = if let Some(e) = v["error"].as_str() {
        e.to_string()
    } else if installed && !ready {
        tr(app, "Restart Windows to finish installing VB-CABLE.")
    } else if !on {
        String::new()
    } else if !installed {
        tr(app, "VB-CABLE is not installed.")
    } else if source.is_empty() {
        tr(app, "Pick a microphone above.")
    } else {
        let name = v["name"].as_str().unwrap_or("FoxDen VB-Cable (Microphone)");
        tf(app, "Apps can pick {0}", &[&format!("{name}.")])
    };
    g.set_vmic_note(note.into());
    g.set_vmic_warn(v["error"].is_string());
    g.set_gate_on(cfg["gate_on"].as_bool().unwrap_or(false));
    g.set_gate_db(f(&cfg["gate_db"]));
    g.set_monitor(cfg["monitor"].as_bool().unwrap_or(false));
    g.set_monitor_volume(cfg["monitor_volume"].as_f64().unwrap_or(0.5) as f32);
    g.set_vmic_state(
        tr(
            app,
            if !installed {
                "Not installed"
            } else if !ready {
                "Restart Windows to finish"
            } else if v["ours"].as_bool().unwrap_or(false) {
                "Installed by FoxDen"
            } else {
                "Installed"
            },
        )
        .into(),
    );
    g.set_vmic_removable(v["ours"].as_bool().unwrap_or(false));
}

fn priority(g: &AudioPage, app: &AppHandle, s: &St) {
    let v = &s.prio;
    if !v.is_object() {
        return;
    }
    let c = &v["config"];
    let on = c["on"].as_bool().unwrap_or(false);
    g.set_prio_on(on);
    g.set_p_on(on);
    g.set_p_mixer(c["mixer"].as_bool().unwrap_or(false));
    g.set_p_windows(c["windows"].as_bool().unwrap_or(false));
    g.set_p_inputs(s.prio_inputs);
    let (rows, listed, choices) = if s.prio_inputs {
        (&v["inputs"], &c["inputs"], &v["choices_in"])
    } else {
        (&v["outputs"], &c["outputs"], &v["choices_out"])
    };
    let list: Vec<AudPrio> = rows
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(i, r)| {
            let name = r["name"].as_str().unwrap_or_default();
            let status = r["status"].as_str().unwrap_or_default();
            let chosen = r["chosen"].as_bool().unwrap_or(false);
            let tag = if chosen {
                "In use"
            } else {
                match status {
                    "available" => "Available",
                    "off" => "Off",
                    _ => "Unplugged",
                }
            };
            AudPrio {
                name: name.into(),
                num: i as i32 + 1,
                tag: tr(app, tag).into(),
                chosen,
                dim: status == "off" || status == "unplugged",
                up_label: tf(app, "Move {0} up", &[name]).into(),
                down_label: tf(app, "Move {0} down", &[name]).into(),
                remove_label: tf(app, "Remove {0}", &[name]).into(),
            }
        })
        .collect();
    fill(g.get_prio_rows(), list, |m| g.set_prio_rows(m));
    let ids: HashSet<&str> = listed.as_array().into_iter().flatten().filter_map(|e| e["id"].as_str()).collect();
    let mut add = vec![opt("", &tr(app, "Add a device"), false)];
    for e in choices.as_array().into_iter().flatten() {
        let id = e["id"].as_str().unwrap_or_default();
        if !ids.contains(id) {
            add.push(opt(id, e["name"].as_str().unwrap_or_default(), false));
        }
    }
    fill(g.get_prio_add(), add, |m| g.set_prio_add(m));
    g.set_prio_note(
        tr(
            app,
            if s.prio_inputs {
                "Inputs set Windows' default recording device. The mixer's mic stays as picked."
            } else {
                "FoxDen uses the first device that is really there. A wireless headset counts as off while its receiver is plugged in but the headset is switched off."
            },
        )
        .into(),
    );
}

/// The preset whose layout and gains the bands have now, if any.
fn preset_of(bands: &[Value]) -> i32 {
    if bands.len() != FREQS.len() {
        return -1;
    }
    let plain = bands.iter().zip(FREQS).all(|(b, fq)| {
        b["filter"] == "PeakingEQ"
            && b["frequency"].as_f64().unwrap_or(0.0).round() == fq
            && (b["q"].as_f64().unwrap_or(0.0) - 1.414).abs() < 0.01
    });
    if !plain {
        return -1;
    }
    PRESETS
        .iter()
        .position(|p| p.iter().zip(bands).all(|(g, b)| (b["gain_db"].as_f64().unwrap_or(0.0) - g).abs() < 0.01))
        .map_or(-1, |i| i as i32)
}

fn eq(g: &AudioPage, app: &AppHandle, s: &St) {
    let Some(id) = s.eq.as_deref() else {
        g.set_eq_open(false);
        return;
    };
    let Some(c) = s.chan(id) else {
        g.set_eq_open(false);
        return;
    };
    g.set_eq_open(true);
    let l = tr(app, label_of(id));
    g.set_eq_title(tf(app, "{0} EQ", &[&l]).into());
    g.set_eq_on_label(tf(app, "{0} EQ on", &[&l]).into());
    let on = c["eq_on"].as_bool().unwrap_or(false);
    g.set_eq_on(on);
    let bands = c["eq"].as_array().cloned().unwrap_or_default();
    g.set_preset(preset_of(&bands));

    let list = s.profiles(kind_of(id));
    let mut pf = vec![opt("", &tr(app, "Apply a profile"), false)];
    for p in &list {
        let name = p["name"].as_str().unwrap_or_default();
        let text = if p["builtin"].as_bool().unwrap_or(false) { name.to_string() } else { tf(app, "{0} (yours)", &[name]) };
        pf.push(opt(name, &text, false));
    }
    fill(g.get_pf_options(), pf, |m| g.set_pf_options(m));
    g.set_pf_mine(
        list.iter()
            .any(|p| p["name"].as_str() == s.pf_current.as_deref() && !p["builtin"].as_bool().unwrap_or(false)),
    );

    let filters: Vec<&Value> = s.mix["filters"].as_array().into_iter().flatten().collect();
    let options = filters
        .iter()
        .map(|x| opt(x["id"].as_str().unwrap_or_default(), &tr(app, x["label"].as_str().unwrap_or_default()), false))
        .collect();
    fill(g.get_filters(), options, |m| g.set_filters(m));
    let rows = bands
        .iter()
        .enumerate()
        .map(|(n, b)| {
            let fq = b["frequency"].as_f64().unwrap_or(1000.0);
            let filter = b["filter"].as_str().unwrap_or_default();
            let at = tf(app, "{0} Hz", &[&hz(fq)]);
            let q = (b["q"].as_f64().unwrap_or(1.0) * 100.0).round() / 100.0;
            let fdef = filters.iter().position(|x| x["id"] == filter);
            AudBand {
                name: hz(fq).into(),
                gain: f(&b["gain_db"]),
                db: signed(b["gain_db"].as_f64().unwrap_or(0.0)).into(),
                filter: fdef.map_or(-1, |i| i as i32),
                freq: format!("{}", fq.round()).into(),
                q: format!("{q}").into(),
                gain_off: fdef.is_some_and(|i| filters[i]["gain"] == false),
                gain_label: tf(app, "{0} gain", &[&at]).into(),
                filter_label: tf(app, "{0} filter", &[&at]).into(),
                freq_label: tf(app, "Band {0} frequency in Hz", &[&(n + 1).to_string()]).into(),
                q_label: tf(app, "{0} Q", &[&at]).into(),
            }
        })
        .collect();
    fill(g.get_bands(), rows, |m| g.set_bands(m));

    let i = CHANNELS.iter().position(|x| x.0 == id).unwrap_or(0);
    let curve = s.mix["curves"][i].as_array().cloned().unwrap_or_default();
    let mut path = String::new();
    let last = curve.len().saturating_sub(1).max(1) as f64;
    for (n, db) in curve.iter().enumerate() {
        let x = n as f64 / last * 100.0;
        let y = 30.0 - db.as_f64().unwrap_or(0.0).clamp(-15.0, 15.0) / 15.0 * 30.0;
        path.push_str(&format!("{}{x:.2} {y:.2} ", if n == 0 { "M" } else { "L" }));
    }
    g.set_curve(path.trim_end().into());
    g.set_curve_off(!on);
}

#[derive(Clone, Copy)]
enum Fmt {
    Pct,
    Db,
    Ratio,
    Ms1,
    Ms0,
    DbPerS,
    Hz,
    Dbfs,
}

struct BlockDef {
    id: &'static str,
    name: &'static str,
    mic: bool,
    hint: &'static str,
    meter: Option<&'static str>,
    sliders: &'static [(&'static str, &'static str, f32, f32, f32, Fmt, bool)],
    check: Option<(&'static str, &'static str)>,
}

const BLOCKS: [BlockDef; 6] = [
    BlockDef {
        id: "denoise",
        name: "Noise suppression",
        mic: true,
        hint: "Adds 20 ms of delay.",
        meter: None,
        sliders: &[("strength", "Strength", 0.0, 1.0, 0.01, Fmt::Pct, false)],
        check: None,
    },
    BlockDef {
        id: "gate",
        name: "Gate",
        mic: true,
        hint: "",
        meter: None,
        sliders: &[
            ("threshold_db", "Threshold", -90.0, 0.0, 1.0, Fmt::Dbfs, false),
            ("attack_ms", "Attack", 0.1, 50.0, 0.1, Fmt::Ms1, true),
            ("hold_ms", "Hold", 0.0, 1000.0, 5.0, Fmt::Ms0, true),
            ("release_ms", "Release", 5.0, 1000.0, 5.0, Fmt::Ms0, true),
        ],
        check: None,
    },
    BlockDef {
        id: "agc",
        name: "Level stabilizer",
        mic: true,
        hint: "",
        meter: Some("agc"),
        sliders: &[
            ("target_db", "Target", -40.0, -6.0, 0.5, Fmt::Db, false),
            ("max_gain_db", "Max boost", 0.0, 30.0, 0.5, Fmt::Db, false),
            ("max_cut_db", "Max cut", 0.0, 30.0, 0.5, Fmt::Db, true),
            ("speed_db", "Speed", 0.5, 60.0, 0.5, Fmt::DbPerS, true),
            ("floor_db", "Hold below", -80.0, -20.0, 1.0, Fmt::Db, true),
        ],
        check: None,
    },
    BlockDef {
        id: "comp",
        name: "Compressor",
        mic: false,
        hint: "",
        meter: Some("comp"),
        sliders: &[
            ("threshold_db", "Threshold", -60.0, 0.0, 0.5, Fmt::Db, false),
            ("ratio", "Ratio", 1.0, 20.0, 0.1, Fmt::Ratio, false),
            ("makeup_db", "Makeup", 0.0, 24.0, 0.5, Fmt::Db, false),
            ("attack_ms", "Attack", 0.1, 200.0, 0.1, Fmt::Ms1, true),
            ("release_ms", "Release", 10.0, 2000.0, 1.0, Fmt::Ms0, true),
            ("knee_db", "Knee", 0.0, 24.0, 0.5, Fmt::Db, true),
        ],
        check: Some(("auto_makeup", "Auto makeup")),
    },
    BlockDef {
        id: "deesser",
        name: "De-esser",
        mic: true,
        hint: "",
        meter: Some("deess"),
        sliders: &[
            ("frequency", "Frequency", 3000.0, 10000.0, 100.0, Fmt::Hz, false),
            ("threshold_db", "Threshold", -30.0, 0.0, 0.5, Fmt::Db, false),
            ("amount_db", "Amount", 0.0, 24.0, 0.5, Fmt::Db, false),
        ],
        check: None,
    },
    BlockDef {
        id: "limiter",
        name: "Limiter",
        mic: false,
        hint: "",
        meter: Some("limit"),
        sliders: &[
            ("ceiling_db", "Ceiling", -24.0, 0.0, 0.1, Fmt::Dbfs, false),
            ("release_ms", "Release", 5.0, 2000.0, 1.0, Fmt::Ms0, true),
            ("lookahead_ms", "Lookahead", 0.0, 5.0, 0.1, Fmt::Ms1, true),
        ],
        check: None,
    },
];

fn format(app: &AppHandle, fmt: Fmt, v: f64) -> String {
    match fmt {
        Fmt::Pct => pct(v),
        Fmt::Db => tf(app, "{0} dB", &[&signed(v)]),
        Fmt::Ratio => format!("{v:.1}:1"),
        Fmt::Ms1 => tf(app, "{0} ms", &[&format!("{v:.1}")]),
        Fmt::Ms0 => tf(app, "{0} ms", &[&format!("{}", v.round())]),
        Fmt::DbPerS => tf(app, "{0} dB/s", &[&format!("{v:.1}")]),
        Fmt::Hz => tf(app, "{0} Hz", &[&hz(v)]),
        Fmt::Dbfs => tf(app, "{0} dBFS", &[&signed(v)]),
    }
}

fn shown_blocks(target: &str) -> impl Iterator<Item = &'static BlockDef> {
    let mic = target == "chatCapture";
    BLOCKS.iter().filter(move |b| mic || !b.mic)
}

fn processing(g: &AudioPage, app: &AppHandle, s: &St) {
    let Some(target) = s.proc.as_deref() else {
        g.set_proc_open(false);
        return;
    };
    let Some(p) = s.proc_of(target) else {
        g.set_proc_open(false);
        return;
    };
    g.set_proc_open(true);
    let mic = target == "chatCapture";
    let l = tr(app, label_of(target));
    g.set_proc_title(tf(app, "{0} processing", &[&l]).into());
    let mut presets = vec![opt("", &tr(app, if mic { "Mic preset" } else { "Preset" }), false)];
    for x in s.profiles(kind_of(target)).into_iter().filter(|x| x["processing"].is_object()) {
        let name = x["name"].as_str().unwrap_or_default();
        presets.push(opt(name, name, false));
    }
    fill(g.get_proc_presets(), presets, |m| g.set_proc_presets(m));
    g.set_proc_mic(mic);
    if mic {
        mic_rows(g, app, s, p);
    }
    g.set_proc_note(
        if mic {
            tr(app, "Noise removal and the gate come first and the limiter last, then volume.")
        } else if target == "master" {
            tr(app, "Works on the whole mix, after every channel's volume.")
        } else {
            String::new()
        }
        .into(),
    );
    let mut blocks = Vec::new();
    let mut sliders = Vec::new();
    let cfg = s.cfg();
    for (i, b) in shown_blocks(target).enumerate() {
        let gate = json!({
            "on": cfg["gate_on"], "threshold_db": cfg["gate_db"],
            "attack_ms": p["gate"]["attack_ms"], "hold_ms": p["gate"]["hold_ms"], "release_ms": p["gate"]["release_ms"],
        });
        let v = if b.id == "gate" { &gate } else { &p[b.id] };
        let engines = ["light", "strong", "strong_ll", "auto"];
        let (modes, mode, info) = if b.id == "denoise" {
            let names = ["Light (RNNoise)", "Strong (DeepFilterNet)", "Strong, low latency", "Auto"];
            let modes: Vec<AudOpt> = engines.iter().zip(names).map(|(id, n)| opt(id, &tr(app, n), false)).collect();
            let now = engines.iter().position(|e| v["engine"] == *e).unwrap_or(0) as i32;
            (modes, now, engine_info(app))
        } else {
            (Vec::new(), 0, String::new())
        };
        blocks.push(AudBlock {
            id: b.id.into(),
            name: tr(app, b.name).into(),
            hint: if b.hint.is_empty() { SharedString::new() } else { tr(app, b.hint).into() },
            on: v["on"].as_bool().unwrap_or(false),
            meter: b.meter.is_some(),
            agc: b.meter == Some("agc"),
            gr_title: tr(app, if b.meter == Some("agc") { "Gain applied now" } else { "Gain reduction" }).into(),
            check: b.check.map_or(SharedString::new(), |c| tr(app, c.1).into()),
            checked: b.check.is_some_and(|c| v[c.0].as_bool().unwrap_or(false)),
            modes: slint::ModelRc::new(slint::VecModel::from(modes)),
            mode,
            info: info.into(),
        });
        for (field, label, min, max, step, fmt, adv) in b.sliders {
            let x = v[*field].as_f64().unwrap_or(*min as f64);
            sliders.push(AudPSlider {
                block: i as i32,
                field: (*field).into(),
                label: tr(app, label).into(),
                value: x as f32,
                min: *min,
                max: *max,
                step: *step,
                text: format(app, *fmt, x).into(),
                adv: *adv,
            });
        }
    }
    fill(g.get_blocks(), blocks, |m| g.set_blocks(m));
    fill(g.get_psliders(), sliders, |m| g.set_psliders(m));
    paint_proc(g, s);
}

const STAGES: [(&str, &str); 4] =
    [("agc", "Level stabilizer"), ("comp", "Compressor"), ("deesser", "De-esser"), ("eq", "EQ")];
pub(super) const TARGETS: [(&str, &str); 6] = [
    ("discord", "Discord"),
    ("streaming", "Streaming"),
    ("podcast", "Podcast"),
    ("gaming", "Gaming"),
    ("meeting", "Meeting"),
    ("music", "Music"),
];

/// The block order choice, auto-tune and its summary, for the mic.
fn mic_rows(g: &AudioPage, app: &AppHandle, s: &St, p: &Value) {
    let now: Vec<String> = p["order"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect();
    let label = |order: &[&str]| {
        order.iter().map(|id| tr(app, STAGES.iter().find(|s| s.0 == *id).map_or(*id, |s| s.1))).collect::<Vec<_>>().join(" → ")
    };
    let mut orders: Vec<Vec<&str>> = (0..4)
        .map(|at| {
            let mut o = vec!["agc", "comp", "deesser"];
            o.insert(at, "eq");
            o
        })
        .collect();
    if !now.is_empty() && !orders.iter().any(|o| o.iter().eq(now.iter())) {
        orders.push(now.iter().map(String::as_str).collect());
    }
    let index = orders.iter().position(|o| o.iter().eq(now.iter())).unwrap_or(3) as i32;
    let options: Vec<AudOpt> = orders.iter().map(|o| opt(&o.join(","), &label(o), false)).collect();
    fill(g.get_order_options(), options, |m| g.set_order_options(m));
    g.set_order_index(index);
    let targets: Vec<AudOpt> = TARGETS.iter().map(|(id, n)| opt(id, &tr(app, n), false)).collect();
    fill(g.get_tune_targets(), targets, |m| g.set_tune_targets(m));
    g.set_tune_index(s.tune_target as i32);
    g.set_tune_busy(s.tune_busy);
    g.set_tune_undo(crate::mic_check::can_undo());
    g.set_tune_summary(s.tune_summary.clone().into());
}

/// Delay and CPU use of the noise removal engines, as last measured on the audio thread.
fn engine_info(app: &AppHandle) -> String {
    let Some(st) = foxden_mixer::denoise::status() else {
        return tr(app, "Light and strong low latency add about 20 ms, strong about 40 ms; the strong models use more CPU.");
    };
    let mut line = tf(
        app,
        "Running {0}, {1} ms delay. Light: {2} ms a frame, {3}% of a core. Strong: {4} ms, {5}%. Strong low latency: {6} ms, {7}%.",
        &[
            &tr(app, match st.running {
                "strong" => "strong",
                "strong_ll" => "strong, low latency",
                _ => "light",
            }),
            &format!("{:.0}", st.latency_ms),
            &format!("{:.2}", st.light_ms),
            &format!("{:.1}", st.light_cpu_percent),
            &format!("{:.2}", st.strong_ms),
            &format!("{:.1}", st.strong_cpu_percent),
            &format!("{:.2}", st.strong_ll_ms),
            &format!("{:.1}", st.strong_ll_cpu_percent),
        ],
    );
    if st.fell_back {
        line.push(' ');
        line.push_str(&tr(app, "Strong could not keep up, so light took over."));
    }
    line
}

/// Gain reduction meters of the open processing drawer.
pub(super) fn paint_proc(g: &AudioPage, s: &St) {
    let Some(target) = s.proc.as_deref() else { return };
    let Some(p) = s.proc_of(target) else { return };
    let n = if target == "master" { 5 } else { CHANNELS.iter().position(|x| x.0 == target).unwrap_or(0) };
    let r = &s.reduction[n];
    let rows: Vec<AudGr> = shown_blocks(target)
        .map(|b| {
            let Some(meter) = b.meter else {
                return AudGr { fill: 0.0, up: false, text: SharedString::new() };
            };
            let on = p[b.id]["on"].as_bool().unwrap_or(false) && s.running();
            let v = if on { r[meter].as_f64().unwrap_or(0.0) } else { 0.0 };
            let text = if !on {
                "–".to_string()
            } else if meter == "agc" {
                format!("{} dB", signed((v * 10.0).round() / 10.0))
            } else if v.abs() < 0.05 {
                "0.0 dB".to_string()
            } else {
                format!("−{:.1} dB", v.abs())
            };
            AudGr { fill: (v.abs() / 24.0).min(1.0) as f32, up: v > 0.0 && meter == "agc", text: text.into() }
        })
        .collect();
    fill(g.get_gr(), rows, |m| g.set_gr(m));
}

fn advanced(g: &AudioPage, app: &AppHandle, s: &St) {
    let cfg = s.cfg();
    g.set_auto_assign(cfg["auto_assign"].as_bool().unwrap_or(false));
    let skipped = strs(&cfg["no_auto"])
        .iter()
        .map(|exe| AudSkip {
            exe: exe.as_str().into(),
            name: short(exe).into(),
            label: tf(app, "Let auto-assign place {0} again", &[exe]).into(),
        })
        .collect();
    fill(g.get_skipped(), skipped, |m| g.set_skipped(m));
    let output = cfg["output"].as_str().unwrap_or_default();
    let outputs = strs(&s.mix["outputs"]);
    let inputs = strs(&s.mix["inputs"]);
    {
        let park = cfg["park"].as_str().unwrap_or_default();
        let mut list = vec![opt("", &tr(app, "Pick one for me"), false)];
        list.extend(outputs.iter().filter(|o| *o != output).map(|o| opt(o, o, false)));
        let (list, i) = choose(list, park, tf(app, "{0} (missing)", &[park]));
        fill(g.get_parks(), list, |m| g.set_parks(m));
        g.set_park_index(i);
    }
    let parked = s.mix["parked_on"].as_str().filter(|_| s.running());
    g.set_parked(parked.map_or(String::new(), |p| tf(app, "Now using {0}.", &[p])).into());
    let old: Vec<AudSrc> = g.get_sources().iter().collect();
    let mut rows = Vec::new();
    let null = Value::Null;
    for (n, (id, label)) in CHANNELS[..4].iter().enumerate() {
        let c = chan_of(cfg, id).unwrap_or(&null);
        let source = c["source"].as_str().unwrap_or_default();
        let mut list = vec![opt("", &tr(app, "Nothing, apps only"), false)];
        list.extend(outputs.iter().map(|o| opt(&format!("out:{o}"), &tf(app, "Everything on {0}", &[o]), o == output)));
        list.extend(inputs.iter().map(|i| opt(&format!("in:{i}"), i, false)));
        let current = if source.is_empty() {
            String::new()
        } else {
            format!("{}{source}", if c["loopback"].as_bool().unwrap_or(false) { "out:" } else { "in:" })
        };
        let (list, i) = choose(list, &current, tf(app, "{0} (missing)", &[source]));
        let same = old.get(n).is_some_and(|o| o.options.iter().eq(list.iter().cloned()));
        rows.push(AudSrc {
            channel: (*id).into(),
            label: tr(app, label).into(),
            options: if same { old[n].options.clone() } else { slint::ModelRc::new(slint::VecModel::from(list)) },
            index: i,
        });
    }
    fill(g.get_sources(), rows, |m| g.set_sources(m));
}

fn volumes(g: &AudioPage, app: &AppHandle, s: &St) {
    let v = &s.vols;
    g.set_vol_problem(v["problem"].as_str().unwrap_or_default().into());
    if !v.is_object() {
        return;
    }
    let cfg = s.cfg();
    let mut rows = Vec::new();
    let mut levels = Vec::new();
    let mut active = 0;
    for a in v["apps"].as_array().into_iter().flatten() {
        let exe = a["exe"].as_str().unwrap_or_default();
        let name = short(exe);
        let is_active = a["active"].as_bool().unwrap_or(false);
        if is_active {
            active += 1;
        }
        let ch = cfg["channels"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|c| c["channel"] != "chatCapture" && apps_of(c).iter().any(|x| x.eq_ignore_ascii_case(exe)))
            .map(|c| label_of(c["channel"].as_str().unwrap_or_default()));
        let device = a["device"].as_str().filter(|d| !d.is_empty()).map_or_else(|| tr(app, "Unknown output"), String::from);
        let mut parts = vec![device];
        if let Some(ch) = ch {
            parts.push(tf(app, "FoxDen {0}", &[&tr(app, ch)]));
        }
        if !is_active {
            parts.push(tr(app, "idle"));
        }
        let key = vol_key('a', a);
        levels.push(s.peaks.get(&key).copied().unwrap_or(0.0).min(1.0));
        rows.push(AudVol {
            key: key.into(),
            name: name.as_str().into(),
            sub: parts.join(" · ").into(),
            volume: f(&a["volume"]),
            muted: a["muted"].as_bool().unwrap_or(false),
            idle: !is_active,
            special: false,
            chosen: false,
            capture: false,
            make: false,
            make_label: SharedString::new(),
            vol_label: tf(app, "{0} volume", &[&name]).into(),
            mute_label: tf(app, "Mute {0}", &[&name]).into(),
        });
    }
    fill(g.get_apps(), rows, |m| g.set_apps(m));
    fill(g.get_app_levels(), levels, |m| g.set_app_levels(m));
    g.set_apps_count(active.to_string().into());

    let pv = &s.prio;
    let picked: HashSet<&str> = if pv["config"]["on"].as_bool().unwrap_or(false) {
        ["outputs", "inputs"]
            .iter()
            .flat_map(|l| pv[*l].as_array().into_iter().flatten())
            .filter(|r| r["chosen"].as_bool().unwrap_or(false))
            .filter_map(|r| r["id"].as_str())
            .collect()
    } else {
        HashSet::new()
    };
    let mut rows = Vec::new();
    let mut levels = Vec::new();
    for d in v["devices"].as_array().into_iter().flatten() {
        let id = d["id"].as_str().unwrap_or_default();
        let name = d["name"].as_str().unwrap_or_default();
        let capture = d["capture"].as_bool().unwrap_or(false);
        let default = d["default"].as_bool().unwrap_or(false);
        let note = d["note"].as_str();
        let mut tags = Vec::new();
        if default {
            tags.push(tr(app, "Default"));
        }
        if picked.contains(id) {
            tags.push(tr(app, "Priority pick"));
        }
        if let Some(n) = note {
            tags.push(tr(app, n));
        }
        let key = vol_key('d', d);
        levels.push(s.peaks.get(&key).copied().unwrap_or(0.0).min(1.0));
        rows.push(AudVol {
            key: key.into(),
            name: name.into(),
            sub: tags.join(" · ").into(),
            volume: f(&d["volume"]),
            muted: d["muted"].as_bool().unwrap_or(false),
            idle: false,
            special: note.is_some(),
            chosen: default,
            capture,
            make: !default && note.is_none(),
            make_label: tf(
                app,
                if capture { "Make {0} the default recording device" } else { "Make {0} the default playback device" },
                &[name],
            )
            .into(),
            vol_label: tf(app, "{0} volume", &[name]).into(),
            mute_label: tf(app, "Mute {0}", &[name]).into(),
        });
    }
    g.set_devs_count(rows.len().to_string().into());
    fill(g.get_devices(), rows, |m| g.set_devices(m));
    fill(g.get_dev_levels(), levels, |m| g.set_dev_levels(m));
}
