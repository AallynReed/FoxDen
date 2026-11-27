//! Measures a mono recording: levels, loudness, noise floor, clipping, spectrum, sibilance and rumble.

use serde::Serialize;

use crate::dsp::{Band, Equalizer, FilterType};

const SILENT: f32 = -120.0;
const CLIP: f32 = 0.999;
const BLOCK_MS: f32 = 50.0;
/// Third-octave centres; those above 45% of the rate are left out.
const THIRDS: [f32; 28] = [
    25.0, 31.5, 40.0, 50.0, 63.0, 80.0, 100.0, 125.0, 160.0, 200.0, 250.0, 315.0, 400.0, 500.0,
    630.0, 800.0, 1000.0, 1250.0, 1600.0, 2000.0, 2500.0, 3150.0, 4000.0, 5000.0, 6300.0, 8000.0,
    10000.0, 12500.0,
];
const THIRD_Q: f32 = 4.32;
/// Peak over RMS of a 50 ms block above which it is a click, not speech, dB.
const SPEECH_CREST: f32 = 20.0;
/// The two stages of a fourth-order Butterworth filter.
const BW4: (f32, f32) = (0.5412, 1.3066);

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct BandLevel {
    pub hz: f32,
    pub db: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Analysis {
    pub seconds: f32,
    pub peak_dbfs: f32,
    pub rms_dbfs: f32,
    /// Integrated loudness after BS.1770 K-weighting and gating, LUFS.
    pub lufs: f32,
    /// RMS of the 50 ms blocks with speech in them.
    pub speech_dbfs: f32,
    /// The quietest tenth of the 50 ms blocks.
    pub noise_floor_dbfs: f32,
    pub snr_db: f32,
    /// Share of the blocks with speech in them, percent.
    pub speech_percent: f32,
    pub clipped_samples: usize,
    pub clipped_percent: f32,
    /// Third-octave levels, dBFS RMS.
    pub bands: Vec<BandLevel>,
    /// 4 to 10 kHz next to the whole signal, dB.
    pub sibilance_db: f32,
    /// The loudest tenth of 10 ms moments of 4 to 10 kHz next to the whole signal, dB.
    pub sibilance_peak_db: f32,
    /// The loudest third-octave between 4 and 10 kHz.
    pub sibilance_hz: f32,
    /// Below 80 Hz next to the whole signal, dB.
    pub rumble_db: f32,
    /// Bursts where sound below 150 Hz takes over.
    pub plosives: usize,
    /// Clicks and knocks between words: 10 ms spikes 20 dB over the floor, per second of pause.
    pub clicks_per_second: f32,
    /// The loudest 2% of the 10 ms peaks in the pauses.
    pub click_peak_dbfs: f32,
    /// Peak over RMS of the 10 ms moments in the pauses, their loudest 2%, dB.
    pub transient_crest_db: f32,
}

fn db(power: f64) -> f32 {
    if power > 1e-12 { (10.0 * power.log10()) as f32 } else { SILENT }
}

fn power(x: &[f32]) -> f64 {
    if x.is_empty() {
        return 0.0;
    }
    x.iter().map(|v| f64::from(*v) * f64::from(*v)).sum::<f64>() / x.len() as f64
}

fn filtered(x: &[f32], rate: f32, bands: &[Band]) -> Vec<f32> {
    let mut y = x.to_vec();
    Equalizer::new(bands, rate, 1).process(&mut y);
    y
}

fn band(filter: FilterType, frequency: f32, q: f32, gain_db: f32) -> Band {
    Band { enabled: true, filter, frequency, gain_db, q }
}

/// BS.1770 integrated loudness: 400 ms blocks, 75% overlap, gated at -70 LUFS and 10 LU under.
pub fn loudness(x: &[f32], rate: f32) -> f32 {
    let k = filtered(
        x,
        rate,
        &[band(FilterType::HighShelving, 1681.97, 0.7072, 4.0), band(FilterType::HighPass, 38.14, 0.5003, 0.0)],
    );
    let size = ((rate * 0.4) as usize).max(1);
    let step = (size / 4).max(1);
    let mut blocks: Vec<f64> = if k.len() <= size {
        vec![power(&k)]
    } else {
        (0..=(k.len() - size) / step).map(|i| power(&k[i * step..i * step + size])).collect()
    };
    let lufs = |p: f64| -0.691 + 10.0 * p.max(1e-12).log10();
    blocks.retain(|p| lufs(*p) > -70.0);
    if blocks.is_empty() {
        return SILENT;
    }
    let mean = blocks.iter().sum::<f64>() / blocks.len() as f64;
    let gate = lufs(mean) - 10.0;
    let kept: Vec<f64> = blocks.into_iter().filter(|p| lufs(*p) > gate).collect();
    lufs(kept.iter().sum::<f64>() / kept.len().max(1) as f64) as f32
}

fn blocks(x: &[f32], n: usize) -> Vec<f32> {
    x.chunks(n.max(1)).filter(|c| c.len() * 2 >= n).map(|c| db(power(c))).collect()
}

pub fn analyze(x: &[f32], rate: f32) -> Analysis {
    let total = power(x);
    let peak = x.iter().fold(0f32, |m, v| m.max(v.abs()));
    let clipped = x.iter().filter(|v| v.abs() >= CLIP).count();
    let n = (rate * BLOCK_MS / 1000.0) as usize;
    let mut levels = blocks(x, n);
    levels.sort_by(f32::total_cmp);
    let floor = levels.get(levels.len() / 10).copied().unwrap_or(SILENT);
    let loudest = levels.last().copied().unwrap_or(SILENT);
    // Speech is what stands well clear of the floor, and not far under the loudest moments.
    let cut = (floor + 10.0).max(loudest - 30.0);
    // A click is loud for a moment of its block; speech fills the block.
    let talking: Vec<bool> = x
        .chunks(n.max(1))
        .map(|c| {
            let level = db(power(c));
            let p = c.iter().fold(0f32, |m, v| m.max(v.abs()));
            let crest = if p > 0.0 { 20.0 * p.log10() - level } else { 0.0 };
            c.len() * 2 >= n && level >= cut && crest < SPEECH_CREST
        })
        .collect();
    let speech: Vec<f32> = x
        .chunks(n.max(1))
        .zip(&talking)
        .filter(|(_, t)| **t)
        .flat_map(|(c, _)| c.iter().copied())
        .collect();
    let speech_db = db(power(&speech));
    let speech_blocks = talking.iter().filter(|t| **t).count();
    let bands: Vec<BandLevel> = THIRDS
        .iter()
        .filter(|f| **f < rate * 0.45)
        .map(|&hz| BandLevel { hz, db: db(power(&filtered(x, rate, &[band(FilterType::BandPassPeak0dB, hz, THIRD_Q, 0.0)]))) })
        .collect();
    let hi = (10000f32).min(rate * 0.45);
    let sib = filtered(
        x,
        rate,
        &[band(FilterType::HighPass, 4000.0, BW4.0, 0.0), band(FilterType::HighPass, 4000.0, BW4.1, 0.0),
          band(FilterType::LowPass, hi, BW4.0, 0.0), band(FilterType::LowPass, hi, BW4.1, 0.0)],
    );
    let short = (rate * 0.01) as usize;
    let mut sib_moments: Vec<f32> = x
        .chunks(short.max(1))
        .zip(sib.chunks(short.max(1)))
        .filter(|(a, _)| db(power(a)) >= cut - 10.0)
        .map(|(a, s)| db(power(s)) - db(power(a)))
        .collect();
    sib_moments.sort_by(f32::total_cmp);
    let sib_peak = sib_moments.get(sib_moments.len() * 9 / 10).copied().unwrap_or(SILENT);
    let sib_hz = bands
        .iter()
        .filter(|b| (4000.0..=10000.0).contains(&b.hz))
        .max_by(|a, b| a.db.total_cmp(&b.db))
        .map_or(6300.0, |b| b.hz);
    let low = |hz: f32| {
        filtered(x, rate, &[band(FilterType::LowPass, hz, BW4.0, 0.0), band(FilterType::LowPass, hz, BW4.1, 0.0)])
    };
    let rumble = low(80.0);
    let thumps = low(150.0);
    let plosive_len = (rate * 0.02) as usize;
    let mut plosives = 0;
    let mut inside = false;
    for (a, l) in x.chunks(plosive_len.max(1)).zip(thumps.chunks(plosive_len.max(1))) {
        let (full, lf) = (db(power(a)), db(power(l)));
        let hit = full >= cut && lf > -40.0 && lf - full > -3.0;
        if hit && !inside {
            plosives += 1;
        }
        inside = hit;
    }
    let pause_chunks: Vec<&[f32]> = x
        .chunks(short.max(1))
        .enumerate()
        .filter(|(i, c)| c.len() * 2 >= short && !talking.get(i * short / n.max(1)).copied().unwrap_or(true))
        .map(|(_, c)| c)
        .collect();
    let peak_db = |c: &[f32]| {
        let p = c.iter().fold(0f32, |m, v| m.max(v.abs()));
        if p > 0.0 { 20.0 * p.log10() } else { SILENT }
    };
    let mut pause_peaks: Vec<f32> = pause_chunks.iter().map(|c| peak_db(c)).collect();
    let mut crests: Vec<f32> = pause_chunks.iter().map(|c| peak_db(c) - db(power(c))).collect();
    let mut clicks = 0;
    let mut inside = false;
    for p in &pause_peaks {
        let hit = *p >= floor + 20.0;
        if hit && !inside {
            clicks += 1;
        }
        inside = hit;
    }
    pause_peaks.sort_by(f32::total_cmp);
    crests.sort_by(f32::total_cmp);
    let top = |v: &[f32]| v.get(v.len() * 98 / 100).copied().unwrap_or(SILENT);
    let pause_secs = pause_chunks.len() as f32 * short as f32 / rate;
    let rel = |p: f64| if total > 1e-12 { db(p) - db(total) } else { SILENT };
    let blocks_n = levels.len().max(1);
    Analysis {
        seconds: x.len() as f32 / rate,
        peak_dbfs: if peak > 0.0 { 20.0 * peak.log10() } else { SILENT },
        rms_dbfs: db(total),
        lufs: loudness(x, rate),
        speech_dbfs: speech_db,
        noise_floor_dbfs: floor,
        snr_db: speech_db - floor,
        speech_percent: 100.0 * speech_blocks as f32 / blocks_n as f32,
        clipped_samples: clipped,
        clipped_percent: 100.0 * clipped as f32 / x.len().max(1) as f32,
        bands,
        sibilance_db: rel(power(&sib)),
        sibilance_peak_db: sib_peak,
        sibilance_hz: sib_hz,
        rumble_db: rel(power(&rumble)),
        plosives,
        clicks_per_second: if pause_secs > 0.0 { clicks as f32 / pause_secs } else { 0.0 },
        click_peak_dbfs: top(&pause_peaks),
        transient_crest_db: if crests.is_empty() { 0.0 } else { top(&crests) },
    }
}

/// How the processed recording differs from the raw one, in dB; negative is quieter.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Difference {
    pub peak_db: f32,
    pub lufs: f32,
    pub speech_db: f32,
    /// How far the gate and noise suppression pushed the floor down.
    pub noise_floor_db: f32,
    pub snr_db: f32,
    pub sibilance_db: f32,
    pub sibilance_peak_db: f32,
    pub rumble_db: f32,
    pub clipped_samples: i64,
    pub bands: Vec<BandLevel>,
}

pub fn compare(raw: &Analysis, processed: &Analysis) -> Difference {
    let d = |a: f32, b: f32| ((b - a) * 10.0).round() / 10.0;
    Difference {
        peak_db: d(raw.peak_dbfs, processed.peak_dbfs),
        lufs: d(raw.lufs, processed.lufs),
        speech_db: d(raw.speech_dbfs, processed.speech_dbfs),
        noise_floor_db: d(raw.noise_floor_dbfs, processed.noise_floor_dbfs),
        snr_db: d(raw.snr_db, processed.snr_db),
        sibilance_db: d(raw.sibilance_db, processed.sibilance_db),
        sibilance_peak_db: d(raw.sibilance_peak_db, processed.sibilance_peak_db),
        rumble_db: d(raw.rumble_db, processed.rumble_db),
        clipped_samples: processed.clipped_samples as i64 - raw.clipped_samples as i64,
        bands: raw
            .bands
            .iter()
            .zip(&processed.bands)
            .map(|(a, b)| BandLevel { hz: a.hz, db: d(a.db, b.db) })
            .collect(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub const RATE: f32 = 48000.0;

    pub fn sine(hz: f32, amp: f32, secs: f32) -> Vec<f32> {
        (0..(RATE * secs) as usize)
            .map(|i| amp * (2.0 * std::f32::consts::PI * hz * i as f32 / RATE).sin())
            .collect()
    }

    pub fn noise(amp: f32, n: usize, seed: u64) -> Vec<f32> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                amp * ((s % 20001) as f32 / 10000.0 - 1.0)
            })
            .collect()
    }

    /// A voice-like buzz on 140 Hz in syllables, peaking at `voice`.
    pub fn voice(secs: f32, voice: f32) -> Vec<f32> {
        let mut phase = 0f32;
        (0..(RATE * secs) as usize)
            .map(|i| {
                let t = i as f32 / RATE;
                let syllable = (t * std::f32::consts::PI * 4.0).sin().abs().powf(0.5);
                phase = (phase + 140.0 * (1.0 + 0.03 * (t * 5.0).sin()) / RATE).fract();
                let buzz: f32 = (1..16).map(|h| (phase * 2.0 * std::f32::consts::PI * h as f32).sin() / h as f32).sum();
                voice * 0.5 * syllable * buzz
            })
            .collect()
    }

    /// Talking with a quiet noise bed under it, half the time talking.
    pub fn talk(secs: f32, voice: f32, bed: f32) -> Vec<f32> {
        let v = self::voice(secs, voice);
        let n = noise(bed, v.len(), 7);
        v.iter()
            .zip(&n)
            .enumerate()
            .map(|(i, (a, b))| if (i / (RATE as usize / 4)).is_multiple_of(2) { a + b } else { *b })
            .collect()
    }

    fn near(a: f32, b: f32, by: f32) {
        assert!((a - b).abs() <= by, "{a} is not within {by} of {b}");
    }

    #[test]
    fn a_sine_measures_as_its_level() {
        let a = analyze(&sine(1000.0, 0.1, 2.0), RATE);
        near(a.peak_dbfs, -20.0, 0.05);
        near(a.rms_dbfs, -23.01, 0.05);
        near(a.lufs, -23.0, 0.3);
        assert_eq!(a.clipped_samples, 0);
        let top = a.bands.iter().max_by(|x, y| x.db.total_cmp(&y.db)).unwrap();
        assert_eq!(top.hz, 1000.0);
        near(top.db, -23.0, 0.5);
        assert!(a.sibilance_db < -30.0 && a.rumble_db < -30.0, "{a:?}");
    }

    #[test]
    fn clipping_noise_and_speech_are_found() {
        let square: Vec<f32> = sine(100.0, 3.0, 1.0).iter().map(|v| v.clamp(-1.0, 1.0)).collect();
        let a = analyze(&square, RATE);
        assert!(a.clipped_percent > 50.0, "{a:?}");
        near(a.peak_dbfs, 0.0, 0.01);

        let a = analyze(&talk(4.0, 0.2, 0.002), RATE);
        near(a.noise_floor_dbfs, -58.8, 1.5);
        near(a.speech_dbfs, -23.0, 1.5);
        near(a.speech_percent, 50.0, 5.0);
        assert!(a.snr_db > 35.0, "{a:?}");
    }

    #[test]
    fn sibilance_and_rumble_show_where_they_are() {
        let mut x = sine(300.0, 0.1, 2.0);
        for (v, s) in x.iter_mut().zip(sine(6300.0, 0.1, 2.0)) {
            *v += s;
        }
        let a = analyze(&x, RATE);
        near(a.sibilance_db, -3.0, 1.0);
        assert_eq!(a.sibilance_hz, 6300.0);
        assert!(a.sibilance_peak_db > -4.0);

        let mut x = sine(1000.0, 0.1, 2.0);
        for (v, s) in x.iter_mut().zip(sine(40.0, 0.1, 2.0)) {
            *v += s;
        }
        let a = analyze(&x, RATE);
        near(a.rumble_db, -3.0, 1.0);
    }

    #[test]
    fn plosives_are_counted_as_bursts() {
        let mut x = sine(1000.0, 0.05, 3.0);
        for start in [0.5f32, 1.5, 2.5] {
            let at = (start * RATE) as usize;
            for (i, s) in sine(60.0, 0.5, 0.06).iter().enumerate() {
                x[at + i] += s;
            }
        }
        assert_eq!(analyze(&x, RATE).plosives, 3);
    }

    #[test]
    fn clicks_between_words_are_told_from_speech() {
        let mut x = talk(4.0, 0.2, 0.001);
        let quiet = analyze(&x, RATE);
        for k in 0..8 {
            let at = (RATE as usize / 4) * (2 * k + 1) + 2000;
            for (i, v) in [0.3f32, -0.25, 0.2, -0.1].iter().enumerate() {
                x[at + i] += v;
            }
        }
        let a = analyze(&x, RATE);
        assert!(quiet.clicks_per_second < 0.5 && quiet.transient_crest_db < 8.0, "{quiet:?}");
        assert!((3.0..6.0).contains(&a.clicks_per_second), "{a:?}");
        assert!(a.click_peak_dbfs > -15.0 && a.transient_crest_db > 15.0, "{a:?}");
        assert!((a.speech_dbfs - quiet.speech_dbfs).abs() < 0.5);
    }

    #[test]
    fn differences_are_processed_minus_raw() {
        let raw = analyze(&sine(1000.0, 0.1, 1.0), RATE);
        let half = analyze(&sine(1000.0, 0.05, 1.0), RATE);
        let d = compare(&raw, &half);
        near(d.peak_db, -6.0, 0.1);
        near(d.lufs, -6.0, 0.1);
        assert_eq!(d.bands.len(), raw.bands.len());
    }
}
