use std::fmt;

#[derive(Debug, Clone)]
pub enum Error {
    /// SteelSeries GG, Sonar or Engine isn't running or didn't answer.
    Offline(String),
    /// The service answered with an error status.
    Status { code: u16, body: String },
    /// The answer wasn't what the library expected.
    Parse(String),
    /// The address isn't on this computer; the library only talks to loopback.
    NotLoopback(String),
    /// A value outside what the service accepts.
    Invalid(String),
    /// A local audio or USB error.
    Device(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Offline(m) => write!(f, "not reachable: {m}"),
            Error::Status { code, body } => write!(f, "HTTP {code}: {body}"),
            Error::Parse(m) => write!(f, "unexpected answer: {m}"),
            Error::NotLoopback(a) => write!(f, "refusing non-loopback address {a}"),
            Error::Invalid(m) => write!(f, "invalid value: {m}"),
            Error::Device(m) => write!(f, "device error: {m}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
