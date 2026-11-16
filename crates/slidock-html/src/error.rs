use slidock_canon::{JsonError, NumberError};
use slidock_format::ManifestError;
use slidock_format::paths::PathError;
use thiserror::Error;

/// Why a presentation cannot be printed: a value outside what the format writes. A verified IR never yields one.
#[derive(Clone, Debug, PartialEq, Error)]
pub enum HtmlError {
    #[error("{0}")]
    Number(#[from] NumberError),
    #[error("{0}")]
    Json(#[from] JsonError),
    #[error("{0}")]
    Manifest(#[from] ManifestError),
    #[error("{0}")]
    Path(#[from] PathError),
    /// The presentation does not read back as IR.
    #[error("{0}")]
    Ir(String),
    /// The text is not a deck JSON this crate reads.
    #[error("{0}")]
    Deck(String),
}

pub type Result<T, E = HtmlError> = std::result::Result<T, E>;
