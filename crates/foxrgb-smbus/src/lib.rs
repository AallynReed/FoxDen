//! Lighting on the SMBus (memory modules, motherboards), through the PawnIO driver.
//! Needs administrator rights, or the installed service.

mod corsair;
mod families;
pub mod i2c;
mod pawnio;

use std::sync::{Arc, Mutex};

use foxrgb_core::Backend;

pub use pawnio::Bus;

/// Every SMBus lighting device found, or the reason the bus could not be opened.
pub fn probe() -> Result<Vec<Box<dyn Backend>>, String> {
    let shared = Arc::new(Mutex::new(Bus::open()?));
    let mut found = corsair::probe(shared.clone());
    // The ported families share the bus through the common interface.
    let bus: i2c::SharedBus = Arc::new(Mutex::new(Box::new(Bus::open()?) as Box<dyn i2c::SmBus>));
    for detect in families::all() {
        found.extend(detect(&bus));
    }
    Ok(found)
}

/// The SMBus families found on graphics cards, for the GPU crate to run on each card's bus.
pub fn gpu_families() -> Vec<i2c::Detect> {
    families::gpu()
}

/// PCI ids (vendor, device, subsystem vendor, subsystem device) of every card those families list.
pub fn gpu_cards() -> Vec<[u16; 4]> {
    families::gpu_cards()
}
