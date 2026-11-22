//! Processing chains. Mic: noise suppression, gate, level stabilizer, compressor, de-esser, EQ,
//! limiter. Outputs and master: compressor, limiter. Volume comes after, in the router.

use serde::{Deserialize, Serialize};

use crate::denoise::{DenoiseSettings, Denoiser};
use crate::dsp::{Equalizer, Gate};
use crate::dynamics::{
    Agc, AgcSettings, CompSettings, Compressor, DeEsser, DeEsserSettings, Limiter, LimiterSettings,
};

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Processing {
    pub denoise: DenoiseSettings,
    pub agc: AgcSettings,
    pub comp: CompSettings,
    pub deesser: DeEsserSettings,
    pub limiter: LimiterSettings,
}

impl Processing {
    /// Values in range; outputs keep only the compressor and limiter.
    pub fn clean(self, mic: bool) -> Self {
        let d = Self::default();
        Self {
            denoise: if mic { self.denoise.clean() } else { d.denoise },
            agc: if mic { self.agc.clean() } else { d.agc },
            comp: self.comp.clean(),
            deesser: if mic { self.deesser.clean() } else { d.deesser },
            limiter: self.limiter.clean(),
        }
    }
}

/// Gain reduction in dB since the last read; `agc` is the stabilizer's gain, + is a boost.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize)]
pub struct Reduction {
    pub agc: f32,
    pub comp: f32,
    pub deess: f32,
    pub limit: f32,
}

impl Reduction {
    pub fn merge(&mut self, n: Reduction) {
        self.agc = n.agc;
        self.comp = self.comp.max(n.comp);
        self.deess = self.deess.max(n.deess);
        self.limit = self.limit.max(n.limit);
    }
}

pub struct MicChain {
    s: Processing,
    denoise: Denoiser,
    gate: Gate,
    agc: Agc,
    comp: Compressor,
    deesser: DeEsser,
    pub eq: Equalizer,
    limiter: Limiter,
}

impl MicChain {
    pub fn new(rate: f32, channels: usize, s: &Processing) -> Self {
        Self {
            s: *s,
            denoise: Denoiser::new(rate, channels, &s.denoise),
            gate: Gate::new(rate, channels),
            agc: Agc::new(rate, channels, &s.agc),
            comp: Compressor::new(rate, channels, &s.comp),
            deesser: DeEsser::new(rate, channels, &s.deesser),
            eq: Equalizer::default(),
            limiter: Limiter::new(rate, channels, &s.limiter),
        }
    }

    pub fn set(&mut self, s: &Processing) {
        if s.denoise.on && !self.s.denoise.on {
            self.denoise.reset();
        }
        if s.comp.on && !self.s.comp.on {
            self.comp.reset();
        }
        if s.limiter.on && !self.s.limiter.on {
            self.limiter.reset();
        }
        self.s = *s;
        self.denoise.set(&s.denoise);
        self.agc.set(&s.agc);
        self.comp.set(&s.comp);
        self.deesser.set(&s.deesser);
        self.limiter.set(&s.limiter);
    }

    /// `gate` is the threshold as a linear level, when the gate is on.
    pub fn process(&mut self, samples: &mut [f32], gate: Option<f32>) {
        if self.s.denoise.on {
            self.denoise.process(samples);
        }
        if let Some(t) = gate {
            self.gate.process(samples, t);
        }
        if self.s.agc.on {
            self.agc
                .process(samples, gate.is_some() && self.gate.closed());
        }
        if self.s.comp.on {
            self.comp.process(samples);
        }
        if self.s.deesser.on {
            self.deesser.process(samples);
        }
        self.eq.process(samples);
        if self.s.limiter.on {
            self.limiter.process(samples);
        }
    }

    pub fn take_meter(&mut self) -> Reduction {
        Reduction {
            agc: if self.s.agc.on {
                self.agc.gain_db()
            } else {
                0.0
            },
            comp: self.comp.take_meter(),
            deess: self.deesser.take_meter(),
            limit: self.limiter.take_meter(),
        }
    }

    /// Delay the switched-on blocks add, in samples.
    pub fn latency(&self) -> usize {
        let d = if self.s.denoise.on {
            self.denoise.latency()
        } else {
            0
        };
        d + if self.s.limiter.on {
            self.limiter.latency()
        } else {
            0
        }
    }
}

pub struct OutputChain {
    s: Processing,
    comp: Compressor,
    limiter: Limiter,
}

impl OutputChain {
    pub fn new(rate: f32, channels: usize, s: &Processing) -> Self {
        Self {
            s: *s,
            comp: Compressor::new(rate, channels, &s.comp),
            limiter: Limiter::new(rate, channels, &s.limiter),
        }
    }

    pub fn set(&mut self, s: &Processing) {
        if s.comp.on && !self.s.comp.on {
            self.comp.reset();
        }
        if s.limiter.on && !self.s.limiter.on {
            self.limiter.reset();
        }
        self.s = *s;
        self.comp.set(&s.comp);
        self.limiter.set(&s.limiter);
    }

    pub fn process(&mut self, samples: &mut [f32]) {
        if self.s.comp.on {
            self.comp.process(samples);
        }
        if self.s.limiter.on {
            self.limiter.process(samples);
        }
    }

    pub fn take_meter(&mut self) -> Reduction {
        Reduction {
            comp: self.comp.take_meter(),
            limit: self.limiter.take_meter(),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::{Band, default_bands};
    use std::time::Instant;

    fn everything() -> Processing {
        Processing {
            denoise: DenoiseSettings {
                on: true,
                strength: 0.9,
            },
            agc: AgcSettings {
                on: true,
                ..Default::default()
            },
            comp: CompSettings {
                on: true,
                ..Default::default()
            },
            deesser: DeEsserSettings {
                on: true,
                ..Default::default()
            },
            limiter: LimiterSettings {
                on: true,
                ..Default::default()
            },
        }
    }

    fn speech_like(frames: usize, chans: usize) -> Vec<f32> {
        (0..frames)
            .flat_map(|i| {
                let t = i as f32 / 48000.0;
                let v = (t * 880.0).sin() * (t * 3.0).sin().abs() * 0.4
                    + ((i * 2654435761) % 1000) as f32 / 1000.0 * 0.01;
                std::iter::repeat_n(v, chans)
            })
            .collect()
    }

    fn eq_bands() -> Vec<Band> {
        let mut b = default_bands();
        b[6].gain_db = 3.0;
        b
    }

    #[test]
    fn bypassed_chain_is_identity() {
        let mut m = MicChain::new(48000.0, 2, &Processing::default());
        let input = speech_like(4800, 2);
        let mut x = input.clone();
        m.process(&mut x, None);
        assert_eq!(x, input);
        assert_eq!(m.latency(), 0);
        let mut o = OutputChain::new(48000.0, 2, &Processing::default());
        o.process(&mut x);
        assert_eq!(x, input);
    }

    #[test]
    fn full_chain_stays_under_the_ceiling() {
        let mut m = MicChain::new(48000.0, 2, &everything());
        m.eq = Equalizer::new(&eq_bands(), 48000.0, 2);
        let mut x: Vec<f32> = speech_like(48000, 2).iter().map(|v| v * 4.0).collect();
        for block in x.chunks_mut(960) {
            m.process(block, Some(0.003));
        }
        let ceiling = 10f32.powf(-1.0 / 20.0);
        assert!(x.iter().all(|v| v.is_finite() && v.abs() <= ceiling + 1e-6));
        assert!(m.take_meter().comp > 0.0);
        assert_eq!(m.latency(), 960 + 96);
    }

    /// CPU per second of audio for the whole mic chain, stereo at 48 kHz in 10 ms blocks.
    #[test]
    fn mic_chain_cost() {
        let secs = 10;
        let mut m = MicChain::new(48000.0, 2, &everything());
        m.eq = Equalizer::new(&eq_bands(), 48000.0, 2);
        let mut x = speech_like(48000 * secs, 2);
        let start = Instant::now();
        for block in x.chunks_mut(960) {
            m.process(block, Some(0.003));
        }
        let full = start.elapsed().as_secs_f64() * 1000.0 / secs as f64;
        let mut off = MicChain::new(48000.0, 2, &Processing::default());
        let start = Instant::now();
        for block in x.chunks_mut(960) {
            off.process(block, None);
        }
        let bypassed = start.elapsed().as_secs_f64() * 1000.0 / secs as f64;
        println!("mic chain: {full:.2} ms CPU per second of audio, bypassed {bypassed:.3} ms");
        if !cfg!(debug_assertions) {
            assert!(full < 50.0, "{full} ms per second");
            assert!(bypassed < 0.5, "{bypassed} ms per second");
        }
    }
}
