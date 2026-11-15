//! SMBus device families ported from OpenRGB, each in its own folder.

use crate::i2c::Detect;

mod boards;
mod dram;

pub fn all() -> Vec<Detect> {
    [dram::all(), boards::all()].concat()
}

pub fn gpu() -> Vec<Detect> {
    boards::gpu()
}

pub fn gpu_cards() -> Vec<[u16; 4]> {
    boards::gpu_cards()
}
