//! NVIDIA graphics card I2C through NVAPI. Port of OpenRGB i2c_smbus_nvapi.cpp.

use foxrgb_smbus::i2c::SharedBus;

pub fn buses() -> Vec<SharedBus> {
    Vec::new()
}
