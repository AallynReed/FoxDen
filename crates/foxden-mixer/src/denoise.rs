//! Noise suppression with nnnoiseless: mono, 48 kHz, 480-sample frames. Other rates are
//! resampled linearly on the way in and out.

use std::collections::VecDeque;

use nnnoiseless::DenoiseState;
use serde::{Deserialize, Serialize};

pub const FRAME: usize = DenoiseState::FRAME_SIZE;
const MODEL_RATE: f32 = 48000.0;
const PCM: f32 = 32768.0;
const FIFO: usize = 8192;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DenoiseSettings {
    pub on: bool,
    /// 0 keeps the dry sound, 1 is fully denoised.
    pub strength: f32,
}

impl Default for DenoiseSettings {
    fn default() -> Self {
        Self {
            on: false,
            strength: 1.0,
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
        }
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
        self.strength = s.clean().strength;
    }

    /// Empties the buffers, as when it is switched on again.
    pub fn reset(&mut self) {
        self.input.clear();
        self.model_out.clear();
        self.output.clear();
        self.output.extend(std::iter::repeat_n(0.0, self.prime));
        self.dry.fill(0.0);
    }

    /// Delay added, in samples at the stream's rate: one frame of buffering and one of overlap.
    pub fn latency(&self) -> usize {
        self.prime + (FRAME as f32 * self.rate / MODEL_RATE).round() as usize
    }

    fn run_frames(&mut self) {
        while self.input.len() >= FRAME {
            for (f, x) in self.frame_in.iter_mut().zip(self.input.drain(..FRAME)) {
                *f = x * PCM;
            }
            self.state
                .process_frame(&mut self.frame_out, &self.frame_in);
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
}
