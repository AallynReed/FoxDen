//! FoxDen's audio mixer: channels with volume, mute, chat mix and EQ, fed by devices and by
//! single apps captured with Windows' process loopback.

pub mod analysis;
pub mod capture;
pub mod chain;
pub mod channel;
pub mod chatmix;
pub mod denoise;
pub mod dsp;
pub mod dynamics;
pub mod park;
pub mod presets;
pub mod router;
pub mod system;
pub mod tune;
pub mod volumes;

pub use channel::Channel;

pub type Result<T> = std::result::Result<T, String>;
