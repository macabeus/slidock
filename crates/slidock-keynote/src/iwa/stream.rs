//! An IWA file's object stream: `varint length, TSP.ArchiveInfo, payload…` per object. ArchiveInfo carries the
//! object's id (what every `TSP.Reference` points at) and one `MessageInfo` per payload with its type id. The first
//! payload is the object; further ones are diffs against it.

use std::collections::BTreeMap;
use std::fmt;

use prost::Message as _;

use crate::iwa::error::{IwaError, corrupt};
use crate::iwa::registry::{Archive, DecodeFailure, decode, registered};
use crate::iwa::schema::{ARCHIVE_INFO, DecodeLimit, MESSAGES, WireError, WireReader, sanitize};
use crate::proto::tsp;

/// An object's id in the index, unique across its files.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectId(pub u64);

impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct IwaObject {
    pub id: ObjectId,
    /// The first payload's type; `None` when its MessageInfo states none.
    pub type_id: Option<u32>,
    /// The registered name, also for a type the schema does not decode.
    pub type_name: Option<&'static str>,
    /// Absent when the schema does not decode the type or the payload is damaged.
    pub archive: Option<Archive>,
}

/// What the decoder read and passed over.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IwaStats {
    pub objects: u64,
    pub decoded: u64,
    /// Objects of a registered type the trimmed schema leaves out.
    pub skipped: u64,
    /// Counts of type ids outside the registry (a newer Keynote than the schema); `None`, first, for a payload with
    /// no type.
    pub unknown_type_ids: BTreeMap<Option<u32>, u64>,
    /// Payloads of a decoded type that failed to decode.
    pub decode_errors: u64,
    /// iWork diff records (`should_merge`).
    pub merge_records: u64,
    /// Objects with payloads after the first, which are diffs against it.
    pub extra_payloads: u64,
    /// Payload messages decoded, nested ones included.
    pub messages: u64,
    /// What the decoded messages cost in memory ([`crate::iwa::schema::MessageSchema::cost`]), every object's
    /// header included: a header is decoded for every object, and the object stays in the graph.
    pub decoded_bytes: u64,
}

/// One file's objects. A `should_merge` record patches an object of the same id; it is returned apart, decoded from
/// its first payload, so the graph can keep the base it patches or, when there is none, take the record itself.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IwaFile {
    pub objects: Vec<IwaObject>,
    pub merges: Vec<IwaObject>,
}

/// The objects of one unframed IWA file. Unknown and undecodable objects are counted in `stats` and kept
/// undecoded; the payload messages decoded count against `limit.messages`, and their cost and every object header's
/// against `limit.bytes`, over every file (`stats` holds what earlier files took); past either the file is too
/// large.
pub fn read_iwa(
    data: &[u8],
    file: &str,
    stats: &mut IwaStats,
    limit: DecodeLimit,
) -> Result<IwaFile, IwaError> {
    let mut reader = WireReader::new(data);
    let mut out = IwaFile::default();
    while !reader.done() {
        let left = limit.bytes.saturating_sub(stats.decoded_bytes);
        let (info, cost) = match header(&mut reader, left) {
            Ok(header) => header,
            Err(WireError::TooManyBytes(_)) => return Err(too_many_bytes(file, limit)),
            Err(error) => {
                return Err(corrupt(format!(
                    "{file}: bad object header at {}: {error}",
                    reader.pos
                )));
            }
        };
        stats.decoded_bytes += cost;
        let id = info.identifier;
        let mut first = None;
        for message_info in &info.message_infos {
            let length = message_info.length.unwrap_or(0);
            let truncated = || {
                let id = id.map(|id| id.to_string()).unwrap_or_default();
                corrupt(format!("{file}: object {id} is truncated"))
            };
            if u64::from(length) > (data.len() - reader.pos) as u64 {
                return Err(truncated());
            }
            let payload = reader.take(length).map_err(|_| truncated())?;
            first.get_or_insert((message_info.r#type, payload));
        }
        let (Some((type_id, payload)), Some(id)) = (first, id) else {
            continue;
        };
        let id = ObjectId(id);
        if info.should_merge == Some(true) {
            stats.merge_records += 1;
            out.merges
                .push(decode_object(id, type_id, payload, file, stats, limit)?);
            continue;
        }
        if info.message_infos.len() > 1 {
            stats.extra_payloads += 1;
        }
        stats.objects += 1;
        out.objects
            .push(decode_object(id, type_id, payload, file, stats, limit)?);
    }
    Ok(out)
}

/// An object's header and its cost, at most `bytes`.
fn header(reader: &mut WireReader<'_>, bytes: u64) -> Result<(tsp::ArchiveInfo, u64), WireError> {
    let length = reader.uint32()?;
    let wire = reader.take(length)?;
    let limit = DecodeLimit {
        messages: u64::MAX,
        bytes,
    };
    let sanitized = sanitize(MESSAGES, ARCHIVE_INFO, wire, limit)?;
    // The sanitized bytes are canonical ArchiveInfo, which prost reads.
    let info = tsp::ArchiveInfo::decode(sanitized.bytes.as_slice())
        .map_err(|_| WireError::FieldPastEnd)?;
    Ok((info, sanitized.cost))
}

fn too_many_bytes(file: &str, limit: DecodeLimit) -> IwaError {
    IwaError::TooLarge(format!(
        "{file}: the index decodes to more than {} bytes",
        limit.bytes
    ))
}

fn decode_object(
    id: ObjectId,
    type_id: Option<u32>,
    payload: &[u8],
    file: &str,
    stats: &mut IwaStats,
    limit: DecodeLimit,
) -> Result<IwaObject, IwaError> {
    let mut object = IwaObject {
        id,
        type_id,
        type_name: None,
        archive: None,
    };
    let Some(row) = type_id.and_then(registered) else {
        *stats.unknown_type_ids.entry(type_id).or_default() += 1;
        return Ok(object);
    };
    object.type_name = Some(row.name);
    let Some(kept) = row.kept else {
        stats.skipped += 1;
        return Ok(object);
    };
    let left = DecodeLimit {
        messages: limit.messages.saturating_sub(stats.messages),
        bytes: limit.bytes.saturating_sub(stats.decoded_bytes),
    };
    match decode(&kept, payload, left) {
        Ok((archive, sanitized)) => {
            stats.messages += sanitized.messages;
            stats.decoded_bytes += sanitized.cost;
            stats.decoded += 1;
            object.archive = Some(archive);
        }
        Err(DecodeFailure::TooManyMessages) => {
            return Err(IwaError::TooLarge(format!(
                "{file}: the index holds more than {} messages",
                limit.messages
            )));
        }
        Err(DecodeFailure::TooManyBytes) => return Err(too_many_bytes(file, limit)),
        Err(DecodeFailure::Damaged) => stats.decode_errors += 1,
    }
    Ok(object)
}
