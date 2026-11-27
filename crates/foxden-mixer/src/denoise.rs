//! AI noise removal: light with nnnoiseless (RNNoise), strong with DeepFilterNet 3. Both run on
//! mono 48 kHz 480-sample frames; other rates are resampled linearly on the way in and out.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use df::tract::{DfParams, DfTract, RuntimeParams};
use ndarray::Array2;
use nnnoiseless::DenoiseState;
use serde::{Deserialize, Serialize};

pub const FRAME: usize = DenoiseState::FRAME_SIZE;
const MODEL_RATE: f32 = 48000.0;
const PCM: f32 = 32768.0;
const FIFO: usize = 8192;

/// A strong frame taking longer than this share of its 10 ms, on average, falls back to light.
const KEEP_UP: f32 = 0.6;
const WARM_FRAMES: u32 = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Engine {
    #[default]
    Light,
    Strong,
    /// Strong while it keeps up, else light.
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DenoiseSettings {
    pub on: bool,
    /// Light: 0 keeps the dry sound, 1 is fully denoised. Strong: how far noise may be pushed down.
    pub strength: f32,
    pub engine: Engine,
}

impl Default for DenoiseSettings {
    fn default() -> Self {
        Self {
            on: false,
            strength: 1.0,
            engine: Engine::Light,
        }
    }
}

impl DenoiseSettings {
    pub fn clean(self) -> Self {
        Self {
            on: self.on,
            strength: if self.strength.is_finite() {
                self.strength.clamp(0.0, 1.0)
            } else {
                1.0
            },
            engine: self.engine,
        }
    }
}

/// The strong engine's attenuation limit for a strength.
pub fn attenuation_db(strength: f32) -> f32 {
    if strength >= 0.99 { 100.0 } else { 6.0 + 30.0 * strength.clamp(0.0, 1.0) }
}

/// What the running chain's noise removal is doing, for the Audio tab and AI assistants.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Status {
    /// "light" or "strong".
    pub running: &'static str,
    pub strong_ready: bool,
    /// The strong engine could not keep up and light took over.
    pub fell_back: bool,
    pub latency_ms: f32,
    /// Average time per 10 ms frame, and the share of one core that is.
    pub light_ms: f32,
    pub strong_ms: f32,
    pub light_cpu_percent: f32,
    pub strong_cpu_percent: f32,
    pub error: Option<String>,
}

static STATUS: Mutex<Option<Status>> = Mutex::new(None);

/// The last report of the mic chain on the audio thread; empty until noise removal has run.
pub fn status() -> Option<Status> {
    STATUS.lock().ok().and_then(|s| s.clone())
}

struct Strong {
    model: DfTract,
    noisy: Array2<f32>,
    enh: Array2<f32>,
    delay: usize,
}

// The models are plain data, not tied to the thread that built them.
unsafe impl Send for Strong {}

fn load_strong(strength: f32) -> Result<Strong, String> {
    let params = RuntimeParams::default_with_ch(1).with_atten_lim(attenuation_db(strength));
    let model = DfTract::new(DfParams::default(), &params).map_err(|e| format!("strong noise removal: {e}"))?;
    let hop = model.hop_size;
    if hop != FRAME || model.sr as f32 != MODEL_RATE {
        return Err("strong noise removal: unexpected model frame".into());
    }
    let delay = model.fft_size - hop + model.lookahead * hop;
    Ok(Strong { model, noisy: Array2::zeros((1, hop)), enh: Array2::zeros((1, hop)), delay })
}

type Slot = Arc<Mutex<Option<Result<Strong, String>>>>;

/// Average time per frame, in ms.
#[derive(Default, Clone, Copy)]
struct Clock {
    ms: f32,
    frames: u32,
}

impl Clock {
    fn add(&mut self, since: Instant) {
        let ms = since.elapsed().as_secs_f32() * 1000.0;
        self.frames = self.frames.saturating_add(1);
        let k = if self.frames < 20 { 1.0 / self.frames as f32 } else { 0.05 };
        self.ms += (ms - self.ms) * k;
    }
}

/// Streaming linear resampler.
#[derive(Debug, Clone)]
struct Linear {
    step: f64,
    t: f64,
    prev: f32,
}

impl Linear {
    fn new(from: f32, to: f32) -> Self {
        Self {
            step: f64::from(from) / f64::from(to),
            t: 0.0,
            prev: 0.0,
        }
    }

    fn push(&mut self, x: f32, out: &mut VecDeque<f32>) {
        while self.t < 1.0 {
            if out.len() < FIFO {
                out.push_back(self.prev + (x - self.prev) * self.t as f32);
            }
            self.t += self.step;
        }
        self.t -= 1.0;
        self.prev = x;
    }
}

pub struct Denoiser {
    channels: usize,
    rate: f32,
    state: Box<DenoiseState<'static>>,
    strength: f32,
    on: bool,
    engine: Engine,
    strong: Option<Strong>,
    slot: Option<Slot>,
    strong_error: Option<String>,
    /// Strong is in use; it changes only between frames, with the buffers emptied.
    using_strong: bool,
    fell_back: bool,
    realtime: bool,
    light_clock: Clock,
    strong_clock: Clock,
    down: Option<Linear>,
    up: Option<Linear>,
    input: VecDeque<f32>,
    model_out: VecDeque<f32>,
    output: VecDeque<f32>,
    frame_in: Vec<f32>,
    frame_out: Vec<f32>,
    dry: Vec<f32>,
    prime: usize,
}

impl Denoiser {
    pub fn new(rate: f32, channels: usize, s: &DenoiseSettings) -> Self {
        let resample = (rate - MODEL_RATE).abs() > 0.5;
        let prime = if resample {
            (FRAME as f32 * rate / MODEL_RATE).ceil() as usize + 2
        } else {
            FRAME
        };
        let mut d = Self {
            channels: channels.max(1),
            rate,
            state: DenoiseState::new(),
            strength: 1.0,
            on: false,
            engine: Engine::Light,
            strong: None,
            slot: None,
            strong_error: None,
            using_strong: false,
            fell_back: false,
            realtime: true,
            light_clock: Clock::default(),
            strong_clock: Clock::default(),
            down: resample.then(|| Linear::new(rate, MODEL_RATE)),
            up: resample.then(|| Linear::new(MODEL_RATE, rate)),
            input: VecDeque::with_capacity(FIFO),
            model_out: VecDeque::with_capacity(FIFO),
            output: VecDeque::with_capacity(FIFO),
            frame_in: vec![0.0; FRAME],
            frame_out: vec![0.0; FRAME],
            dry: vec![0.0; FRAME],
            prime,
        };
        d.set(s);
        d.reset();
        d
    }

    pub fn set(&mut self, s: &DenoiseSettings) {
        let s = s.clean();
        if s.engine != self.engine {
            self.fell_back = false;
        }
        let strength_changed = (s.strength - self.strength).abs() > 1e-4;
        self.strength = s.strength;
        self.on = s.on;
        self.engine = s.engine;
        if let Some(strong) = &mut self.strong
            && strength_changed
        {
            strong.model.set_atten_lim(attenuation_db(s.strength));
        }
        if s.on && s.engine != Engine::Light && self.strong.is_none() && self.slot.is_none() && self.strong_error.is_none() {
            let slot: Slot = Arc::default();
            let fill = slot.clone();
            let strength = s.strength;
            let started = std::thread::Builder::new().name("foxden-denoise-load".into()).spawn(move || {
                let loaded = load_strong(strength);
                if let Ok(mut f) = fill.lock() {
                    *f = Some(loaded);
                }
            });
            match started {
                Ok(_) => self.slot = Some(slot),
                Err(e) => self.strong_error = Some(e.to_string()),
            }
        }
    }

    /// For recordings processed off the audio thread: loads the strong engine now and never
    /// falls back for being slow.
    pub fn offline(&mut self) {
        self.realtime = false;
        if self.on && self.engine != Engine::Light && self.strong.is_none() {
            if let Some(slot) = self.slot.take() {
                while slot.lock().is_ok_and(|s| s.is_none()) {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                match slot.lock().ok().and_then(|mut s| s.take()) {
                    Some(Ok(s)) => self.strong = Some(s),
                    Some(Err(e)) => self.strong_error = Some(e),
                    None => {}
                }
            } else {
                match load_strong(self.strength) {
                    Ok(s) => self.strong = Some(s),
                    Err(e) => self.strong_error = Some(e),
                }
            }
        }
    }

    /// Which engine runs now: "light" or "strong".
    pub fn running(&self) -> &'static str {
        if self.using_strong { "strong" } else { "light" }
    }

    fn wants_strong(&self) -> bool {
        self.engine != Engine::Light && !self.fell_back && self.strong.is_some()
    }

    /// Takes up a strong engine that finished loading, without waiting for the lock.
    fn collect(&mut self) {
        let Some(slot) = &self.slot else { return };
        let Ok(mut s) = slot.try_lock() else { return };
        match s.take() {
            Some(Ok(strong)) => self.strong = Some(strong),
            Some(Err(e)) => self.strong_error = Some(e),
            None => return,
        }
        drop(s);
        self.slot = None;
    }

    fn report(&self) {
        if !self.realtime {
            return;
        }
        let Ok(mut s) = STATUS.try_lock() else { return };
        let cpu = |c: &Clock| (c.ms * 100.0).round() / 10.0;
        *s = Some(Status {
            running: self.running(),
            strong_ready: self.strong.is_some(),
            fell_back: self.fell_back,
            latency_ms: self.latency() as f32 * 1000.0 / self.rate,
            light_ms: self.light_clock.ms,
            strong_ms: self.strong_clock.ms,
            light_cpu_percent: cpu(&self.light_clock),
            strong_cpu_percent: cpu(&self.strong_clock),
            error: self.strong_error.clone(),
        });
    }

    /// Empties the buffers, as when it is switched on again.
    pub fn reset(&mut self) {
        self.input.clear();
        self.model_out.clear();
        self.output.clear();
        self.output.extend(std::iter::repeat_n(0.0, self.prime));
        self.dry.fill(0.0);
    }

    /// Delay added, in samples at the stream rate: one frame of buffering, then the delay of the
    /// engine, which is one frame for light.
    pub fn latency(&self) -> usize {
        let engine = match (&self.strong, self.using_strong) {
            (Some(s), true) => s.delay,
            _ => FRAME,
        };
        self.prime + (engine as f32 * self.rate / MODEL_RATE).round() as usize
    }

    fn run_frames(&mut self) {
        self.collect();
        let strong = self.wants_strong();
        if strong != self.using_strong {
            self.using_strong = strong;
            let keep: Vec<f32> = self.input.drain(..).collect();
            self.reset();
            self.input.extend(keep);
        }
        while self.input.len() >= FRAME {
            if self.using_strong {
                self.run_strong();
                continue;
            }
            let at = Instant::now();
            for (f, x) in self.frame_in.iter_mut().zip(self.input.drain(..FRAME)) {
                *f = x * PCM;
            }
            self.state
                .process_frame(&mut self.frame_out, &self.frame_in);
            self.light_clock.add(at);
            let wet = self.strength;
            for ((o, d), x) in self
                .frame_out
                .iter()
                .zip(self.dry.iter_mut())
                .zip(&self.frame_in)
            {
                // The model's output lags its input by one frame; so does `dry`.
                let v = (o * wet + *d * (1.0 - wet)) / PCM;
                *d = *x;
                if self.model_out.len() < FIFO {
                    self.model_out
                        .push_back(if v.is_finite() { v } else { 0.0 });
                }
            }
        }
        self.resample_out();
        let frames = self.light_clock.frames.wrapping_add(self.strong_clock.frames);
        if frames % 100 == 1 {
            self.report();
        }
    }

    fn run_strong(&mut self) {
        let at = Instant::now();
        let Some(strong) = &mut self.strong else { return };
        for (f, x) in strong.noisy.iter_mut().zip(self.input.drain(..FRAME)) {
            *f = x;
        }
        let ok = strong.model.process(strong.noisy.view(), strong.enh.view_mut()).is_ok();
        for v in strong.enh.iter() {
            let v = if ok && v.is_finite() { *v } else { 0.0 };
            if self.model_out.len() < FIFO {
                self.model_out.push_back(v);
            }
        }
        self.strong_clock.add(at);
        if self.realtime && self.strong_clock.frames > WARM_FRAMES && self.strong_clock.ms > KEEP_UP * 10.0 {
            self.fell_back = true;
        }
    }

    fn resample_out(&mut self) {
        match &mut self.up {
            Some(up) => {
                for x in self.model_out.drain(..) {
                    up.push(x, &mut self.output);
                }
            }
            None => {
                let room = FIFO.saturating_sub(self.output.len());
                let n = self.model_out.len().min(room);
                self.output.extend(self.model_out.drain(..n));
                self.model_out.clear();
            }
        }
    }

    /// Denoises interleaved samples in place; every channel gets the denoised mono mix.
    pub fn process(&mut self, samples: &mut [f32]) {
        let ch = self.channels;
        for frame in samples.chunks_exact_mut(ch) {
            let mut m = 0.0;
            for s in frame.iter() {
                if s.is_finite() {
                    m += *s;
                }
            }
            let m = m / ch as f32;
            match &mut self.down {
                Some(down) => down.push(m, &mut self.input),
                None => self.input.push_back(m),
            }
            if self.input.len() >= FRAME {
                self.run_frames();
            }
            let y = self.output.pop_front().unwrap_or(0.0);
            frame.fill(y);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(n: usize, amp: f32, seed: &mut u32) -> Vec<f32> {
        (0..n)
            .map(|_| {
                *seed ^= *seed << 13;
                *seed ^= *seed >> 17;
                *seed ^= *seed << 5;
                (*seed as f32 / u32::MAX as f32 * 2.0 - 1.0) * amp
            })
            .collect()
    }

    fn rms(s: &[f32]) -> f32 {
        (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt()
    }

    #[test]
    fn dry_mix_is_the_input_delayed_by_two_frames() {
        let mut d = Denoiser::new(
            48000.0,
            1,
            &DenoiseSettings {
                on: true,
                strength: 0.0,
                ..Default::default()
            },
        );
        assert_eq!(d.latency(), 960);
        let input: Vec<f32> = (0..4800).map(|i| (i as f32 * 0.01).sin() * 0.5).collect();
        let mut x = input.clone();
        d.process(&mut x);
        for i in 960..4800 {
            assert!((x[i] - input[i - 960]).abs() < 1e-4, "{i}");
        }
    }

    #[test]
    fn denoised_voice_lines_up_with_the_dry_path() {
        let mut d = Denoiser::new(
            48000.0,
            1,
            &DenoiseSettings {
                on: true,
                strength: 1.0,
                ..Default::default()
            },
        );
        let voice = |i: usize| {
            let t = i as f32 / 48000.0;
            let env = (0.5 + 0.5 * (t * 2.0 * std::f32::consts::PI * 3.0).sin()).powi(2);
            (1..12)
                .map(|h| (t * 2.0 * std::f32::consts::PI * 140.0 * h as f32).sin() / h as f32)
                .sum::<f32>()
                * 0.15
                * env
        };
        let input: Vec<f32> = (0..48000 * 2).map(voice).collect();
        let mut x = input.clone();
        d.process(&mut x);
        let best = (900..1020)
            .max_by(|&a, &b| {
                let c = |lag: usize| (48000..96000).map(|i| x[i] * input[i - lag]).sum::<f32>();
                c(a).total_cmp(&c(b))
            })
            .unwrap();
        assert!((best as i32 - 960).abs() <= 2, "lag {best}");
    }

    #[test]
    fn steady_noise_is_turned_down() {
        let mut seed = 1;
        let mut d = Denoiser::new(
            48000.0,
            2,
            &DenoiseSettings {
                on: true,
                strength: 1.0,
                ..Default::default()
            },
        );
        let mono = noise(48000 * 3, 0.05, &mut seed);
        let mut x: Vec<f32> = mono.iter().flat_map(|&v| [v, v]).collect();
        d.process(&mut x);
        let tail = &x[x.len() - 48000..];
        let before = rms(&mono[mono.len() - 24000..]);
        assert!(rms(tail) < before * 0.3, "{} vs {before}", rms(tail));
    }

    #[test]
    fn other_rates_keep_time_and_stay_finite() {
        let mut d = Denoiser::new(
            44100.0,
            2,
            &DenoiseSettings {
                on: true,
                strength: 0.0,
                ..Default::default()
            },
        );
        let n = 44100;
        let mut x: Vec<f32> = (0..n)
            .flat_map(|i| {
                let v = (i as f32 * 0.02).sin() * 0.3;
                [v, if i == 500 { f32::NAN } else { v }]
            })
            .collect();
        d.process(&mut x);
        assert!(x.iter().all(|v| v.is_finite()));
        assert!(
            d.output.len() < 2 * d.prime,
            "no build-up: {}",
            d.output.len()
        );
        let lag = d.latency();
        let src = |i: usize| (i as f32 * 0.02).sin() * 0.3;
        for i in (lag + 2000..n).step_by(97) {
            assert!((x[i * 2] - src(i - lag)).abs() < 0.03, "{i}");
        }
    }

    fn noisy_clicks(secs: f32, seed: &mut u32) -> Vec<f32> {
        let mut x = noise((48000.0 * secs) as usize, 0.03, seed);
        for k in 0..(secs * 4.0) as usize {
            let at = 12000 * k + 3000;
            for (i, v) in [0.5f32, -0.4, 0.3, -0.2, 0.1].iter().enumerate() {
                x[at + i] += v;
            }
        }
        x
    }

    fn run(d: &mut Denoiser, x: &[f32]) -> Vec<f32> {
        let mut y = x.to_vec();
        for block in y.chunks_mut(480) {
            d.process(block);
        }
        y
    }

    #[test]
    fn both_engines_turn_down_noise_and_clicks() {
        for engine in [Engine::Light, Engine::Strong] {
            let mut seed = 9;
            let x = noisy_clicks(3.0, &mut seed);
            let s = DenoiseSettings { on: true, strength: 1.0, engine };
            let mut d = Denoiser::new(48000.0, 1, &s);
            d.offline();
            let y = run(&mut d, &x);
            assert_eq!(d.running(), if engine == Engine::Strong { "strong" } else { "light" });
            let (a, b) = (rms(&x[48000..]), rms(&y[48000..]));
            let most = if engine == Engine::Strong { 0.3 } else { 0.9 };
            assert!(b < a * most, "{engine:?}: {b} vs {a}");
        }
    }

    #[test]
    fn the_engine_switches_and_strength_limits_strong() {
        let mut seed = 4;
        let x = noisy_clicks(2.0, &mut seed);
        let mut d = Denoiser::new(48000.0, 1, &DenoiseSettings { on: true, strength: 1.0, engine: Engine::Light });
        d.offline();
        run(&mut d, &x[..24000]);
        assert_eq!((d.running(), d.latency()), ("light", 960));
        d.set(&DenoiseSettings { on: true, strength: 1.0, engine: Engine::Auto });
        d.offline();
        let full = run(&mut d, &x);
        assert_eq!(d.running(), "strong");
        assert!(d.latency() > 960);
        let mut soft = Denoiser::new(48000.0, 1, &DenoiseSettings { on: true, strength: 0.2, engine: Engine::Strong });
        soft.offline();
        let gentle = run(&mut soft, &x);
        assert!(rms(&gentle[48000..]) > rms(&full[48000..]) * 1.5, "a low strength keeps more of the room");
        d.set(&DenoiseSettings { on: true, strength: 1.0, engine: Engine::Light });
        run(&mut d, &x[..4800]);
        assert_eq!(d.running(), "light");
    }

    #[test]
    fn a_slow_strong_engine_falls_back_to_light() {
        let mut d = Denoiser::new(48000.0, 1, &DenoiseSettings { on: true, strength: 1.0, engine: Engine::Auto });
        d.offline();
        d.realtime = true;
        d.strong_clock = Clock { ms: 9.0, frames: WARM_FRAMES + 1 };
        let mut seed = 2;
        run(&mut d, &noisy_clicks(0.5, &mut seed));
        assert!(d.fell_back);
        run(&mut d, &noisy_clicks(0.5, &mut seed));
        assert_eq!(d.running(), "light");
    }
}
