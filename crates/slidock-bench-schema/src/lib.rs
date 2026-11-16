//! The bench's persisted data model, with TypeScript types exported through `ts-rs` for the bench UI:
//!
//! - [`corpus`]: the corpus manifest (`slidock-corpus@1`);
//! - [`results`]: `results.json` (`slidock-bench-results@1`): the [`stamp`], one record per row with its outcome,
//!   scores and per-lane metrics ([`lanes`]), and the summaries;
//! - [`runs`]: `runs.json`, the index of runs the views list and compare;
//! - [`compare`]: the `regression` and `diff` reports;
//! - [`parity`]: the golden reference (`slidock-parity-ref@2`), the parity report (levels B and C) and the
//!   allowlist;
//! - [`api`]: the wire types of `slidock-bench ui`'s read-only API.
//!
//! Every file is written as canonical JSON ([`to_canonical`]): keys sorted by UTF-16 code unit, numbers as
//! JavaScript prints them, two-space indent and a trailing newline. Objects are strict: an unknown key fails the
//! parse, as zod's `strictObject` does.

pub mod api;
pub mod compare;
pub mod corpus;
mod de;
mod error;
pub mod ids;
pub mod lanes;
mod literal;
pub mod parity;
pub mod results;
pub mod runs;
pub mod stamp;
pub mod typescript;

pub use error::{Error, Issues};
pub use ids::{Frontend, Lane, PairKey, PresentationId, RowId, RowStep, Sha256, SlideId, Tier};
pub use literal::True;

use serde::Serialize;
use serde::de::DeserializeOwned;
use slidock_canon::Layout;

/// The canonical text of a bench file: sorted keys, two-space indent, trailing newline.
pub fn to_canonical<T: Serialize + ?Sized>(value: &T) -> Result<String, Error> {
    Ok(slidock_canon::to_canonical_json(value, Layout::Indented)?)
}

/// The SHA-256 of a value's compact canonical JSON: stamps, run keys, digests.
pub fn sha256_json<T: Serialize + ?Sized>(value: &T) -> Result<Sha256, Error> {
    Ok(Sha256::of(slidock_canon::to_canonical_json(
        value,
        Layout::Compact,
    )?))
}

/// Parses a bench file, naming the JSON path of the first value that does not fit.
pub fn parse<T: DeserializeOwned>(text: &str) -> Result<T, Error> {
    let mut deserializer = serde_json::Deserializer::from_str(text);
    let value =
        serde_path_to_error::deserialize(&mut deserializer).map_err(|error| Error::Parse {
            path: error.path().to_string(),
            message: error.inner().to_string(),
        })?;
    deserializer.end().map_err(|error| Error::Parse {
        path: ".".to_owned(),
        message: error.to_string(),
    })?;
    Ok(value)
}
