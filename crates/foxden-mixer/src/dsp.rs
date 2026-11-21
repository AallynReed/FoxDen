//! Parametric EQ from the standard RBJ biquads.

use serde::{Deserialize, Serialize};

pub const MAX_BANDS: usize = 10;
pub const MIN_FREQ: f32 = 20.0;
pub const MAX_FREQ: f32 = 20000.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum FilterType {
    ByPass = 0,
    LowPass = 1,
    HighPass = 2,
    BandPassPeakQ = 3,
    BandPassPeak0dB = 4,
    NotchFilter = 5,
    AllPass = 6,
    #[default]
    PeakingEQ = 7,
    LowShelving = 8,
    HighShelving = 9,
}

impl FilterType {
    pub const ALL: [FilterType; 10] = [
        FilterType::ByPass,
        FilterType::LowPass,
        FilterType::HighPass,
        FilterType::BandPassPeakQ,
        FilterType::BandPassPeak0dB,
        FilterType::NotchFilter,
        FilterType::AllPass,
        FilterType::PeakingEQ,
        FilterType::LowShelving,
        FilterType::HighShelving,
    ];

    pub fn label(self) -> &'static str {
        match self {
            FilterType::ByPass => "Bypass",
            FilterType::LowPass => "Low pass",
            FilterType::HighPass => "High pass",
            FilterType::BandPassPeakQ => "Band pass (Q)",
            FilterType::BandPassPeak0dB => "Band pass (0 dB)",
            FilterType::NotchFilter => "Notch",
            FilterType::AllPass => "All pass",
            FilterType::PeakingEQ => "Peaking",
            FilterType::LowShelving => "Low shelf",
            FilterType::HighShelving => "High shelf",
        }
    }

    pub fn uses_gain(self) -> bool {
        matches!(
            self,
            FilterType::PeakingEQ | FilterType::LowShelving | FilterType::HighShelving
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Band {
    pub enabled: bool,
    pub filter: FilterType,
    pub frequency: f32,
    pub gain_db: f32,
    pub q: f32,
}

impl Default for Band {
    fn default() -> Self {
        Self {
            enabled: true,
            filter: FilterType::PeakingEQ,
            frequency: 1000.0,
            gain_db: 0.0,
            q: 0.707,
        }
    }
}

/// Ten flat bands at octave centres, 32 Hz to 16 kHz.
pub fn default_bands() -> Vec<Band> {
    [
        32.0, 64.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
    ]
    .iter()
    .map(|&f| Band {
        frequency: f,
        q: 1.414,
        ..Band::default()
    })
    .collect()
}

#[derive(Debug, Clone, Copy, Default)]
struct Coeffs {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

fn coeffs(band: &Band, rate: f32) -> Option<Coeffs> {
    if !band.enabled || band.filter == FilterType::ByPass {
        return None;
    }
    let f = band.frequency.clamp(MIN_FREQ, (rate * 0.49).min(MAX_FREQ));
    let q = band.q.max(0.05);
    let w0 = 2.0 * std::f32::consts::PI * f / rate;
    let (sin, cos) = w0.sin_cos();
    let alpha = sin / (2.0 * q);
    let a = 10f32.powf(band.gain_db / 40.0);
    let (b0, b1, b2, a0, a1, a2) = match band.filter {
        FilterType::LowPass => (
            (1.0 - cos) / 2.0,
            1.0 - cos,
            (1.0 - cos) / 2.0,
            1.0 + alpha,
            -2.0 * cos,
            1.0 - alpha,
        ),
        FilterType::HighPass => (
            (1.0 + cos) / 2.0,
            -(1.0 + cos),
            (1.0 + cos) / 2.0,
            1.0 + alpha,
            -2.0 * cos,
            1.0 - alpha,
        ),
        FilterType::BandPassPeakQ => (
            sin / 2.0,
            0.0,
            -sin / 2.0,
            1.0 + alpha,
            -2.0 * cos,
            1.0 - alpha,
        ),
        FilterType::BandPassPeak0dB => (alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
        FilterType::NotchFilter => (1.0, -2.0 * cos, 1.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
        FilterType::AllPass => (
            1.0 - alpha,
            -2.0 * cos,
            1.0 + alpha,
            1.0 + alpha,
            -2.0 * cos,
            1.0 - alpha,
        ),
        FilterType::PeakingEQ => (
            1.0 + alpha * a,
            -2.0 * cos,
            1.0 - alpha * a,
            1.0 + alpha / a,
            -2.0 * cos,
            1.0 - alpha / a,
        ),
        FilterType::LowShelving => {
            let s = 2.0 * a.sqrt() * alpha;
            (
                a * ((a + 1.0) - (a - 1.0) * cos + s),
                2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                a * ((a + 1.0) - (a - 1.0) * cos - s),
                (a + 1.0) + (a - 1.0) * cos + s,
                -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                (a + 1.0) + (a - 1.0) * cos - s,
            )
        }
        FilterType::HighShelving => {
            let s = 2.0 * a.sqrt() * alpha;
            (
                a * ((a + 1.0) + (a - 1.0) * cos + s),
                -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                a * ((a + 1.0) + (a - 1.0) * cos - s),
                (a + 1.0) - (a - 1.0) * cos + s,
                2.0 * ((a - 1.0) - (a + 1.0) * cos),
                (a + 1.0) - (a - 1.0) * cos - s,
            )
        }
        FilterType::ByPass => unreachable!(),
    };
    Some(Coeffs {
        b0: b0 / a0,
        b1: b1 / a0,
        b2: b2 / a0,
        a1: a1 / a0,
        a2: a2 / a0,
    })
}

/// A chain of biquads with its own state per audio channel.
#[derive(Debug, Clone, Default)]
pub struct Equalizer {
    stages: Vec<Coeffs>,
    state: Vec<[f32; 4]>,
    channels: usize,
}

impl Equalizer {
    pub fn new(bands: &[Band], rate: f32, channels: usize) -> Self {
        let stages: Vec<Coeffs> = bands
            .iter()
            .take(MAX_BANDS)
            .filter_map(|b| coeffs(b, rate))
            .collect();
        let state = vec![[0.0; 4]; stages.len() * channels.max(1)];
        Self {
            stages,
            state,
            channels: channels.max(1),
        }
    }

    pub fn is_flat(&self) -> bool {
        self.stages.is_empty()
    }

    /// Filters interleaved samples in place.
    pub fn process(&mut self, samples: &mut [f32]) {
        if self.stages.is_empty() {
            return;
        }
        for (i, s) in samples.iter_mut().enumerate() {
            let ch = i % self.channels;
            let mut x = *s;
            for (k, c) in self.stages.iter().enumerate() {
                let st = &mut self.state[k * self.channels + ch];
                let y = c.b0 * x + c.b1 * st[0] + c.b2 * st[1] - c.a1 * st[2] - c.a2 * st[3];
                st[1] = st[0];
                st[0] = x;
                st[3] = st[2];
                st[2] = y;
                x = y;
            }
            *s = x;
        }
    }
}

/// Magnitude in dB of the whole chain at `freq`, for drawing the EQ curve.
pub fn response_db(bands: &[Band], rate: f32, freq: f32) -> f32 {
    let w = 2.0 * std::f32::consts::PI * freq / rate;
    let (z1r, z1i) = (w.cos(), -w.sin());
    let (z2r, z2i) = ((2.0 * w).cos(), -(2.0 * w).sin());
    let mut db = 0.0;
    for c in bands.iter().filter_map(|b| coeffs(b, rate)) {
        let nr = c.b0 + c.b1 * z1r + c.b2 * z2r;
        let ni = c.b1 * z1i + c.b2 * z2i;
        let dr = 1.0 + c.a1 * z1r + c.a2 * z2r;
        let di = c.a1 * z1i + c.a2 * z2i;
        let mag = ((nr * nr + ni * ni) / (dr * dr + di * di).max(1e-12)).sqrt();
        db += 20.0 * mag.max(1e-9).log10();
    }
    db
}

/// A noise gate: silent below the threshold, open while the level is above it and for a moment after.
#[derive(Debug, Clone)]
pub struct Gate {
    channels: usize,
    open: f32,
    hold: u32,
    hold_frames: u32,
    attack: f32,
    release: f32,
}

impl Gate {
    pub fn new(rate: f32, channels: usize) -> Self {
        let per = |ms: f32| 1.0 - (-1.0 / (rate * ms / 1000.0)).exp();
        Self {
            channels: channels.max(1),
            open: 0.0,
            hold: 0,
            hold_frames: (rate * 0.15) as u32,
            attack: per(2.0),
            release: per(60.0),
        }
    }

    /// Gates interleaved samples in place; `threshold` is a linear level.
    pub fn process(&mut self, samples: &mut [f32], threshold: f32) {
        for frame in samples.chunks_mut(self.channels) {
            let level = frame.iter().fold(0f32, |m, s| m.max(s.abs()));
            if level >= threshold {
                self.hold = self.hold_frames;
            } else {
                self.hold = self.hold.saturating_sub(1);
            }
            let (target, speed) = if self.hold > 0 {
                (1.0, self.attack)
            } else {
                (0.0, self.release)
            };
            self.open += (target - self.open) * speed;
            for s in frame.iter_mut() {
                *s *= self.open;
            }
        }
    }
}

pub fn db_to_linear(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_closes_on_quiet_and_opens_on_loud() {
        let mut g = Gate::new(48000.0, 2);
        let t = db_to_linear(-40.0);
        let mut quiet = vec![0.001f32; 48000];
        g.process(&mut quiet, t);
        assert!(quiet.iter().all(|s| s.abs() < 0.001));
        let mut loud = vec![0.5f32; 48000];
        g.process(&mut loud, t);
        assert!((loud[loud.len() - 1] - 0.5).abs() < 1e-3);
        let mut tail = vec![0.001f32; 96000];
        g.process(&mut tail, t);
        assert!(tail[100] > 0.0009, "held open just after speech");
        assert!(tail[tail.len() - 1] < 1e-6, "closed well after");
    }

    #[test]
    fn peaking_hits_its_gain() {
        let b = [Band {
            gain_db: 6.0,
            frequency: 1000.0,
            q: 1.0,
            ..Band::default()
        }];
        let db = response_db(&b, 48000.0, 1000.0);
        assert!((db - 6.0).abs() < 0.1, "{db}");
        assert!(response_db(&b, 48000.0, 50.0).abs() < 0.5);
    }

    #[test]
    fn flat_is_identity() {
        let mut eq = Equalizer::new(&default_bands(), 48000.0, 2);
        let mut s = [0.5f32, -0.5, 0.25, 0.1];
        eq.process(&mut s);
        assert!((s[0] - 0.5).abs() < 1e-4);
    }
}
