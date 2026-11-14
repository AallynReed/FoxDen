use foxrgb_fan::FanBackend;

/// Every read-only sensor source that works on this machine. None of them control anything.
pub fn backends() -> Vec<Box<dyn FanBackend>> {
    Vec::new()
}
