use std::fmt;

/// Why a bench file could not be read, written or trusted.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The text is not JSON of the expected shape.
    #[error("{path}: {message}")]
    Parse { path: String, message: String },
    /// A value has no canonical JSON form (a non-finite number, a non-string key).
    #[error(transparent)]
    Json(#[from] slidock_canon::JsonError),
    /// A string is not a valid id or hash.
    #[error("{what}: {value:?} {rule}")]
    Id {
        what: &'static str,
        value: String,
        rule: &'static str,
    },
    /// The value parses but breaks a rule across fields (a repeated id, a holdout that stores rows, …).
    #[error("{0}")]
    Invalid(Issues),
}

/// Every rule a value breaks, in the order they were found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Issues(pub Vec<String>);

impl Issues {
    pub fn push(&mut self, issue: impl Into<String>) {
        self.0.push(issue.into());
    }

    /// `Ok` when no rule was broken.
    pub fn finish(self) -> Result<(), Error> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(Error::Invalid(self))
        }
    }
}

impl fmt::Display for Issues {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.join("; "))
    }
}
