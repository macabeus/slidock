//! The type registry (`proto/registry.tsv`): a name for every type id Keynote writes, and for the types the trimmed
//! schema keeps, their [`Archive`] variant and decoder. Several ids share a message (5 and 6 are both
//! `KN.SlideArchive`).

use crate::iwa::schema::{DecodeLimit, MESSAGES, Sanitized, WireError, sanitize};

/// A message type an [`Archive`] can hold.
pub trait FromArchive: Sized + 'static {
    /// The registered name, `KN.SlideArchive`.
    const TYPE_NAME: &'static str;

    fn from_archive(archive: &Archive) -> Option<&Self>;
}

/// How a kept type decodes: its schema message and its `prost` decoder.
#[derive(Clone, Copy)]
pub(crate) struct Kept {
    pub schema: usize,
    pub decode: fn(&[u8]) -> Result<Archive, prost::DecodeError>,
}

#[derive(Clone, Copy)]
pub(crate) struct Registered {
    pub type_id: u32,
    pub name: &'static str,
    pub kept: Option<Kept>,
}

include!(concat!(env!("OUT_DIR"), "/archives.rs"));

pub(crate) fn registered(type_id: u32) -> Option<&'static Registered> {
    REGISTRY
        .binary_search_by_key(&type_id, |row| row.type_id)
        .ok()
        .and_then(|at| REGISTRY.get(at))
}

/// A registered type's message name, whether or not the schema decodes it.
pub fn type_name(type_id: u32) -> Option<&'static str> {
    registered(type_id).map(|row| row.name)
}

/// Every registered type id and its message name, by id.
pub fn registry() -> impl Iterator<Item = (u32, &'static str)> {
    REGISTRY.iter().map(|row| (row.type_id, row.name))
}

/// The smallest type id registered for a message name.
pub fn type_id(name: &str) -> Option<u32> {
    REGISTRY
        .iter()
        .find(|row| row.name == name)
        .map(|row| row.type_id)
}

/// Whether the trimmed schema decodes a type.
pub fn is_kept(type_id: u32) -> bool {
    registered(type_id).is_some_and(|row| row.kept.is_some())
}

/// Why a payload of a kept type does not become an [`Archive`].
#[derive(Debug)]
pub(crate) enum DecodeFailure {
    TooManyMessages,
    TooManyBytes,
    Damaged,
}

/// A kept type's payload decoded, with the messages it holds and their cost; `limit` caps both.
pub(crate) fn decode(
    kept: &Kept,
    payload: &[u8],
    limit: DecodeLimit,
) -> Result<(Archive, Sanitized), DecodeFailure> {
    let sanitized =
        sanitize(MESSAGES, kept.schema, payload, limit).map_err(|error| match error {
            WireError::TooManyMessages(_) => DecodeFailure::TooManyMessages,
            WireError::TooManyBytes(_) => DecodeFailure::TooManyBytes,
            _ => DecodeFailure::Damaged,
        })?;
    let archive = (kept.decode)(&sanitized.bytes).map_err(|_| DecodeFailure::Damaged)?;
    Ok((archive, sanitized))
}
