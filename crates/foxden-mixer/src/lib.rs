//! FoxDen's audio mixer: channels with volume, mute, chat mix and EQ, fed by devices and by
//! single apps captured with Windows' process loopback.

pub mod capture;
pub mod channel;
pub mod chatmix;
pub mod dsp;
pub mod park;
pub mod router;
pub mod system;

pub use channel::Channel;

pub type Result<T> = std::result::Result<T, String>;
