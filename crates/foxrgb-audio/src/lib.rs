use foxrgb_core::AudioFrame;

/// Captures what the default output device is playing and analyses it.
pub struct AudioCapture;

impl AudioCapture {
    /// Starts loopback capture on a background thread. Capture stops when the value is dropped.
    pub fn start() -> Result<Self, String> {
        Err("audio capture is not implemented yet".into())
    }

    /// Latest analysis. Cheap; called once per rendered frame.
    pub fn frame(&self) -> AudioFrame {
        AudioFrame::default()
    }
}
