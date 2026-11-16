//! SHA-256 digests as lowercase hex.

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::error::JsonError;
use crate::json::{Layout, to_canonical_json};

/// The SHA-256 of `data` as 64 lowercase hex digits (`sha256Hex` hashes a string's UTF-8 bytes).
pub fn sha256_hex(data: impl AsRef<[u8]>) -> String {
    hex(&Sha256::digest(data.as_ref()))
}

/// The SHA-256 hex of a value's compact canonical JSON (`sha256Canonical`).
pub fn sha256_canonical<T: Serialize + ?Sized>(value: &T) -> Result<String, JsonError> {
    Ok(sha256_hex(to_canonical_json(value, Layout::Compact)?))
}

/// An incremental SHA-256 that finishes as hex.
#[derive(Clone, Debug, Default)]
pub struct Sha256Hex(Sha256);

impl Sha256Hex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, data: impl AsRef<[u8]>) {
        self.0.update(data.as_ref());
    }

    pub fn finish(self) -> String {
        hex(&self.0.finalize())
    }
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        for nibble in [byte >> 4, byte & 0xf] {
            out.push(char::from(
                DIGITS.get(usize::from(nibble)).copied().unwrap_or(b'0'),
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digests_known_vectors() {
        assert_eq!(
            sha256_hex(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let mut hasher = Sha256Hex::new();
        hasher.update("a");
        hasher.update(b"bc");
        assert_eq!(hasher.finish(), sha256_hex("abc"));
    }

    #[test]
    fn hashes_the_compact_canonical_form() {
        let value = serde_json::json!({ "b": 1, "a": [0.5] });
        assert_eq!(
            sha256_canonical(&value).unwrap(),
            sha256_hex(r#"{"a":[0.5],"b":1}"#)
        );
    }
}
