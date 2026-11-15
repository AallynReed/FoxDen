//! SMBus device families ported from OpenRGB, each in its own folder.

use crate::i2c::Detect;

mod boards;
mod dram;

pub fn all() -> Vec<Detect> {
    [dram::all(), boards::all()].concat()
}
