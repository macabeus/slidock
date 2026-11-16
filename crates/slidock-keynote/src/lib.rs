//! The Keynote frontend: IWA archives (raw snappy chunks), the protobuf schema (`prost`), the object graph,
//! styles, text with UTF-16 offsets, drawables, tables and media, lowered to the IR. Builds and transitions keep
//! Keynote's effect ids with a canonical fallback, plus their native effects, timing and Magic Move keys
//! ([`animation`]).

pub mod animation;
pub mod cells;
pub mod context;
pub mod document;
pub mod drawables;
pub mod fonts;
pub mod frontend;
pub mod geometry;
pub mod graph;
pub mod ids;
pub mod iwa;
pub mod lower;
pub mod media;
pub mod num;
pub mod package;
pub mod paint;
pub mod paths;
pub mod proto;
pub mod styles;
pub mod table;
pub mod text;

pub use document::{KeynoteDocument, open_document, report_index};
pub use frontend::KeynoteFrontend;
pub use graph::{GraphIssue, IndexSource, IndexTooLarge, ObjectGraph, read_object_graph};
pub use iwa::{Archive, FromArchive, IwaStats, ObjectId};
pub use package::{DOCUMENT_PATH, KeynotePackage, is_keynote_package, package_root};
