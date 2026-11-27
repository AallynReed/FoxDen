//! Dynamics: compressor, lookahead limiter, level stabilizer and de-esser. Interleaved,
//! stereo-linked, allocation-free once built.

use serde::{Deserialize, Serialize};

const SILENT_DB: f32 = -120.0;
const TINY: f32 = 1e-20;
const LIMIT_LOOKAHEAD_MS: f32 = 2.0;
const AGC_WINDOW_MS: f32 = 400.0;
const AGC_STEP: u32 = 32;
const DEESS_Q: f32 = 1.4;
const DEESS_REL_DB: f32 = -14.0;

#[inline]
fn to_db(x: f32) -> f32 {
    if x > 1e-6 {
        20.0 * x.log10()
    } else {
        SILENT_DB
    }
}

#[inline]
fn from_db(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// One-pole coefficient reaching 63% in `ms`.
fn coef(rate: f32, ms: f32) -> f32 {
    1.0 - (-1.0 / (rate * ms.max(0.01) / 1000.0)).exp()
}

/// NaN and infinity become silence; returns the frame's peak.
#[inline]
fn sanitize(frame: &mut [f32]) -> f32 {
    let mut peak = 0f32;
    for s in frame.iter_mut() {
        if !s.is_finite() {
            *s = 0.0;
        }
        peak = peak.max(s.abs());
    }
    peak
}

fn num(v: f32, lo: f32, hi: f32, or: f32) -> f32 {
    if v.is_finite() { v.clamp(lo, hi) } else { or }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CompSettings {
    pub on: bool,
    pub threshold_db: f32,
    pub ratio: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub knee_db: f32,
    pub makeup_db: f32,
    pub auto_makeup: bool,
}

impl Default for CompSettings {
    fn default() -> Self {
        Self {
            on: false,
            threshold_db: -18.0,
            ratio: 3.0,
            attack_ms: 5.0,
            release_ms: 120.0,
            knee_db: 6.0,
            makeup_db: 0.0,
            auto_makeup: false,
        }
    }
}

impl CompSettings {
    pub fn clean(self) -> Self {
        let d = Self::default();
        Self {
            on: self.on,
            threshold_db: num(self.threshold_db, -60.0, 0.0, d.threshold_db),
            ratio: num(self.ratio, 1.0, 20.0, d.ratio),
            attack_ms: num(self.attack_ms, 0.1, 200.0, d.attack_ms),
            release_ms: num(self.release_ms, 10.0, 2000.0, d.release_ms),
            knee_db: num(self.knee_db, 0.0, 24.0, d.knee_db),
            makeup_db: num(self.makeup_db, 0.0, 24.0, d.makeup_db),
            auto_makeup: self.auto_makeup,
        }
    }

    /// Makeup gain in use: the set one, or half the reduction a full-scale signal gets.
    pub fn makeup(&self) -> f32 {
        if self.auto_makeup {
            -self.threshold_db * (1.0 - 1.0 / self.ratio.max(1.0)) / 2.0
        } else {
            self.makeup_db
        }
    }
}

/// Feed-forward compressor with a soft knee, smoothed in dB.
#[derive(Debug, Clone)]
pub struct Compressor {
    channels: usize,
    rate: f32,
    threshold: f32,
    slope: f32,
    knee: f32,
    makeup: f32,
    attack: f32,
    release: f32,
    hold: f32,
    env: f32,
    gr: f32,
    meter: f32,
}

impl Compressor {
    pub fn new(rate: f32, channels: usize, s: &CompSettings) -> Self {
        let mut c = Self {
            channels: channels.max(1),
            rate,
            threshold: 0.0,
            slope: 0.0,
            knee: 0.0,
            makeup: 0.0,
            attack: 1.0,
            release: 1.0,
            hold: coef(rate, 10.0),
            env: 0.0,
            gr: 0.0,
            meter: 0.0,
        };
        c.set(s);
        c
    }

    pub fn set(&mut self, s: &CompSettings) {
        let s = s.clean();
        self.threshold = s.threshold_db;
        self.slope = 1.0 - 1.0 / s.ratio;
        self.knee = s.knee_db;
        self.makeup = s.makeup();
        self.attack = coef(self.rate, s.attack_ms);
        self.release = coef(self.rate, s.release_ms);
    }

    /// Static reduction in dB for a level in dB.
    pub fn curve(&self, db: f32) -> f32 {
        let over = db - self.threshold;
        if self.knee > 0.0 && 2.0 * over.abs() <= self.knee {
            let k = over + self.knee / 2.0;
            self.slope * k * k / (2.0 * self.knee)
        } else if over > 0.0 {
            self.slope * over
        } else {
            0.0
        }
    }

    pub fn process(&mut self, samples: &mut [f32]) {
        let flat = from_db(self.makeup);
        for frame in samples.chunks_exact_mut(self.channels) {
            let peak = sanitize(frame);
            self.env = if peak > self.env {
                peak
            } else {
                let e = self.env + (peak - self.env) * self.hold;
                if e < TINY { 0.0 } else { e }
            };
            let target = self.curve(to_db(self.env));
            let k = if target > self.gr {
                self.attack
            } else {
                self.release
            };
            self.gr += (target - self.gr) * k;
            if self.gr < 1e-4 {
                self.gr = 0.0;
            }
            let g = if self.gr == 0.0 {
                flat
            } else {
                from_db(self.makeup - self.gr)
            };
            for s in frame.iter_mut() {
                *s *= g;
            }
            self.meter = self.meter.max(self.gr);
        }
    }

    /// Forgets the level, as when it is switched on again.
    pub fn reset(&mut self) {
        (self.env, self.gr, self.meter) = (0.0, 0.0, 0.0);
    }

    /// Most gain reduction in dB since the last call.
    pub fn take_meter(&mut self) -> f32 {
        std::mem::take(&mut self.meter)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LimiterSettings {
    pub on: bool,
    pub ceiling_db: f32,
    pub release_ms: f32,
    /// How far ahead it looks, ms; 0 is a single sample.
    pub lookahead_ms: f32,
}

impl Default for LimiterSettings {
    fn default() -> Self {
        Self {
            on: false,
            ceiling_db: -1.0,
            release_ms: 80.0,
            lookahead_ms: LIMIT_LOOKAHEAD_MS,
        }
    }
}

impl LimiterSettings {
    pub fn clean(self) -> Self {
        let d = Self::default();
        Self {
            on: self.on,
            ceiling_db: num(self.ceiling_db, -24.0, 0.0, d.ceiling_db),
            release_ms: num(self.release_ms, 5.0, 2000.0, d.release_ms),
            lookahead_ms: num(self.lookahead_ms, 0.0, 5.0, d.lookahead_ms),
        }
    }
}

fn lag_of(rate: f32, s: &LimiterSettings) -> usize {
    (rate * s.clean().lookahead_ms / 1000.0).round().max(1.0) as usize
}

/// Brickwall limiter: the gain needed for each peak is held across the lookahead and smoothed
/// with a moving average of the same length, so it is fully down when the peak comes out.
#[derive(Debug, Clone)]
pub struct Limiter {
    channels: usize,
    rate: f32,
    ceiling: f32,
    release: f32,
    delay: Vec<f32>,
    pos: usize,
    window: usize,
    /// Sliding minimum: values and times, a ring of `window` slots.
    min_v: Vec<f32>,
    min_t: Vec<u64>,
    min_head: usize,
    min_len: usize,
    held: Vec<f32>,
    held_pos: usize,
    sum: f64,
    t: u64,
    gain: f32,
    meter: f32,
}

impl Limiter {
    pub fn new(rate: f32, channels: usize, s: &LimiterSettings) -> Self {
        let channels = channels.max(1);
        let lag = lag_of(rate, s);
        let window = lag + 1;
        let mut l = Self {
            channels,
            rate,
            ceiling: 1.0,
            release: 1.0,
            delay: vec![0.0; lag * channels],
            pos: 0,
            window,
            min_v: vec![1.0; window],
            min_t: vec![0; window],
            min_head: 0,
            min_len: 0,
            held: vec![1.0; window],
            held_pos: 0,
            sum: window as f64,
            t: 0,
            gain: 1.0,
            meter: 1.0,
        };
        l.set(s);
        l
    }

    pub fn set(&mut self, s: &LimiterSettings) {
        let s = s.clean();
        if lag_of(self.rate, &s) != self.window - 1 {
            *self = Limiter::new(self.rate, self.channels, &s);
            return;
        }
        self.ceiling = from_db(s.ceiling_db);
        self.release = coef(self.rate, s.release_ms);
    }

    /// Empties the lookahead, so switching it on again plays no old sound.
    pub fn reset(&mut self) {
        self.delay.fill(0.0);
        self.held.fill(1.0);
        self.sum = self.window as f64;
        self.min_len = 0;
        self.gain = 1.0;
        self.meter = 1.0;
    }

    /// Samples the output lags the input by.
    pub fn latency(&self) -> usize {
        self.window - 1
    }

    fn min_push(&mut self, v: f32) -> f32 {
        let w = self.window;
        while self.min_len > 0 {
            let back = (self.min_head + self.min_len - 1) % w;
            if self.min_v[back] >= v {
                self.min_len -= 1;
            } else {
                break;
            }
        }
        let at = (self.min_head + self.min_len) % w;
        self.min_v[at] = v;
        self.min_t[at] = self.t;
        self.min_len += 1;
        while self.min_t[self.min_head] + w as u64 <= self.t {
            self.min_head = (self.min_head + 1) % w;
            self.min_len -= 1;
        }
        self.min_v[self.min_head]
    }

    pub fn process(&mut self, samples: &mut [f32]) {
        let ch = self.channels;
        let lag = self.window - 1;
        for frame in samples.chunks_exact_mut(ch) {
            let peak = sanitize(frame);
            let need = if peak > self.ceiling {
                self.ceiling / peak
            } else {
                1.0
            };
            let hold = self.min_push(need);
            self.sum += f64::from(hold) - f64::from(self.held[self.held_pos]);
            self.held[self.held_pos] = hold;
            self.held_pos = (self.held_pos + 1) % self.window;
            self.t += 1;
            let smooth = (self.sum / self.window as f64) as f32;
            if smooth < self.gain {
                self.gain = smooth;
            } else {
                self.gain += (smooth - self.gain) * self.release;
            }
            let g = self.gain;
            self.meter = self.meter.min(g);
            let at = self.pos * ch;
            for (c, s) in frame.iter_mut().enumerate() {
                let out = self.delay[at + c];
                self.delay[at + c] = *s;
                *s = (out * g).clamp(-self.ceiling, self.ceiling);
            }
            self.pos = (self.pos + 1) % lag;
        }
        // Keeps the running sum exact over long runs.
        if self.t.is_multiple_of(1 << 16) {
            self.sum = self.held.iter().map(|&v| f64::from(v)).sum();
        }
    }

    pub fn take_meter(&mut self) -> f32 {
        let g = std::mem::replace(&mut self.meter, 1.0);
        -to_db(g).min(0.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgcSettings {
    pub on: bool,
    /// RMS level to aim for, dBFS.
    pub target_db: f32,
    pub max_gain_db: f32,
    pub max_cut_db: f32,
    /// How fast the gain moves, dB per second.
    pub speed_db: f32,
    /// Below this the gain stays where it is, dBFS.
    pub floor_db: f32,
}

impl Default for AgcSettings {
    fn default() -> Self {
        Self {
            on: false,
            target_db: -20.0,
            max_gain_db: 12.0,
            max_cut_db: 12.0,
            speed_db: 6.0,
            floor_db: -50.0,
        }
    }
}

impl AgcSettings {
    pub fn clean(self) -> Self {
        let d = Self::default();
        Self {
            on: self.on,
            target_db: num(self.target_db, -40.0, -6.0, d.target_db),
            max_gain_db: num(self.max_gain_db, 0.0, 30.0, d.max_gain_db),
            max_cut_db: num(self.max_cut_db, 0.0, 30.0, d.max_cut_db),
            speed_db: num(self.speed_db, 0.5, 60.0, d.speed_db),
            floor_db: num(self.floor_db, -80.0, -20.0, d.floor_db),
        }
    }
}

/// Volume stabilizer: moves a slow gain towards the target RMS, measured only while there is
/// sound above the floor, so silence and a closed gate leave the gain alone.
#[derive(Debug, Clone)]
pub struct Agc {
    channels: usize,
    rate: f32,
    s: AgcSettings,
    floor: f32,
    rms_coef: f32,
    fast_rel: f32,
    mean_sq: f32,
    fast: f32,
    gain_db: f32,
    target_lin: f32,
    gain: f32,
    step: u32,
}

impl Agc {
    pub fn new(rate: f32, channels: usize, s: &AgcSettings) -> Self {
        let mut a = Self {
            channels: channels.max(1),
            rate,
            s: *s,
            floor: 0.0,
            rms_coef: coef(rate, AGC_WINDOW_MS),
            fast_rel: coef(rate, 10.0),
            mean_sq: 0.0,
            fast: 0.0,
            gain_db: 0.0,
            target_lin: 1.0,
            gain: 1.0,
            step: 0,
        };
        a.set(s);
        a.mean_sq = from_db(a.s.target_db).powi(2);
        a
    }

    pub fn set(&mut self, s: &AgcSettings) {
        self.s = s.clean();
        self.floor = from_db(self.s.floor_db).powi(2);
        self.gain_db = self.gain_db.clamp(-self.s.max_cut_db, self.s.max_gain_db);
        self.target_lin = from_db(self.gain_db);
    }

    /// `hold` freezes the gain, as while the gate is closed.
    pub fn process(&mut self, samples: &mut [f32], hold: bool) {
        let ch = self.channels;
        let smooth = 1.0 / AGC_STEP as f32;
        for frame in samples.chunks_exact_mut(ch) {
            sanitize(frame);
            let sq = frame.iter().map(|s| s * s).sum::<f32>() / ch as f32;
            self.fast += (sq - self.fast) * self.fast_rel;
            if self.fast < TINY {
                self.fast = 0.0;
            }
            if !hold && self.fast > self.floor {
                self.mean_sq += (sq - self.mean_sq) * self.rms_coef;
                self.step += 1;
                if self.step >= AGC_STEP {
                    self.step = 0;
                    let level = 10.0 * self.mean_sq.max(1e-12).log10();
                    let want =
                        (self.s.target_db - level).clamp(-self.s.max_cut_db, self.s.max_gain_db);
                    let most = self.s.speed_db * AGC_STEP as f32 / self.rate;
                    self.gain_db += (want - self.gain_db).clamp(-most, most);
                    self.target_lin = from_db(self.gain_db);
                }
            }
            self.gain += (self.target_lin - self.gain) * smooth;
            for s in frame.iter_mut() {
                *s *= self.gain;
            }
        }
    }

    /// Gain applied now in dB; positive is a boost.
    pub fn gain_db(&self) -> f32 {
        self.gain_db
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeEsserSettings {
    pub on: bool,
    pub frequency: f32,
    /// Most reduction of the band, dB.
    pub amount_db: f32,
    /// Band level next to the whole signal where reduction starts, dB.
    pub threshold_db: f32,
}

impl Default for DeEsserSettings {
    fn default() -> Self {
        Self {
            on: false,
            frequency: 6500.0,
            amount_db: 6.0,
            threshold_db: DEESS_REL_DB,
        }
    }
}

impl DeEsserSettings {
    pub fn clean(self) -> Self {
        let d = Self::default();
        Self {
            on: self.on,
            frequency: num(self.frequency, 3000.0, 10000.0, d.frequency),
            amount_db: num(self.amount_db, 0.0, 24.0, d.amount_db),
            threshold_db: num(self.threshold_db, -30.0, 0.0, d.threshold_db),
        }
    }
}

/// Turns down a band around the sibilance frequency when it gets loud next to the whole signal:
/// output is the input minus part of the band-passed signal, a notch at full reduction.
#[derive(Debug, Clone)]
pub struct DeEsser {
    channels: usize,
    rate: f32,
    amount: f32,
    threshold: f32,
    b: [f32; 3],
    a: [f32; 2],
    state: Vec<[f32; 4]>,
    band_env: f32,
    full_env: f32,
    attack: f32,
    release: f32,
    meter: f32,
}

impl DeEsser {
    pub fn new(rate: f32, channels: usize, s: &DeEsserSettings) -> Self {
        let channels = channels.max(1);
        let mut d = Self {
            channels,
            rate,
            amount: 0.0,
            threshold: DEESS_REL_DB,
            b: [0.0; 3],
            a: [0.0; 2],
            state: vec![[0.0; 4]; channels],
            band_env: 0.0,
            full_env: 0.0,
            attack: coef(rate, 1.0),
            release: coef(rate, 60.0),
            meter: 0.0,
        };
        d.set(s);
        d
    }

    pub fn set(&mut self, s: &DeEsserSettings) {
        let s = s.clean();
        self.amount = s.amount_db;
        self.threshold = s.threshold_db;
        let f = s.frequency.min(self.rate * 0.45);
        let w0 = 2.0 * std::f32::consts::PI * f / self.rate;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * DEESS_Q);
        let a0 = 1.0 + alpha;
        self.b = [alpha / a0, 0.0, -alpha / a0];
        self.a = [-2.0 * cos / a0, (1.0 - alpha) / a0];
    }

    fn env(env: &mut f32, x: f32, att: f32, rel: f32) {
        let k = if x > *env { att } else { rel };
        *env += (x - *env) * k;
        if *env < TINY {
            *env = 0.0;
        }
    }

    pub fn process(&mut self, samples: &mut [f32]) {
        let ch = self.channels;
        let ([b0, b1, b2], [a1, a2]) = (self.b, self.a);
        let floor = from_db(-60.0);
        for frame in samples.chunks_exact_mut(ch) {
            let full = sanitize(frame);
            let mut band = 0f32;
            for (s, st) in frame.iter_mut().zip(self.state.iter_mut()) {
                let x = *s;
                let mut y = b0 * x + b1 * st[0] + b2 * st[1] - a1 * st[2] - a2 * st[3];
                if y.abs() < TINY {
                    y = 0.0;
                }
                (st[1], st[0], st[3], st[2]) = (st[0], x, st[2], y);
                band = band.max(y.abs());
                *s = y;
            }
            Self::env(&mut self.band_env, band, self.attack, self.release);
            Self::env(&mut self.full_env, full, self.attack, self.release);
            let gr = if self.band_env > floor && self.full_env > 0.0 {
                let rel = to_db(self.band_env) - to_db(self.full_env);
                ((rel - self.threshold) * 1.5).clamp(0.0, self.amount)
            } else {
                0.0
            };
            let cut = 1.0 - from_db(-gr);
            for (s, st) in frame.iter_mut().zip(self.state.iter()) {
                // st[0] is this frame's input, `s` holds the band.
                *s = st[0] - cut * *s;
            }
            self.meter = self.meter.max(gr);
        }
    }

    pub fn take_meter(&mut self) -> f32 {
        std::mem::take(&mut self.meter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 48000.0;

    fn sine(freq: f32, db: f32, secs: f32) -> Vec<f32> {
        let n = (RATE * secs) as usize;
        let a = from_db(db);
        (0..n)
            .flat_map(|i| {
                let v = a * (2.0 * std::f32::consts::PI * freq * i as f32 / RATE).sin();
                [v, v]
            })
            .collect()
    }

    fn peak(s: &[f32]) -> f32 {
        s.iter().fold(0f32, |m, x| m.max(x.abs()))
    }

    fn rms_db(s: &[f32]) -> f32 {
        10.0 * (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).log10()
    }

    /// Largest jump between neighbouring samples of one channel.
    fn biggest_step(s: &[f32]) -> f32 {
        s.as_chunks::<2>().0.iter()
            .zip(s.as_chunks::<2>().0.iter().skip(1))
            .fold(0f32, |m, (a, b)| m.max((a[0] - b[0]).abs()))
    }

    #[test]
    fn compressor_reduces_by_the_ratio() {
        let s = CompSettings {
            on: true,
            threshold_db: -20.0,
            ratio: 4.0,
            knee_db: 0.0,
            ..Default::default()
        };
        let mut c = Compressor::new(RATE, 2, &s);
        let mut x = sine(1000.0, -4.0, 1.0);
        c.process(&mut x);
        let gr = -to_db(peak(&x[x.len() / 2..]) / from_db(-4.0));
        assert!(
            (gr - 12.0).abs() < 0.6,
            "16 dB over at 4:1 is 12 dB down, got {gr}"
        );
        assert!(c.take_meter() > 11.0);
        let mut quiet = sine(1000.0, -30.0, 0.5);
        let mut c = Compressor::new(RATE, 2, &s);
        c.process(&mut quiet);
        assert!(
            (peak(&quiet) - from_db(-30.0)).abs() < 1e-4,
            "untouched under the threshold"
        );
    }

    #[test]
    fn compressor_attack_and_release_times() {
        let s = CompSettings {
            on: true,
            threshold_db: -20.0,
            ratio: 10.0,
            attack_ms: 10.0,
            release_ms: 100.0,
            knee_db: 0.0,
            ..Default::default()
        };
        let mut c = Compressor::new(RATE, 1, &s);
        let mut gr = Vec::new();
        for i in 0..(RATE as usize) {
            let mut x = [if i < 24000 { 1.0 } else { 0.01 }];
            c.process(&mut x);
            gr.push(c.gr);
        }
        let full = gr[23999];
        let t63 = gr.iter().position(|&g| g >= full * 0.632).unwrap() as f32 / RATE * 1000.0;
        assert!((t63 - 10.0).abs() < 1.0, "attack {t63} ms");
        let r = gr[24000..].iter().position(|&g| g <= full * 0.368).unwrap() as f32 / RATE * 1000.0;
        assert!((100.0..125.0).contains(&r), "release {r} ms");
    }

    #[test]
    fn soft_knee_is_continuous_and_auto_makeup_lifts() {
        let s = CompSettings {
            threshold_db: -20.0,
            ratio: 4.0,
            knee_db: 10.0,
            auto_makeup: true,
            ..Default::default()
        };
        let c = Compressor::new(RATE, 2, &s);
        let mut last = 0.0;
        for i in 0..400 {
            let db = -40.0 + i as f32 * 0.1;
            let g = c.curve(db);
            assert!(g >= last && g - last < 0.1);
            last = g;
        }
        assert!((c.curve(-25.0)).abs() < 1e-6 && (c.curve(-10.0) - 7.5).abs() < 1e-4);
        assert!((s.makeup() - 7.5).abs() < 1e-4);
    }

    #[test]
    fn limiter_never_passes_the_ceiling() {
        let s = LimiterSettings {
            on: true,
            ceiling_db: -3.0,
            release_ms: 50.0,
            ..Default::default()
        };
        let mut l = Limiter::new(RATE, 2, &s);
        let ceiling = from_db(-3.0);
        // Quiet with sudden full-scale spikes and a loud burst.
        let mut x: Vec<f32> = (0..96000)
            .flat_map(|i| {
                let base = 0.1 * (i as f32 * 0.05).sin();
                let v = if i % 7919 == 7918 {
                    1.5
                } else if (40000..44000).contains(&i) {
                    2.0 * (i as f32 * 0.3).sin()
                } else {
                    base
                };
                [v, -v]
            })
            .collect();
        let input = x.clone();
        l.process(&mut x);
        assert!(peak(&x) <= ceiling + 1e-6, "{}", peak(&x));
        assert!(l.take_meter() > 9.0);
        let lag = l.latency() * 2;
        assert_eq!(l.latency(), 96);
        assert!(
            (x[1000 + lag] - input[1000]).abs() < 1e-6,
            "quiet part passes as it was"
        );
    }

    #[test]
    fn a_limiter_without_lookahead_still_holds_the_ceiling() {
        let s = LimiterSettings { on: true, ceiling_db: -4.0, lookahead_ms: 0.0, ..Default::default() };
        let mut l = Limiter::new(RATE, 1, &s);
        assert_eq!(l.latency(), 1);
        let mut x: Vec<f32> = (0..48000).map(|i| if i % 4801 == 4800 { 1.8 } else { 0.1 * (i as f32 * 0.05).sin() }).collect();
        l.process(&mut x);
        assert!(peak(&x) <= from_db(-4.0) + 1e-6, "{}", peak(&x));
        l.set(&LimiterSettings { lookahead_ms: 2.0, ..s });
        assert_eq!(l.latency(), 96);
    }

    #[test]
    fn the_lookahead_does_the_limiting_not_the_clamp() {
        let mut l = Limiter::new(
            RATE,
            2,
            &LimiterSettings {
                on: true,
                ..Default::default()
            },
        );
        let mut x = sine(1000.0, 6.0, 0.5);
        l.process(&mut x);
        let ceiling = from_db(-1.0);
        let flat = x
            .iter()
            .filter(|v| (v.abs() - ceiling).abs() < 1e-5)
            .count();
        assert!(
            flat < x.len() / 10,
            "{flat} of {} samples sit on the ceiling",
            x.len()
        );
    }

    #[test]
    fn a_reset_limiter_plays_no_old_sound() {
        let mut l = Limiter::new(
            RATE,
            2,
            &LimiterSettings {
                on: true,
                ..Default::default()
            },
        );
        let mut loud = sine(1000.0, 0.0, 0.1);
        l.process(&mut loud);
        l.reset();
        let mut quiet = vec![0.0f32; 960];
        l.process(&mut quiet);
        assert!(quiet.iter().all(|v| *v == 0.0));
        let mut x = sine(1000.0, -20.0, 0.1);
        let dry = x.clone();
        l.process(&mut x);
        let lag = l.latency() * 2;
        assert!(
            (x[lag + 10] - dry[10]).abs() < 1e-6,
            "gain is back at unity"
        );
    }

    #[test]
    fn limiter_gain_moves_without_clicks() {
        let mut l = Limiter::new(
            RATE,
            2,
            &LimiterSettings {
                on: true,
                ..Default::default()
            },
        );
        let mut x = sine(100.0, 0.0, 0.5);
        x.extend(sine(100.0, -20.0, 0.5));
        l.process(&mut x);
        let step = biggest_step(&x);
        // A 100 Hz full-scale sine moves at most 0.013 per sample.
        assert!(step < 0.02, "{step}");
    }

    #[test]
    fn agc_moves_to_target_and_holds_in_silence() {
        let s = AgcSettings {
            on: true,
            target_db: -20.0,
            max_gain_db: 15.0,
            speed_db: 20.0,
            ..Default::default()
        };
        let mut a = Agc::new(RATE, 2, &s);
        let mut x = sine(300.0, -30.0, 4.0);
        a.process(&mut x, false);
        let out = rms_db(&x[x.len() - 9600..]);
        assert!((out + 20.0).abs() < 1.0, "reaches the target, {out}");
        let before = a.gain_db();
        let mut silence = vec![0.0f32; 9600];
        a.process(&mut silence, false);
        let held = a.gain_db();
        assert!((held - before).abs() < 0.5, "speech tail barely moves it");
        let mut silence = vec![0.0f32; 480000];
        a.process(&mut silence, false);
        let mut hiss = sine(5000.0, -65.0, 2.0);
        a.process(&mut hiss, false);
        assert!((a.gain_db() - held).abs() < 1e-3, "no pumping on silence");
        let mut loud = sine(300.0, -10.0, 0.1);
        a.process(&mut loud, true);
        assert!((a.gain_db() - held).abs() < 1e-3, "frozen while gated");
        let dry = sine(300.0, -3.0, 4.0);
        let mut loud = dry.clone();
        a.process(&mut loud, false);
        assert!(
            a.gain_db() >= -12.0 - 1e-3 && a.gain_db() < -4.0,
            "cuts, within max cut"
        );
        let gains: Vec<f32> = loud
            .iter()
            .zip(&dry)
            .step_by(2)
            .filter(|(_, d)| d.abs() > 0.1)
            .map(|(o, d)| o / d)
            .collect();
        let jump = gains
            .windows(2)
            .fold(0f32, |m, w| m.max((w[0] - w[1]).abs()));
        assert!(jump < 0.01, "gain glides, {jump}");
    }

    #[test]
    fn deesser_cuts_sibilance_not_voice() {
        let s = DeEsserSettings {
            on: true,
            frequency: 6500.0,
            amount_db: 8.0,
            ..Default::default()
        };
        let mut d = DeEsser::new(RATE, 2, &s);
        let mut voice = sine(300.0, -12.0, 0.5);
        d.process(&mut voice);
        assert!(d.take_meter() < 0.5);
        let v = peak(&voice[voice.len() / 2..]);
        assert!((to_db(v) + 12.0).abs() < 0.3);
        let mut ess = sine(6500.0, -12.0, 0.5);
        d.process(&mut ess);
        assert!(d.take_meter() > 7.5);
        let e = to_db(peak(&ess[ess.len() / 2..]));
        assert!((e + 20.0).abs() < 0.7, "{e}");
    }

    #[test]
    fn a_higher_deesser_threshold_cuts_less() {
        let cut = |threshold_db| {
            let s = DeEsserSettings { on: true, amount_db: 12.0, threshold_db, ..Default::default() };
            let mut d = DeEsser::new(RATE, 2, &s);
            let mut x = sine(300.0, -12.0, 0.5);
            for (v, e) in x.iter_mut().zip(sine(6500.0, -20.0, 0.5)) {
                *v += e;
            }
            d.process(&mut x);
            d.take_meter()
        };
        assert!(cut(-20.0) > cut(-10.0) + 3.0);
        assert_eq!(cut(0.0), 0.0);
        let clean = DeEsserSettings { threshold_db: -99.0, ..Default::default() }.clean();
        assert_eq!(clean.threshold_db, -30.0);
    }

    #[test]
    fn blocks_survive_nan_and_decay_without_denormals() {
        let mut c = Compressor::new(RATE, 2, &CompSettings::default());
        let mut l = Limiter::new(RATE, 2, &LimiterSettings::default());
        let mut a = Agc::new(RATE, 2, &AgcSettings::default());
        let mut d = DeEsser::new(RATE, 2, &DeEsserSettings::default());
        let mut x = sine(7000.0, -6.0, 0.1);
        x[100] = f32::NAN;
        x[201] = f32::INFINITY;
        x[300] = f32::NEG_INFINITY;
        x.extend(vec![0.0; 480000]);
        c.process(&mut x);
        l.process(&mut x);
        a.process(&mut x, false);
        d.process(&mut x);
        assert!(x.iter().all(|s| s.is_finite()));
        assert!(
            x[x.len() - 2000..]
                .iter()
                .all(|s| *s == 0.0 || s.is_normal())
        );
        assert!(d.state.iter().flatten().all(|s| *s == 0.0 || s.is_normal()));
        assert!(d.band_env == 0.0 || d.band_env.is_normal());
        assert!(a.fast == 0.0 || a.fast.is_normal());
        let mut after = sine(1000.0, -6.0, 0.05);
        c.process(&mut after);
        l.process(&mut after);
        a.process(&mut after, false);
        d.process(&mut after);
        assert!(peak(&after) > 0.1, "still works after the bad samples");
    }

    #[test]
    fn works_at_44100() {
        let mut l = Limiter::new(
            44100.0,
            2,
            &LimiterSettings {
                on: true,
                ..Default::default()
            },
        );
        assert_eq!(l.latency(), 88);
        let mut x = vec![1.0f32; 2000];
        l.process(&mut x);
        assert!(peak(&x) <= from_db(-1.0) + 1e-6);
    }
}
