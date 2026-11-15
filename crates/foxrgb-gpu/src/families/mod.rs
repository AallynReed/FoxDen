//! Graphics card families ported from OpenRGB, each in its own folder.

use foxrgb_smbus::i2c::Detect;

mod a;
mod b;

pub fn known(ids: [u16; 4]) -> bool {
    a::known(ids) || b::known(ids)
}

pub fn all() -> Vec<Detect> {
    [a::all(), b::all()].concat()
}
