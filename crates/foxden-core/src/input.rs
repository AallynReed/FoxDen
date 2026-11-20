use crate::Color;
use serde::Serialize;

pub const AUDIO_BANDS: usize = 16;

/// Latest audio analysis, low to high frequency. All values 0..=1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct AudioFrame {
    pub level: f32,
    pub bands: [f32; AUDIO_BANDS],
}

/// Downscaled desktop image, row-major. Empty while capture is off.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScreenFrame {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<Color>,
}

impl ScreenFrame {
    /// Nearest pixel to canvas position `x`, `y` (0..=1). Black when empty.
    pub fn sample(&self, x: f32, y: f32) -> Color {
        if self.width == 0 || self.height == 0 || self.pixels.len() < self.width * self.height {
            return Color::BLACK;
        }
        let px = ((x.clamp(0.0, 1.0) * self.width as f32) as usize).min(self.width - 1);
        let py = ((y.clamp(0.0, 1.0) * self.height as f32) as usize).min(self.height - 1);
        self.pixels[py * self.width + px]
    }
}

/// A recent key press at canvas position `x`, `y`, `age` seconds ago.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Press {
    pub x: f32,
    pub y: f32,
    pub age: f32,
    /// Half the key's width and height on the canvas.
    pub hw: f32,
    pub hh: f32,
    /// Different for every press, for colours picked at random.
    pub seed: u32,
}

/// Everything an effect may react to besides position and time.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Inputs {
    pub audio: AudioFrame,
    pub screen: ScreenFrame,
    pub presses: Vec<Press>,
    /// Each lit keyboard on the canvas: its rect, and whether it has a number pad.
    pub keyboards: Vec<([f32; 4], bool)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_sample() {
        let frame = ScreenFrame {
            width: 2,
            height: 1,
            pixels: vec![Color(1, 0, 0), Color(2, 0, 0)],
        };
        assert_eq!(frame.sample(0.1, 0.5), Color(1, 0, 0));
        assert_eq!(frame.sample(1.0, 1.0), Color(2, 0, 0));
        assert_eq!(ScreenFrame::default().sample(0.5, 0.5), Color::BLACK);
    }
}
