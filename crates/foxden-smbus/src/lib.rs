//! Lighting on the SMBus (memory modules, motherboards), through the PawnIO driver.
//! Needs administrator rights, or the installed service.

mod corsair;
mod families;
pub mod i2c;
mod pawnio;

use std::sync::{Arc, Mutex};

use foxden_core::Backend;

pub use pawnio::Bus;

/// Every SMBus lighting device found on every bus, or the reason no bus could be opened.
pub fn probe() -> Result<Vec<Box<dyn Backend>>, String> {
    let mut found = Vec::new();
    // Corsair memory talks to the bus directly; the ported families go through the common interface.
    for (bus, own) in Bus::open_all()?.into_iter().zip(Bus::open_all()?) {
        found.extend(corsair::probe(Arc::new(Mutex::new(own))));
        let shared: i2c::SharedBus = Arc::new(Mutex::new(Box::new(bus) as Box<dyn i2c::SmBus>));
        for detect in families::all() {
            found.extend(detect(&shared));
        }
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
