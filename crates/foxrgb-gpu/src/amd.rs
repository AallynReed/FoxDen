//! AMD graphics card I2C through ADL. Port of OpenRGB i2c_smbus_amdadl.cpp.

use foxrgb_smbus::i2c::SharedBus;

pub fn buses() -> Vec<SharedBus> {
    Vec::new()
}
