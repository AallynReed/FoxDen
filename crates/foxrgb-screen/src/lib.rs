use foxrgb_core::ScreenFrame;

/// Captures the primary monitor, downscaled, for screen ambience.
pub struct ScreenCapture;

impl ScreenCapture {
    /// Starts capture on a background thread. Capture stops when the value is dropped.
    pub fn start() -> Result<Self, String> {
        Err("screen capture is not implemented yet".into())
    }

    /// Latest downscaled frame. Cheap; called once per rendered frame.
    pub fn frame(&self) -> ScreenFrame {
        ScreenFrame::default()
    }
}
