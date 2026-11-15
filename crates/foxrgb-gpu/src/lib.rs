//! Graphics card lighting over the card's own I2C bus, through the AMD and NVIDIA drivers.

mod amd;
mod families;
mod nvidia;

use foxrgb_core::Backend;
use foxrgb_smbus::i2c::SharedBus;

/// The I2C buses of every graphics card the drivers expose.
pub fn buses() -> Vec<SharedBus> {
    let mut buses = amd::buses();
    buses.extend(nvidia::buses());
    buses
}

/// Every graphics card lighting device found.
pub fn probe() -> Vec<Box<dyn Backend>> {
    let mut found = Vec::new();
    for bus in buses() {
        for detect in families::all() {
            found.extend(detect(&bus));
        }
    }
    found
}
