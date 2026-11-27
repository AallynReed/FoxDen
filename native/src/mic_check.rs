//! Mic samples: recorded into memory, measured raw and through the mic chain, then dropped.
//! Auto-tune works out a whole chain for one use from a sample.

use std::sync::Mutex;

use foxden_mixer::Channel;
use foxden_mixer::analysis::{self, Analysis};
use foxden_mixer::chain::Processing;
use foxden_mixer::dsp::Band;
use foxden_mixer::tune::{self, Target, Tuned};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager};

use crate::mixer::{Change, Mixer};
use crate::state::lock;

pub const MIN_SECONDS: f64 = 1.0;
pub const MAX_SECONDS: f64 = 10.0;

/// The mic channel as the mixer has it: where it records from and its whole chain.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MicSetup {
    pub source: String,
    pub loopback: bool,
    pub processing: Processing,
    pub eq_on: bool,
    pub eq: Vec<Band>,
    pub gate_on: bool,
    pub gate_db: f32,
}

/// The chain auto-tune replaced, for undo.
static UNDO: Mutex<Option<MicSetup>> = Mutex::new(None);

pub fn setup_of(config: &Value) -> Result<MicSetup, String> {
    let mic = config["channels"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|c| c["channel"] == "chatCapture")
        .ok_or("The mixer has no mic channel.")?;
    fn read<T: serde::de::DeserializeOwned>(v: &Value) -> Result<T, String> {
        serde_json::from_value(v.clone()).map_err(|e| e.to_string())
    }
    Ok(MicSetup {
        source: mic["source"].as_str().unwrap_or_default().to_string(),
        loopback: mic["loopback"].as_bool().unwrap_or(false),
        processing: read(&mic["processing"])?,
        eq_on: mic["eq_on"].as_bool().unwrap_or(false),
        eq: read(&mic["eq"])?,
        gate_on: config["gate_on"].as_bool().unwrap_or(false),
        gate_db: config["gate_db"].as_f64().unwrap_or(-50.0) as f32,
    })
}

fn mixer(app: &AppHandle) -> Result<tauri::State<'static, Mixer>, String> {
    app.try_state::<Mixer>().ok_or_else(|| "The audio mixer isn't available.".into())
}

pub fn current(app: &AppHandle) -> Result<MicSetup, String> {
    let view = serde_json::to_value(crate::mixer::get_mixer(mixer(app)?, false)?).map_err(|e| e.to_string())?;
    setup_of(&view["config"])
}

/// The mixer changes that put `s` on the mic channel.
pub fn changes(s: &MicSetup) -> Vec<Change> {
    vec![
        Change::Processing { channel: Some(Channel::ChatCapture), processing: s.processing },
        Change::Eq { channel: Channel::ChatCapture, enabled: s.eq_on, bands: s.eq.clone() },
        Change::Gate { on: s.gate_on, threshold_db: s.gate_db },
    ]
}

pub fn apply(app: &AppHandle, s: &MicSetup) -> Result<(), String> {
    for change in changes(s) {
        crate::mixer::mixer_set(mixer(app)?, change)?;
    }
    let _ = app.emit("mixer", Value::Null);
    Ok(())
}

/// Records from the mic channel's source, into memory only.
pub fn record(s: &MicSetup, seconds: f64) -> Result<(Vec<f32>, u32), String> {
    if s.source.is_empty() {
        return Err("Pick a microphone for the Mic channel first.".into());
    }
    foxden_mixer::router::record(&s.source, s.loopback, seconds as f32)
}

fn processed(raw: &[f32], rate: f32, s: &MicSetup) -> Analysis {
    let eq = s.eq_on.then_some(s.eq.as_slice());
    analysis::analyze(&tune::run_chain(raw, rate, &s.processing, eq, s.gate_on.then_some(s.gate_db)), rate)
}

/// Raw and processed measurements of a sample, and how they differ.
pub fn measure(raw: &[f32], rate: u32, s: &MicSetup) -> Value {
    let r = analysis::analyze(raw, rate as f32);
    let p = processed(raw, rate as f32, s);
    json!({
        "seconds": r.seconds, "sample_rate": rate, "source": s.source,
        "difference": analysis::compare(&r, &p), "raw": r, "processed": p,
    })
}

pub struct Tuning {
    pub tuned: Tuned,
    pub before: Analysis,
    pub after: Analysis,
    pub raw: Analysis,
    pub setup: MicSetup,
}

/// Works out the chain for `target` from a sample; nothing changes until [`commit`].
pub fn work_out(raw: &[f32], rate: u32, s: &MicSetup, target: Target, mono: bool) -> Tuning {
    let rate_f = rate as f32;
    let measured = analysis::analyze(raw, rate_f);
    let before = processed(raw, rate_f, s);
    let (tuned, after) = tune::tune_recording(raw, rate_f, &measured, target, mono);
    let setup = MicSetup {
        source: s.source.clone(),
        loopback: s.loopback,
        processing: tuned.processing,
        eq_on: true,
        eq: tuned.eq.clone(),
        gate_on: tuned.gate_on,
        gate_db: tuned.gate_db,
    };
    Tuning { tuned, before, after, raw: measured, setup }
}

/// Puts a worked-out chain on the mic and keeps the old one for undo.
pub fn commit(app: &AppHandle, old: &MicSetup, t: &Tuning) -> Result<(), String> {
    apply(app, &t.setup)?;
    *lock(&UNDO) = Some(old.clone());
    Ok(())
}

pub fn can_undo() -> bool {
    lock(&UNDO).is_some()
}

pub fn undo(app: &AppHandle) -> Result<MicSetup, String> {
    let old = lock(&UNDO).clone().ok_or("There is no auto-tune to undo.")?;
    apply(app, &old)?;
    *lock(&UNDO) = None;
    Ok(old)
}

pub fn report(t: &Tuning) -> Value {
    json!({
        "target": t.tuned.target, "target_lufs": t.tuned.target_lufs, "notes": t.tuned.notes,
        "settings": {
            "processing": t.tuned.processing, "eq": t.tuned.eq,
            "gate": {"on": t.tuned.gate_on, "threshold_db": t.tuned.gate_db},
        },
        "raw": t.raw,
        "before": t.before,
        "after": t.after,
        "change": analysis::compare(&t.before, &t.after),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mic_channel_is_read_and_written_whole() {
        let config = serde_json::to_value(crate::mixer::AudioConfig::default()).unwrap();
        let s = setup_of(&config).unwrap();
        assert_eq!((s.source.as_str(), s.gate_db, s.eq.len()), ("", -50.0, 10));
        let c = changes(&s);
        assert!(matches!(c[0], Change::Processing { channel: Some(Channel::ChatCapture), .. }));
        assert!(matches!(c[2], Change::Gate { on: false, .. }));
        assert!(record(&s, 2.0).unwrap_err().contains("Pick a microphone"));
    }

    #[test]
    fn work_out_reports_before_and_after_without_changing_anything() {
        let config = serde_json::to_value(crate::mixer::AudioConfig::default()).unwrap();
        let s = setup_of(&config).unwrap();
        let raw: Vec<f32> = (0..96000).map(|i| 0.05 * (i as f32 * 0.04).sin() + if i % 977 == 0 { 0.2 } else { 0.0 }).collect();
        let t = work_out(&raw, 48000, &s, Target::Music, false);
        assert_eq!(t.setup.processing, t.tuned.processing);
        let r = report(&t);
        assert_eq!(r["target"], "music");
        assert!(r["after"]["lufs"].as_f64().unwrap() > r["before"]["lufs"].as_f64().unwrap());
        assert!(r["settings"]["processing"]["denoise"]["on"] == false);
        let m = measure(&raw, 48000, &s);
        assert_eq!(m["difference"]["lufs"], 0.0, "a chain with everything off changes nothing");
    }
}
