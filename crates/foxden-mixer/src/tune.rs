//! Mic settings worked out from an analysis of a sample, for one use.

use serde::{Deserialize, Serialize};

use crate::analysis::Analysis;
use crate::chain::{MicChain, Processing, Stage};
use crate::denoise::{DenoiseSettings, Engine};
use crate::dsp::{self, Band, Equalizer, FilterType, GateTiming};
use crate::dynamics::{AgcSettings, CompSettings, DeEsserSettings, LimiterSettings};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    /// Clear and steady speech for voice chat.
    Discord,
    Streaming,
    /// Warm, full range and natural.
    Podcast,
    /// Voice chat over keyboard and mouse clicks and shouted callouts, with little delay.
    Gaming,
    /// Teams, Zoom or Meet: intelligible through low-bitrate codecs and the app's own suppression.
    Meeting,
    /// Singing or an instrument, kept natural.
    Music,
}

pub const TARGETS: [Target; 6] =
    [Target::Discord, Target::Streaming, Target::Podcast, Target::Gaming, Target::Meeting, Target::Music];

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Tuned {
    pub target: Target,
    pub target_lufs: f32,
    pub processing: Processing,
    pub gate_on: bool,
    pub gate_db: f32,
    pub eq: Vec<Band>,
    /// Why each block was set as it was.
    pub notes: Vec<String>,
}

enum Gating {
    /// Opens this far over the floor, staying this far under the voice.
    Firm(f32, f32),
    /// Only over a floor louder than the first value.
    Soft(f32, f32, f32),
    /// Over the floor and over the clicks between words.
    Clicks,
}

struct Aim {
    lufs: f32,
    ceiling: f32,
    lookahead: f32,
    limit_release: f32,
    high_pass: f32,
    low_pass: Option<f32>,
    gating: Gating,
    gate: GateTiming,
    stabilize: bool,
    ratio: f32,
    attack: f32,
    release: f32,
    knee: f32,
    comp_above: f32,
    agc_speed: f32,
    deess_from: f32,
    deess_amount: f32,
    mud_over: f32,
    mud_cut: f32,
    presence: f32,
    presence_boost: f32,
}

fn timing(attack_ms: f32, hold_ms: f32, release_ms: f32) -> GateTiming {
    GateTiming { attack_ms, hold_ms, release_ms }
}

fn aim(target: Target, mono: bool) -> Aim {
    let base = Aim {
        lufs: -20.0, ceiling: -3.0, lookahead: 2.0, limit_release: 120.0, high_pass: 100.0, low_pass: Some(12000.0),
        gating: Gating::Firm(14.0, 6.0), gate: GateTiming::default(), stabilize: true,
        ratio: 2.0, attack: 10.0, release: 150.0, knee: 8.0, comp_above: 6.0, agc_speed: 6.0,
        deess_from: -14.0, deess_amount: 6.0, mud_over: 4.0, mud_cut: 5.0, presence: -6.0, presence_boost: 5.0,
    };
    match target {
        Target::Discord => base,
        Target::Streaming => Aim {
            lufs: -16.0, ceiling: -1.0, limit_release: 80.0, high_pass: 80.0, low_pass: None, gating: Gating::Firm(12.0, 6.0),
            ratio: 3.5, attack: 5.0, release: 120.0, knee: 6.0, comp_above: 4.0,
            deess_amount: 8.0, mud_cut: 4.0, presence: -8.0, presence_boost: 4.0, ..base
        },
        Target::Podcast => Aim {
            lufs: if mono { -19.0 } else { -16.0 }, ceiling: -1.0, high_pass: 70.0, low_pass: None,
            gating: Gating::Soft(-60.0, 8.0, 10.0), gate: timing(5.0, 250.0, 150.0),
            ratio: 2.5, attack: 20.0, release: 250.0, knee: 10.0, agc_speed: 3.0,
            deess_from: -12.0, deess_amount: 4.0, mud_cut: 3.0, presence: -10.0, presence_boost: 2.5, ..base
        },
        Target::Gaming => Aim {
            lufs: -17.0, ceiling: -3.0, lookahead: 0.0, limit_release: 50.0, gating: Gating::Clicks, gate: timing(0.5, 40.0, 30.0),
            ratio: 4.0, attack: 1.5, release: 60.0, knee: 4.0, comp_above: 0.0, agc_speed: 8.0, ..base
        },
        Target::Meeting => Aim {
            lufs: -18.0, ceiling: -2.0, high_pass: 110.0, low_pass: None, gating: Gating::Soft(-60.0, 8.0, 10.0),
            gate: timing(5.0, 300.0, 200.0), ratio: 2.5, attack: 10.0, release: 200.0, knee: 10.0, comp_above: 5.0,
            agc_speed: 4.0, deess_from: -13.0, deess_amount: 4.0, mud_cut: 3.0, presence: -6.0, presence_boost: 3.0, ..base
        },
        Target::Music => Aim {
            lufs: -14.0, ceiling: -1.0, limit_release: 150.0, high_pass: 40.0, low_pass: None,
            gating: Gating::Soft(-50.0, 6.0, 20.0), gate: timing(10.0, 400.0, 400.0), stabilize: false,
            ratio: 1.5, attack: 30.0, release: 300.0, knee: 12.0, comp_above: 10.0,
            deess_from: -8.0, deess_amount: 3.0, mud_over: 8.0, mud_cut: 3.0, presence: -30.0, presence_boost: 0.0, ..base
        },
    }
}

fn level(a: &Analysis, lo: f32, hi: f32) -> Option<f32> {
    let v: Vec<f32> = a.bands.iter().filter(|b| b.hz >= lo && b.hz <= hi).map(|b| b.db).collect();
    (!v.is_empty()).then(|| v.iter().sum::<f32>() / v.len() as f32)
}

fn r1(v: f32) -> f32 {
    (v * 10.0).round() / 10.0
}

/// Clicks between words are many and sharp enough for the strong engine.
fn clicky(a: &Analysis) -> bool {
    a.clicks_per_second > 1.0 && a.transient_crest_db > 12.0
}

pub fn tune(a: &Analysis, target: Target, mono: bool) -> Tuned {
    let aim = aim(target, mono);
    let mut notes = Vec::new();
    let floor = a.noise_floor_dbfs.max(-100.0);
    let speech = if a.speech_dbfs > floor { a.speech_dbfs } else { floor + 20.0 };

    // 0 with a floor at -70 dBFS or under, 1 at -40 or over.
    let noisy = ((floor + 70.0) / 30.0).clamp(0.0, 1.0);
    let strength = match target {
        Target::Discord => noisy.max(0.8),
        Target::Streaming => 0.2 + noisy * 0.8,
        Target::Podcast => noisy * 0.6,
        Target::Gaming if clicky(a) => 1.0,
        Target::Gaming => noisy.max(0.85),
        // Calls run their own suppression on top; this keeps the two from sounding watery.
        Target::Meeting => (0.6 + noisy * 0.25).min(0.85),
        // Both engines smear tones.
        Target::Music => 0.0,
    };
    let engine = match target {
        Target::Gaming | Target::Music => Engine::Light,
        Target::Meeting | Target::Podcast | Target::Streaming => Engine::Strong,
        Target::Discord if clicky(a) => Engine::Strong,
        Target::Discord => Engine::Light,
    };
    let denoise = DenoiseSettings { on: strength >= 0.1, strength: (strength * 100.0).round() / 100.0, engine };
    notes.push(if denoise.on {
        format!(
            "Noise removal {} at {:.0}% for a floor of {floor:.0} dBFS and {:.1} clicks a second.",
            if engine == Engine::Strong { "strong" } else { "light" },
            denoise.strength * 100.0,
            a.clicks_per_second
        )
    } else if target == Target::Music {
        "Noise removal off: it would damage tones.".into()
    } else {
        format!("Noise removal off: the floor of {floor:.0} dBFS is low enough.")
    });

    let gate_at = match aim.gating {
        Gating::Firm(over, under) => Some((floor + over).min(speech - under)),
        Gating::Soft(from, over, under) => (floor > from).then(|| (floor + over).min(speech - under)),
        Gating::Clicks => Some((floor + 12.0).max(a.click_peak_dbfs + 2.0).min(speech - 6.0)),
    };
    let (gate_on, gate_db) = match gate_at {
        Some(db) => (true, r1(db.clamp(-80.0, -20.0))),
        None => (false, -60.0),
    };
    notes.push(if gate_on {
        format!(
            "Gate opens at {gate_db:.0} dBFS, over the floor and under the voice; holds {:.0} ms.",
            aim.gate.hold_ms
        )
    } else {
        "Gate off: the room is quiet enough to leave it natural.".into()
    });

    let offset = if a.lufs > -70.0 { a.lufs - speech } else { 0.0 };
    let agc_target = r1((aim.lufs - offset).clamp(-40.0, -6.0));
    let agc = if aim.stabilize {
        notes.push(format!(
            "Level stabilizer aims at {agc_target:.0} dBFS RMS for about {:.0} LUFS; the voice measured {:.0} LUFS.",
            aim.lufs, a.lufs
        ));
        AgcSettings {
            on: true,
            target_db: agc_target,
            max_gain_db: r1((agc_target - speech + 6.0).clamp(6.0, 30.0)),
            max_cut_db: 12.0,
            speed_db: aim.agc_speed,
            floor_db: r1((floor + 6.0).clamp(-80.0, -20.0)),
        }
    } else {
        AgcSettings::default()
    };
    // Without the stabilizer the compressor's makeup brings the level up.
    let makeup = if aim.stabilize { 0.0 } else { r1((aim.lufs - a.lufs).clamp(0.0, 24.0)) };
    if !aim.stabilize {
        notes.push(format!(
            "No level stabilizer, to keep the dynamics; {makeup:.1} dB of makeup for about {:.0} LUFS from {:.0}.",
            aim.lufs, a.lufs
        ));
    }
    let comp = CompSettings {
        on: true,
        threshold_db: if aim.stabilize {
            r1((agc_target + aim.comp_above).min(-3.0))
        } else {
            r1((speech + aim.comp_above).min(-3.0))
        },
        ratio: aim.ratio,
        attack_ms: aim.attack,
        release_ms: aim.release,
        knee_db: aim.knee,
        makeup_db: makeup,
        auto_makeup: false,
    };

    let excess = a.sibilance_peak_db - aim.deess_from;
    let deesser = if excess > 0.0 {
        let amount = r1((aim.deess_amount * (0.5 + excess / 12.0)).clamp(2.0, aim.deess_amount * 1.5));
        let s = DeEsserSettings {
            on: true,
            frequency: a.sibilance_hz.clamp(4000.0, 9000.0),
            amount_db: amount,
            threshold_db: r1((a.sibilance_peak_db - amount / 1.5).clamp(-30.0, 0.0)),
        };
        notes.push(format!("De-esser at {:.0} Hz: sibilance peaks {:.0} dB next to the voice.", s.frequency, a.sibilance_peak_db));
        s
    } else {
        notes.push("De-esser off: little sibilance.".into());
        DeEsserSettings::default()
    };
    let limiter = LimiterSettings { on: true, ceiling_db: aim.ceiling, release_ms: aim.limit_release, lookahead_ms: aim.lookahead };

    let mut eq = dsp::default_bands();
    let high_pass = if a.rumble_db > -25.0 { aim.high_pass + 20.0 } else { aim.high_pass };
    eq[0] = Band { enabled: true, filter: FilterType::HighPass, frequency: high_pass, gain_db: 0.0, q: 0.707 };
    notes.push(format!("High pass at {high_pass:.0} Hz; rumble measured {:.0} dB.", a.rumble_db));
    if let (Some(mud), Some(mid), Some(pres), Some(body)) =
        (level(a, 200.0, 315.0), level(a, 800.0, 1250.0), level(a, 2000.0, 3150.0), level(a, 125.0, 160.0))
    {
        let muddy = mud - mid - aim.mud_over;
        if muddy > 0.0 {
            eq[3].gain_db = -r1(muddy.min(aim.mud_cut));
            eq[3].q = 1.0;
            notes.push(format!("Cut {:.1} dB at 250 Hz against boominess.", -eq[3].gain_db));
        }
        let short = aim.presence - (pres - mid);
        if short > 0.0 && aim.presence_boost > 0.0 {
            eq[6].frequency = 3000.0;
            eq[6].gain_db = r1(short.min(aim.presence_boost));
            notes.push(format!("Lift {:.1} dB at 3 kHz for clarity.", eq[6].gain_db));
        }
        if target == Target::Podcast && body - mid < -12.0 {
            eq[2].gain_db = 2.0;
            notes.push("Lift 2 dB at 125 Hz for warmth.".into());
        }
    }
    if let Some(hz) = aim.low_pass {
        eq[9] = Band { enabled: true, filter: FilterType::LowPass, frequency: hz, gain_db: 0.0, q: 0.707 };
    }
    // EQ first, so the stabilizer and compressor hold the level whatever the EQ does.
    let order = [Stage::Eq, Stage::Agc, Stage::Comp, Stage::Deesser];
    let processing = Processing { denoise, agc, comp, deesser, limiter, order, gate: aim.gate };
    Tuned { target, target_lufs: aim.lufs, processing: processing.clean(true), gate_on, gate_db, eq, notes }
}

/// Tunes for a recording, then checks the new chain on it; see [`calibrate`].
pub fn tune_recording(x: &[f32], rate: f32, raw: &Analysis, target: Target, mono: bool) -> (Tuned, Analysis) {
    let mut t = tune(raw, target, mono);
    let after = calibrate(x, rate, &mut t);
    (t, after)
}

/// Plays the recording through the tuned chain and moves the level until the result lands on the
/// target loudness. Returns the analysis of the processed recording.
pub fn calibrate(x: &[f32], rate: f32, t: &mut Tuned) -> Analysis {
    let mut after = processed(x, rate, t);
    for _ in 0..3 {
        let miss = t.target_lufs - after.lufs;
        if after.lufs <= -70.0 || miss.abs() < 0.5 {
            break;
        }
        let step = miss.clamp(-12.0, 12.0);
        let p = &mut t.processing;
        if p.agc.on {
            p.agc.target_db = r1((p.agc.target_db + step).clamp(-40.0, -6.0));
            p.agc.max_gain_db = r1((p.agc.max_gain_db + step.max(0.0)).clamp(6.0, 30.0));
            p.comp.threshold_db = r1((p.comp.threshold_db + step).min(-3.0));
        } else {
            p.comp.makeup_db = r1((p.comp.makeup_db + step).clamp(0.0, 24.0));
        }
        after = processed(x, rate, t);
    }
    t.notes.push(format!("Checked on the sample: {:.1} LUFS after, aiming at {:.0}.", after.lufs, t.target_lufs));
    after
}

fn processed(x: &[f32], rate: f32, t: &Tuned) -> Analysis {
    crate::analysis::analyze(&run_chain(x, rate, &t.processing, Some(&t.eq), t.gate_on.then_some(t.gate_db)), rate)
}

/// Plays a mono recording through a mic chain as the mixer would, once to settle its levels,
/// then again for the result.
pub fn run_chain(x: &[f32], rate: f32, p: &Processing, eq: Option<&[Band]>, gate_db: Option<f32>) -> Vec<f32> {
    let mut chain = MicChain::new(rate, 1, p);
    chain.offline();
    if let Some(bands) = eq {
        chain.eq = Equalizer::new(bands, rate, 1);
    }
    let gate = gate_db.map(dsp::db_to_linear);
    let mut y = x.to_vec();
    for block in y.chunks_mut(480) {
        chain.process(block, gate);
    }
    y.copy_from_slice(x);
    for block in y.chunks_mut(480) {
        chain.process(block, gate);
    }
    let late = chain.latency().min(y.len());
    y.drain(..late);
    y
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::tests::{RATE, noise, sine, talk};
    use crate::analysis::{BandLevel, analyze};

    fn sample(floor: f32, speech: f32, sib: f32, rumble: f32) -> Analysis {
        Analysis {
            seconds: 5.0,
            peak_dbfs: speech + 12.0,
            rms_dbfs: speech - 3.0,
            lufs: speech + 1.0,
            speech_dbfs: speech,
            noise_floor_dbfs: floor,
            snr_db: speech - floor,
            speech_percent: 60.0,
            clipped_samples: 0,
            clipped_percent: 0.0,
            bands: [125.0, 160.0, 200.0, 250.0, 315.0, 800.0, 1000.0, 1250.0, 2000.0, 2500.0, 3150.0]
                .iter()
                .map(|&hz| BandLevel { hz, db: if hz < 400.0 { speech } else if hz < 1500.0 { speech - 6.0 } else { speech - 24.0 } })
                .collect(),
            sibilance_db: sib - 6.0,
            sibilance_peak_db: sib,
            sibilance_hz: 6300.0,
            rumble_db: rumble,
            plosives: 0,
            clicks_per_second: 0.2,
            click_peak_dbfs: floor + 6.0,
            transient_crest_db: 6.0,
        }
    }

    #[test]
    fn every_target_follows_its_aim() {
        let a = sample(-60.0, -24.0, -20.0, -40.0);
        let d = tune(&a, Target::Discord, false);
        let s = tune(&a, Target::Streaming, false);
        let p = tune(&a, Target::Podcast, false);
        assert_eq!((d.target_lufs, s.target_lufs, p.target_lufs), (-20.0, -16.0, -16.0));
        assert_eq!(tune(&a, Target::Podcast, true).target_lufs, -19.0);
        assert_eq!((d.processing.limiter.ceiling_db, s.processing.limiter.ceiling_db, p.processing.limiter.ceiling_db), (-3.0, -1.0, -1.0));
        assert!(d.processing.denoise.strength > s.processing.denoise.strength);
        assert!(s.processing.denoise.strength > p.processing.denoise.strength);
        assert!(d.processing.comp.ratio < s.processing.comp.ratio);
        assert!((2.0..=3.0).contains(&p.processing.comp.ratio) && p.processing.comp.attack_ms > s.processing.comp.attack_ms);
        let hp = |t: &Tuned| (t.eq[0].filter, t.eq[0].frequency);
        assert_eq!((hp(&d), hp(&s), hp(&p)), ((FilterType::HighPass, 100.0), (FilterType::HighPass, 80.0), (FilterType::HighPass, 70.0)));
        assert_eq!(d.eq[9].filter, FilterType::LowPass);
        assert!(d.eq[6].gain_db > p.eq[6].gain_db && p.eq[6].gain_db > 0.0, "presence lift");
        assert!(d.eq[3].gain_db < 0.0, "boomy low mids are cut");
    }

    #[test]
    fn levels_come_from_the_measurement() {
        let quiet = tune(&sample(-60.0, -34.0, -30.0, -40.0), Target::Streaming, false);
        let loud = tune(&sample(-60.0, -14.0, -30.0, -40.0), Target::Streaming, false);
        assert_eq!(quiet.processing.agc.target_db, loud.processing.agc.target_db);
        assert!(quiet.processing.agc.max_gain_db > loud.processing.agc.max_gain_db);

        let noisy = tune(&sample(-42.0, -20.0, -30.0, -40.0), Target::Discord, false);
        let still = tune(&sample(-75.0, -20.0, -30.0, -40.0), Target::Discord, false);
        assert!(noisy.gate_db > still.gate_db);
        assert!(noisy.gate_db < -20.0 - 5.0, "the gate stays under the voice");
        assert!((noisy.processing.denoise.strength - 0.93).abs() < 0.01);
        assert!(!tune(&sample(-80.0, -20.0, -30.0, -40.0), Target::Podcast, false).processing.denoise.on);
        assert!(!tune(&sample(-75.0, -20.0, -30.0, -40.0), Target::Podcast, false).gate_on);
        assert!(tune(&sample(-50.0, -20.0, -30.0, -40.0), Target::Podcast, false).gate_on);

        let hissy = tune(&sample(-60.0, -20.0, -4.0, -40.0), Target::Streaming, false);
        assert!(hissy.processing.deesser.on && hissy.processing.deesser.threshold_db < -4.0);
        assert!(!tune(&sample(-60.0, -20.0, -13.0, -40.0), Target::Podcast, false).processing.deesser.on);
        assert!(tune(&sample(-60.0, -20.0, -13.0, -40.0), Target::Discord, false).processing.deesser.on);
        let rumbly = tune(&sample(-60.0, -20.0, -30.0, -12.0), Target::Podcast, false);
        assert_eq!(rumbly.eq[0].frequency, 90.0);
    }

    #[test]
    fn a_tuned_chain_reaches_its_targets() {
        let mut x = talk(10.0, 0.1, 0.002);
        for (v, s) in x.iter_mut().zip(sine(7000.0, 0.002, 10.0)) {
            *v += s;
        }
        for (v, n) in x.iter_mut().zip(noise(0.001, 480000, 3)) {
            *v += n;
        }
        let raw = analyze(&x, RATE);
        for target in TARGETS {
            let mut t = tune(&raw, target, false);
            // DeepFilterNet takes a synthetic buzz for noise, so the level maths is checked with light.
            t.processing.denoise.engine = Engine::Light;
            let after = calibrate(&x, RATE, &mut t);
            assert!((after.lufs - t.target_lufs).abs() < 2.0, "{target:?}: {} LUFS, {:?}", after.lufs, t.notes);
            assert!(after.peak_dbfs <= t.processing.limiter.ceiling_db + 0.3, "{target:?}: peak {}", after.peak_dbfs);
            assert!(target == Target::Music || after.snr_db > raw.snr_db, "{target:?}: {} dB to {} dB", raw.snr_db, after.snr_db);
        }
    }

    #[test]
    fn gaming_is_fast_and_shuts_out_clicks() {
        let mut a = sample(-60.0, -22.0, -20.0, -40.0);
        let calm = tune(&a, Target::Gaming, false);
        (a.clicks_per_second, a.click_peak_dbfs, a.transient_crest_db) = (4.0, -38.0, 22.0);
        let clicks = tune(&a, Target::Gaming, false);
        let discord = tune(&a, Target::Discord, false);
        assert!(clicks.gate_db > calm.gate_db && (-36.0..=-28.0).contains(&clicks.gate_db), "{}", clicks.gate_db);
        assert!(clicks.processing.gate.hold_ms < discord.processing.gate.hold_ms);
        assert!(clicks.processing.gate.attack_ms < 1.0);
        assert_eq!(clicks.processing.denoise.strength, 1.0);
        assert_eq!(clicks.processing.denoise.engine, Engine::Light);
        assert_eq!(discord.processing.denoise.engine, Engine::Strong, "clicks call for the strong engine");
        assert!(clicks.processing.comp.attack_ms < discord.processing.comp.attack_ms);
        assert!(clicks.processing.comp.ratio > discord.processing.comp.ratio);
        assert_eq!(clicks.processing.limiter.lookahead_ms, 0.0);
        assert!(clicks.processing.limiter.ceiling_db <= -3.0);
        assert!((-18.0..=-16.0).contains(&clicks.target_lufs));
    }

    #[test]
    fn meetings_stay_intelligible_and_unforced() {
        let quiet = tune(&sample(-72.0, -24.0, -20.0, -40.0), Target::Meeting, false);
        let office = tune(&sample(-45.0, -24.0, -20.0, -40.0), Target::Meeting, false);
        assert_eq!(quiet.target_lufs, -18.0);
        assert!((100.0..=120.0).contains(&quiet.eq[0].frequency));
        assert!(quiet.eq[6].gain_db > 0.0 && quiet.eq[6].gain_db <= 3.0);
        assert!(quiet.processing.denoise.strength >= 0.6 && office.processing.denoise.strength <= 0.85);
        assert!(office.processing.denoise.strength > quiet.processing.denoise.strength);
        assert_eq!(office.processing.denoise.engine, Engine::Strong);
        assert!(!quiet.gate_on && office.gate_on && office.processing.gate.hold_ms >= 250.0);
        assert!(office.gate_db <= -34.0, "the soft gate stays well under quiet speech");
        assert!(quiet.processing.comp.ratio <= 3.0 && quiet.processing.comp.attack_ms >= 10.0);
    }

    #[test]
    fn music_stays_natural() {
        let a = sample(-65.0, -24.0, -20.0, -40.0);
        let m = tune(&a, Target::Music, false);
        assert!(!m.processing.denoise.on && !m.gate_on && !m.processing.agc.on);
        assert!((30.0..=50.0).contains(&m.eq[0].frequency));
        assert!(m.eq[1..].iter().all(|b| b.gain_db == 0.0 && b.filter == FilterType::PeakingEQ), "flat");
        assert!(m.processing.comp.ratio < 2.0 && m.processing.comp.attack_ms >= 20.0);
        assert_eq!((m.target_lufs, m.processing.limiter.ceiling_db), (-14.0, -1.0));
        assert!((m.processing.comp.makeup_db - (-14.0 - a.lufs)).abs() < 0.1);
        assert!(!m.processing.deesser.on);
        let mut hiss = a.clone();
        hiss.sibilance_peak_db = -2.0;
        let h = tune(&hiss, Target::Music, false);
        assert!(h.processing.deesser.on && h.processing.deesser.amount_db <= 4.5, "light de-essing");
        let mut loud = a.clone();
        loud.noise_floor_dbfs = -45.0;
        assert!(tune(&loud, Target::Music, false).gate_on, "a soft expander over a loud floor");
        let mut boomy = a;
        for b in boomy.bands.iter_mut().filter(|b| b.hz < 400.0) {
            b.db += 10.0;
        }
        assert!(tune(&boomy, Target::Music, false).eq[3].gain_db < 0.0, "clear problems still get fixed");
    }

    #[test]
    fn a_bypassed_chain_changes_nothing() {
        let x = sine(500.0, 0.2, 0.5);
        assert_eq!(run_chain(&x, RATE, &Processing::default(), None, None), x);
    }
}
