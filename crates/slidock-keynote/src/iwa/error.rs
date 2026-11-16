use thiserror::Error;

/// Why an index file fails to read: damaged bytes, or a decompressed size past what is left of the index budget.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum IwaError {
    /// The file is left out of the graph and reported.
    #[error("{0}")]
    Corrupt(String),
    /// The whole input is refused.
    #[error("{0}")]
    TooLarge(String),
}

impl IwaError {
    pub fn message(&self) -> &str {
        match self {
            IwaError::Corrupt(message) | IwaError::TooLarge(message) => message,
        }
    }
}

pub(crate) fn corrupt(message: impl Into<String>) -> IwaError {
    IwaError::Corrupt(message.into())
}
