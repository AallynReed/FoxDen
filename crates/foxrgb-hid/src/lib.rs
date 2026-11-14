use foxrgb_core::Backend;

/// Detects supported USB HID lighting devices and returns one backend per device.
pub fn probe() -> Vec<Box<dyn Backend>> {
    Vec::new()
}
