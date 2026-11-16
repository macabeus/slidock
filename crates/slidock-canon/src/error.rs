use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Error)]
pub enum NumberError {
    #[error("non-finite number: {0}")]
    NonFinite(f64),
    #[error("number needs e-notation: {0}")]
    Exponent(String),
    #[error("toFixed takes 0 to 100 fraction digits, not {0}")]
    FixedDigits(u8),
    #[error("number too large for its grid: {0}")]
    OutOfRange(f64),
}

#[derive(Clone, Debug, PartialEq, Error)]
pub enum JsonError {
    #[error("in canonical JSON: {0}")]
    Number(#[from] NumberError),
    #[error("not representable in canonical JSON: {0}")]
    Unsupported(&'static str),
    #[error("canonical JSON object keys must be strings or integers")]
    KeyNotString,
    #[error("{0}")]
    Custom(String),
}

impl serde::ser::Error for JsonError {
    fn custom<T: std::fmt::Display>(msg: T) -> Self {
        JsonError::Custom(msg.to_string())
    }
}
