//! SteelSeries GG from Rust: Sonar (audio routing, volumes, chat mix, EQ), Engine (devices,
//! on-board settings, firmware) and GameSense (lighting events).
//!
//! Every call goes to services on this computer only. Start with [`Endpoints::discover`].

pub mod chatmix;
pub mod discovery;
pub mod dsp;
pub mod engine;
pub mod error;
pub mod gamesense;
pub mod http;
pub mod sonar;

#[cfg(feature = "audio")]
pub mod router;

#[cfg(feature = "hid")]
pub mod hid;

pub use discovery::{CoreProps, Endpoints};
pub use engine::EngineClient;
pub use error::{Error, Result};
pub use gamesense::GameSenseClient;
pub use sonar::{Channel, Mode, SonarClient, StreamMix};
