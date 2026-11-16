//! Node ids and media hashes. Every id is `<prefix>-<body>`, derived from the source's own identity so a re-import
//! diffs cleanly; ids share one namespace per presentation.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::Digest;

use crate::formats::IdPrefix;

const BODY_MAX: usize = 120;
const HASH_CHARS: usize = 8;

/// An id error.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum IdError {
    #[error("not a node id (^[a-z]{{2}}-[A-Za-z0-9._-]{{1,120}}$): {0:?}")]
    Invalid(String),
    #[error("a node id needs at least one part")]
    NoParts,
    #[error("not a SHA-256 (64 lowercase hex digits): {0:?}")]
    InvalidSha256(String),
}

/// A node id: `^[a-z]{2}-[A-Za-z0-9._-]{1,120}$`. The body excludes `:`, so bench rows
/// `presentation:slide:frontend` parse.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(String);

pub type SlideId = NodeId;
pub type LayoutId = NodeId;
pub type MasterId = NodeId;
pub type ThemeId = NodeId;
pub type ElementId = NodeId;

fn is_body_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-'
}

/// Whether `text` matches the id pattern.
pub fn is_node_id(text: &str) -> bool {
    let bytes = text.as_bytes();
    match bytes {
        [a, b, b'-', body @ ..] => {
            a.is_ascii_lowercase()
                && b.is_ascii_lowercase()
                && !body.is_empty()
                && body.len() <= BODY_MAX
                && body.iter().copied().all(is_body_char)
        }
        _ => false,
    }
}

impl NodeId {
    pub fn new(text: impl Into<String>) -> Result<NodeId, IdError> {
        let text = text.into();
        if is_node_id(&text) {
            Ok(NodeId(text))
        } else {
            Err(IdError::Invalid(text))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The two-letter prefix (`pp`).
    pub fn prefix(&self) -> &str {
        self.0.split_once('-').map_or("", |(prefix, _)| prefix)
    }

    /// The body after the prefix.
    pub fn body(&self) -> &str {
        self.0.split_once('-').map_or("", |(_, body)| body)
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for NodeId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::borrow::Borrow<str> for NodeId {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl Serialize for NodeId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for NodeId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        NodeId::new(text).map_err(serde::de::Error::custom)
    }
}

/// The first 8 hex digits of the SHA-256 of `text`'s UTF-8 bytes.
fn short_hash(text: &str) -> String {
    let mut hex = slidock_canon::sha256_hex(text.as_bytes());
    hex.truncate(HASH_CHARS);
    hex
}

/// A native id as an id part: each UTF-16 code unit outside `[A-Za-z0-9._-]` becomes `_`, and the first 8 hex of
/// the original's SHA-256 are appended whenever anything was replaced (or the part is empty), so each native id
/// escapes to its own part.
pub fn escape_id_part(part: &str) -> String {
    let mut safe = String::with_capacity(part.len());
    let mut replaced = false;
    for c in part.chars() {
        if c.is_ascii() && is_body_char(c as u8) {
            safe.push(c);
        } else {
            replaced = true;
            for _ in 0..c.len_utf16() {
                safe.push('_');
            }
        }
    }
    if !replaced && !part.is_empty() {
        safe
    } else {
        format!("{safe}.{}", short_hash(part))
    }
}

/// A body longer than the limit keeps its head and gains the hash of the whole body. Bodies are ASCII here, so
/// bytes and UTF-16 code units agree.
fn fit_body(body: String) -> String {
    if body.len() <= BODY_MAX {
        return body;
    }
    let head: String = body.chars().take(BODY_MAX - HASH_CHARS - 1).collect();
    format!("{head}.{}", short_hash(&body))
}

/// One native identity part of an id: a string or an integer (`String(part)`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdPart<'a> {
    Str(&'a str),
    Int(i64),
}

impl fmt::Display for IdPart<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IdPart::Str(s) => f.write_str(s),
            IdPart::Int(n) => write!(f, "{n}"),
        }
    }
}

impl<'a> From<&'a str> for IdPart<'a> {
    fn from(part: &'a str) -> Self {
        IdPart::Str(part)
    }
}

impl<'a> From<&'a String> for IdPart<'a> {
    fn from(part: &'a String) -> Self {
        IdPart::Str(part)
    }
}

macro_rules! int_part {
    ($($ty:ty),*) => {
        $(impl From<$ty> for IdPart<'_> {
            fn from(part: $ty) -> Self {
                IdPart::Int(part as i64)
            }
        })*
    };
}
int_part!(u8, u16, u32, i32, i64, usize);

/// The id of a node from its format prefix and native identity parts, joined by `.`.
pub fn node_id(prefix: IdPrefix, parts: &[IdPart<'_>]) -> Result<NodeId, IdError> {
    if parts.is_empty() {
        return Err(IdError::NoParts);
    }
    let escaped: Vec<String> = parts
        .iter()
        .map(|part| escape_id_part(&part.to_string()))
        .collect();
    NodeId::new(format!(
        "{}-{}",
        prefix.as_str(),
        fit_body(escaped.join("."))
    ))
}

/// The id of content a frontend materialises on `owner` from an inherited node (`od-p3.m.12`): owner-scoped, so
/// the copy on every slide gets its own id, and deterministic.
pub fn derived_id(owner: &NodeId, key: IdPart<'_>) -> Result<NodeId, IdError> {
    let body = format!("{}.m.{}", owner.body(), escape_id_part(&key.to_string()));
    NodeId::new(format!("{}-{}", owner.prefix(), fit_body(body)))
}

/// A SHA-256 digest: media are keyed by the hash of their bytes as stored in the source. Serialises as 64
/// lowercase hex digits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Sha256(pub [u8; 32]);

impl Sha256 {
    /// The digest of `bytes`.
    pub fn of(bytes: impl AsRef<[u8]>) -> Sha256 {
        Sha256(sha2::Sha256::digest(bytes.as_ref()).into())
    }

    pub fn parse(hex: &str) -> Result<Sha256, IdError> {
        let invalid = || IdError::InvalidSha256(hex.to_owned());
        let bytes = hex.as_bytes();
        if bytes.len() != 64 {
            return Err(invalid());
        }
        let nibble = |b: u8| match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            _ => None,
        };
        let mut out = [0u8; 32];
        let (pairs, _) = bytes.as_chunks::<2>();
        for (slot, [hi, lo]) in out.iter_mut().zip(pairs) {
            *slot = (nibble(*hi).ok_or_else(invalid)? << 4) | nibble(*lo).ok_or_else(invalid)?;
        }
        Ok(Sha256(out))
    }

    pub fn to_hex(&self) -> String {
        self.to_string()
    }
}

impl fmt::Display for Sha256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

impl Serialize for Sha256 {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Sha256 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Sha256::parse(&text).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(prefix: IdPrefix, parts: &[IdPart<'_>]) -> String {
        node_id(prefix, parts).unwrap().to_string()
    }

    #[test]
    fn keeps_native_ids_that_need_no_escaping() {
        assert_eq!(id(IdPrefix::Pp, &[256u32.into()]), "pp-256");
        assert_eq!(
            id(IdPrefix::Pp, &[2_147_483_649i64.into(), 4u32.into()]),
            "pp-2147483649.4"
        );
        assert_eq!(id(IdPrefix::Kn, &["5213335".into()]), "kn-5213335");
    }

    #[test]
    fn escapes_and_appends_the_hash() {
        let hash = &slidock_canon::sha256_hex("Google Shape;163;p35")[..8];
        assert_eq!(
            escape_id_part("Google Shape;163;p35"),
            format!("Google_Shape_163_p35.{hash}")
        );
        assert_ne!(escape_id_part("a b"), escape_id_part("a_b"));
        assert!(is_node_id(&id(
            IdPrefix::Od,
            &["m".into(), "Title Slide".into()]
        )));
        assert!(is_node_id(&id(IdPrefix::Ir, &["".into()])));
        assert!(escape_id_part("é😀").starts_with("___."));
    }

    #[test]
    fn shortens_long_bodies_deterministically() {
        let long = id(IdPrefix::Od, &[IdPart::Str(&"x".repeat(300))]);
        assert!(is_node_id(&long));
        assert_eq!(long.len(), 3 + 120);
        assert_eq!(long, id(IdPrefix::Od, &[IdPart::Str(&"x".repeat(300))]));
        assert_ne!(long, id(IdPrefix::Od, &[IdPart::Str(&"x".repeat(301))]));
        assert_eq!(node_id(IdPrefix::Od, &[]), Err(IdError::NoParts));
    }

    #[test]
    fn derives_owner_scoped_ids() {
        let owner = NodeId::new("od-p3").unwrap();
        assert_eq!(
            derived_id(&owner, 12u32.into()).unwrap().as_str(),
            "od-p3.m.12"
        );
        assert_ne!(
            derived_id(&owner, 12u32.into()),
            derived_id(&NodeId::new("od-p4").unwrap(), 12u32.into())
        );
        assert!(derived_id(&owner, "page number".into()).is_ok());
    }

    #[test]
    fn validates_ids_and_hashes() {
        assert!(NodeId::new("pp-256").is_ok());
        for bad in ["p-1", "PP-1", "pp-", "pp-a:b", "pp_1"] {
            assert!(NodeId::new(bad).is_err(), "{bad}");
        }
        assert!(NodeId::new(format!("pp-{}", "a".repeat(121))).is_err());
        let sha = Sha256::of(b"abc");
        assert_eq!(
            sha.to_string(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(Sha256::parse(&sha.to_string()), Ok(sha));
        assert!(Sha256::parse(&"A".repeat(64)).is_err());
        assert!(Sha256::parse("abc").is_err());
    }
}
