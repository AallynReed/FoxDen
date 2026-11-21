use serde::{Deserialize, Serialize};

/// The mixer's channels. The names are the ones saved configs and the UI use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Channel {
    Game,
    ChatRender,
    ChatCapture,
    Media,
    Aux,
}

impl Channel {
    /// The order levels and settings are kept in.
    pub const ALL: [Channel; 5] = [
        Channel::Game,
        Channel::ChatRender,
        Channel::Media,
        Channel::Aux,
        Channel::ChatCapture,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Channel::Game => "Game",
            Channel::ChatRender => "Chat",
            Channel::ChatCapture => "Mic",
            Channel::Media => "Media",
            Channel::Aux => "Aux",
        }
    }

    pub fn is_capture(self) -> bool {
        self == Channel::ChatCapture
    }

    pub fn index(self) -> usize {
        match self {
            Channel::Game => 0,
            Channel::ChatRender => 1,
            Channel::Media => 2,
            Channel::Aux => 3,
            Channel::ChatCapture => 4,
        }
    }
}
