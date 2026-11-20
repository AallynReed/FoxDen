use crate::{Color, Device, DeviceKind};

/// A source of devices: native drivers, the OpenRGB bridge, or the virtual preview.
pub trait Backend: Send {
    fn name(&self) -> &str;
    fn devices(&self) -> Vec<Device>;
    /// `colors` has one entry per LED, in `Device::leds` order.
    fn push(&mut self, device_id: &str, colors: &[Color]) -> Result<(), String>;
    /// Called once the pushes of one frame are done, for backends that send frames in batches.
    fn flush(&mut self) -> Result<(), String> {
        Ok(())
    }
}

/// Preview-only devices laid out like the development machine.
#[derive(Default)]
pub struct VirtualBackend;

impl Backend for VirtualBackend {
    fn name(&self) -> &str {
        "virtual"
    }

    fn devices(&self) -> Vec<Device> {
        use DeviceKind::*;
        vec![
            Device::grid(
                "virtual:board",
                "Motherboard ARGB header",
                Motherboard,
                [0.06, 0.12, 0.52, 0.05],
                24,
                1,
            ),
            Device::grid(
                "virtual:gpu",
                "Graphics card",
                Gpu,
                [0.06, 0.30, 0.40, 0.05],
                12,
                1,
            ),
            Device::grid(
                "virtual:ram0",
                "RAM 1",
                Memory,
                [0.68, 0.10, 0.03, 0.36],
                1,
                10,
            ),
            Device::grid(
                "virtual:ram1",
                "RAM 2",
                Memory,
                [0.76, 0.10, 0.03, 0.36],
                1,
                10,
            ),
            Device::grid(
                "virtual:keyboard",
                "TKL keyboard",
                Keyboard,
                [0.06, 0.58, 0.54, 0.32],
                17,
                6,
            ),
            Device::grid(
                "virtual:mouse",
                "Mouse",
                Mouse,
                [0.70, 0.62, 0.06, 0.22],
                1,
                2,
            ),
        ]
    }

    fn push(&mut self, _device_id: &str, _colors: &[Color]) -> Result<(), String> {
        Ok(())
    }
}
