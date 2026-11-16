//! IWA, iWork's archive format: Snappy-framed files ([`frame`]) holding streams of protobuf objects ([`stream`]),
//! decoded by the trimmed schema ([`schema`] and [`registry`]).

pub mod dump;
pub mod error;
pub mod frame;
pub mod registry;
pub mod schema;
pub mod stream;

pub use dump::dump;
pub use error::IwaError;
pub use frame::{decompress_snappy, snappy_length, unframe_iwa};
pub use registry::{Archive, FromArchive, is_kept, registry, type_id, type_name};
pub use schema::{DecodeLimit, MAX_DEPTH, WireError};
pub use stream::{IwaFile, IwaObject, IwaStats, ObjectId, read_iwa};
