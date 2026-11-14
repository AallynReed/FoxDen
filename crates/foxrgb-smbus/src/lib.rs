//! Lighting on the SMBus (memory modules), through the PawnIO driver. Needs administrator rights.

mod corsair;
mod pawnio;

use std::sync::{Arc, Mutex};

use foxrgb_core::Backend;

pub use pawnio::Bus;

/// Every SMBus lighting device found, or the reason the bus could not be opened.
pub fn probe() -> Result<Vec<Box<dyn Backend>>, String> {
    let bus = Arc::new(Mutex::new(Bus::open()?));
    Ok(corsair::probe(bus))
}
