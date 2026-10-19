use std::fmt;

/// Everything that can go wrong, worded so it can be shown to the user as is.
#[derive(Debug)]
pub enum Error {
    /// The transcript or caption file could not be read or understood.
    Input(String),
    /// The model server could not be reached or refused the request.
    Server(String),
    /// The model answered, but not with something usable.
    Model(String),
    /// A file could not be read or written.
    Io(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Input(m) => write!(f, "transcript problem: {m}"),
            Error::Server(m) => write!(f, "model server problem: {m}"),
            Error::Model(m) => write!(f, "model answer problem: {m}"),
            Error::Io(e) => write!(f, "file problem: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
