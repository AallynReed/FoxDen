//! Macros for any keyboard or mouse: the model, plain JSON for scripts, and on Windows playback,
//! hotkeys and recording.

pub mod inputs;
pub mod keys;
pub mod model;
pub mod presets;
pub mod spec;

#[cfg(windows)]
pub mod win;

pub use model::{Entry, Hotkey, Input, Library, Macro, MediaKey, Mode, Step};
