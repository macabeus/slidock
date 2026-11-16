//! Canonical JSON of IR values, printed by `slidock-canon`: keys sorted by UTF-16 code unit, JS number printing,
//! compact or indented with a trailing newline.

use serde::Serialize;
pub use slidock_canon::{JsonError, Layout};

/// The compact canonical form (cache keys, hashes, sizes).
pub fn to_json<T: Serialize + ?Sized>(value: &T) -> Result<String, JsonError> {
    slidock_canon::to_canonical_json(value, Layout::Compact)
}

/// The indented canonical form with a trailing newline (dumps, goldens).
pub fn to_json_pretty<T: Serialize + ?Sized>(value: &T) -> Result<String, JsonError> {
    slidock_canon::to_canonical_json(value, Layout::Indented)
}

/// A canonical dump of a presentation, as `compile --emit ir` writes it and the IR frontend reads it back: the
/// presentation with the IR's `$schema`, as indented canonical JSON.
pub fn dump_presentation<P: Serialize + ?Sized>(presentation: &P) -> Result<String, JsonError> {
    let mut value =
        serde_json::to_value(presentation).map_err(|error| JsonError::Custom(error.to_string()))?;
    if let serde_json::Value::Object(object) = &mut value {
        object.insert(
            "$schema".to_owned(),
            serde_json::Value::from(crate::presentation::SchemaUrl::WIRE),
        );
    }
    to_json_pretty(&value)
}

/// The SHA-256 hex of the compact canonical form.
pub fn sha256_canonical<T: Serialize + ?Sized>(value: &T) -> Result<String, JsonError> {
    slidock_canon::sha256_canonical(value)
}
