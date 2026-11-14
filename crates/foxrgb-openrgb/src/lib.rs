use foxrgb_core::{Backend, Color, Device};

/// Client for an OpenRGB SDK server.
pub struct OpenRgbBackend {
    devices: Vec<Device>,
}

impl OpenRgbBackend {
    /// Connects and enumerates controllers. OpenRGB listens on 127.0.0.1:6742 by default.
    pub fn connect(host: &str, port: u16) -> Result<Self, String> {
        Err(format!("OpenRGB client is not implemented yet ({host}:{port})"))
    }
}

impl Backend for OpenRgbBackend {
    fn name(&self) -> &str {
        "openrgb"
    }

    fn devices(&self) -> Vec<Device> {
        self.devices.clone()
    }

    fn push(&mut self, _device_id: &str, _colors: &[Color]) -> Result<(), String> {
        Ok(())
    }
}
