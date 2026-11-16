//! The two errors that end a command early, and everything else that can go wrong, with their exit codes: 0 ok, 1 a
//! failed check, 2 refused or bad usage, 3 nothing compared.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum BenchError {
    /// Bad arguments: exits 2 after the usage.
    #[error("{0}")]
    Usage(String),
    /// A request the bench refuses (a holdout id, a dirty tree, a missing prerequisite): exits 2.
    #[error("refused: {0}")]
    Refusal(String),
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Schema(#[from] slidock_bench_schema::Error),
    #[error(transparent)]
    Json(#[from] slidock_canon::JsonError),
    /// Anything else that stops a command: a tool that failed, an unreadable file.
    #[error("{0}")]
    Failed(String),
}

pub type Result<T> = std::result::Result<T, BenchError>;

impl BenchError {
    pub fn io(path: impl AsRef<Path>) -> impl FnOnce(std::io::Error) -> BenchError {
        let path = path.as_ref().to_path_buf();
        move |source| BenchError::Io { path, source }
    }

    pub fn failed(message: impl std::fmt::Display) -> BenchError {
        BenchError::Failed(message.to_string())
    }

    pub fn refused(message: impl std::fmt::Display) -> BenchError {
        BenchError::Refusal(message.to_string())
    }

    pub fn usage(message: impl std::fmt::Display) -> BenchError {
        BenchError::Usage(message.to_string())
    }

    pub fn exit_code(&self) -> Exit {
        match self {
            BenchError::Usage(_) | BenchError::Refusal(_) => Exit::Refused,
            _ => Exit::Failed,
        }
    }
}

/// A command's exit status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exit {
    Ok,
    /// A failed check: a regression, a difference, a broken oracle.
    Failed,
    /// Refused or bad usage.
    Refused,
    /// `diff` or `regression` with nothing compared; `budgets` on a loaded machine, its CPU times not judged.
    NothingCompared,
}

impl Exit {
    pub fn code(self) -> u8 {
        match self {
            Exit::Ok => 0,
            Exit::Failed => 1,
            Exit::Refused => 2,
            Exit::NothingCompared => 3,
        }
    }

    /// `Ok` when `ok`, else `Failed`.
    pub fn check(ok: bool) -> Exit {
        if ok { Exit::Ok } else { Exit::Failed }
    }
}

impl From<Exit> for ExitCode {
    fn from(exit: Exit) -> ExitCode {
        ExitCode::from(exit.code())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes() {
        assert_eq!(BenchError::usage("x").exit_code().code(), 2);
        assert_eq!(BenchError::refused("x").exit_code().code(), 2);
        assert_eq!(BenchError::failed("x").exit_code().code(), 1);
        assert_eq!(Exit::NothingCompared.code(), 3);
        assert_eq!(
            BenchError::refused("holdout").to_string(),
            "refused: holdout"
        );
    }
}
