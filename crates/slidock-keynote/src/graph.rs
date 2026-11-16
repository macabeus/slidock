//! Every object of a document's index, by id: references resolve across IWA files. Objects of one id keep the first
//! read (files in path order). A `should_merge` record whose base is missing from the index stands in for it (some
//! objects exist only this way); with a base, the base is kept and the record reported.

use std::borrow::Cow;

use indexmap::IndexMap;
use slidock_canon::utf16;
use thiserror::Error;

use crate::iwa::{
    Archive, DecodeLimit, FromArchive, IwaError, IwaObject, IwaStats, ObjectId, read_iwa,
    unframe_iwa,
};
use crate::proto::tsp::Reference;

const INDEX_DIR: &str = "Index/";
const IWA: &str = ".iwa";
/// The index's budget, for its files as read and as decompressed; a 200-slide presentation decompresses to about
/// 3 MiB.
pub const MAX_INDEX_BYTES: u64 = 32 * 1024 * 1024;
/// The messages the index may decode to, nested ones included: decoding multiplies bytes into objects (a 4-byte
/// reference becomes an object), so the byte budget alone does not bound memory. A 197-slide presentation decodes
/// 146,000.
pub const MAX_INDEX_MESSAGES: u64 = 2_000_000;
/// What the decoded index may cost in memory ([`crate::iwa::schema::MessageSchema::cost`]), the same on every target:
/// a decoded message is a struct of every field the schema keeps, so a few hostile bytes (an empty element of a
/// repeated field) can stand for a large one. The 197-slide corpus presentation costs about 18 MB.
pub const MAX_INDEX_DECODED_BYTES: u64 = 256 * 1024 * 1024;

/// What reading an index may take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexBudget {
    /// The index files, as declared and as decompressed.
    pub bytes: u64,
    /// The messages they decode to, nested ones included.
    pub messages: u64,
    /// What the decoded messages cost in memory.
    pub decoded_bytes: u64,
}

impl Default for IndexBudget {
    fn default() -> Self {
        IndexBudget {
            bytes: MAX_INDEX_BYTES,
            messages: MAX_INDEX_MESSAGES,
            decoded_bytes: MAX_INDEX_DECODED_BYTES,
        }
    }
}

/// The index as declared or decompressed passes its budget: the input is refused.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("{0}")]
pub struct IndexTooLarge(pub String);

/// What reading the index passed over, for the frontend to report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GraphIssue {
    MergeUnapplied { id: ObjectId, file: String },
    FileUnreadable { file: String, reason: String },
}

/// An object of the index and the file it was read from.
#[derive(Clone, Debug, PartialEq)]
pub struct Object {
    pub id: ObjectId,
    pub type_id: Option<u32>,
    /// The registered name, also for a type the schema does not decode.
    pub type_name: Option<&'static str>,
    /// Absent when the schema does not decode the type or the payload is damaged.
    pub archive: Option<Archive>,
    file: usize,
}

/// A decoded object a reference points at.
#[derive(Clone, Copy, Debug)]
pub struct Resolved<'g> {
    pub id: ObjectId,
    pub archive: &'g Archive,
    /// The index file it was read from.
    pub file: &'g str,
}

impl<'g> Resolved<'g> {
    /// The object as `T`, when it decoded as one.
    pub fn get<T: FromArchive>(&self) -> Option<&'g T> {
        T::from_archive(self.archive)
    }
}

#[derive(Clone, Debug, Default)]
pub struct ObjectGraph {
    objects: IndexMap<ObjectId, Object>,
    files: Vec<String>,
    pub stats: IwaStats,
    pub issues: Vec<GraphIssue>,
}

/// The object id a reference names.
pub fn referenced(reference: Option<&Reference>) -> Option<ObjectId> {
    reference.and_then(|r| r.identifier).map(ObjectId)
}

impl ObjectGraph {
    /// The object a reference points at, when it decoded as a `T`.
    pub fn get<T: FromArchive>(&self, reference: Option<&Reference>) -> Option<&T> {
        self.resolve(reference).and_then(|object| object.get::<T>())
    }

    /// The object of an id, when it decoded as a `T`.
    pub fn get_id<T: FromArchive>(&self, id: ObjectId) -> Option<&T> {
        self.resolve_id(id).and_then(|object| object.get::<T>())
    }

    /// The object a reference points at, when it decoded.
    pub fn resolve(&self, reference: Option<&Reference>) -> Option<Resolved<'_>> {
        referenced(reference).and_then(|id| self.resolve_id(id))
    }

    pub fn resolve_id(&self, id: ObjectId) -> Option<Resolved<'_>> {
        let object = self.objects.get(&id)?;
        // Every decoded object has a registered name.
        object.type_name?;
        Some(Resolved {
            id: object.id,
            archive: object.archive.as_ref()?,
            file: self.files.get(object.file)?,
        })
    }

    /// The registered type name of the object a reference points at, decoded or not.
    pub fn type_name(&self, reference: Option<&Reference>) -> Option<&'static str> {
        referenced(reference)
            .and_then(|id| self.objects.get(&id))
            .and_then(|object| object.type_name)
    }

    /// The index file an object was read from.
    pub fn file_of(&self, id: ObjectId) -> Option<&str> {
        self.objects
            .get(&id)
            .and_then(|object| self.files.get(object.file))
            .map(String::as_str)
    }

    /// Every object, decoded or not, in file order.
    pub fn objects(&self) -> impl Iterator<Item = &Object> {
        self.objects.values()
    }

    /// Every decoded object, in file order.
    pub fn archives(&self) -> impl Iterator<Item = &Archive> {
        self.objects
            .values()
            .filter_map(|object| object.archive.as_ref())
    }

    /// Every object decoded as a `T`, in file order.
    pub fn of_type<T: FromArchive>(&self) -> impl Iterator<Item = (ObjectId, &T, &str)> {
        self.objects.values().filter_map(|object| {
            let message = object.archive.as_ref().and_then(T::from_archive)?;
            let file = self.files.get(object.file)?;
            Some((object.id, message, file.as_str()))
        })
    }
}

/// Several index files' bytes, and why any could not be read.
#[derive(Debug, Default)]
pub struct ReadMany<'a> {
    pub read: IndexMap<String, Cow<'a, [u8]>>,
    pub failed: IndexMap<String, String>,
}

/// What the graph reads the index through: entry sizes, and several entries' bytes at once.
pub trait IndexSource {
    /// Every entry's path and declared (uncompressed) size.
    fn entry_sizes(&self) -> Vec<(&str, u64)>;

    fn read_many(&self, paths: &[&str]) -> ReadMany<'_>;
}

/// Decodes every IWA file under `Index/` within one budget: the files' declared sizes are checked against it before
/// any is read, and what they decompress to after. A damaged file is left out and reported; lowering reports the
/// objects it held as missing.
pub fn read_object_graph(
    source: &impl IndexSource,
    budget: &IndexBudget,
) -> Result<ObjectGraph, IndexTooLarge> {
    let limit = DecodeLimit {
        messages: budget.messages,
        bytes: budget.decoded_bytes,
    };
    let budget = budget.bytes;
    let mut entries: Vec<(&str, u64)> = source
        .entry_sizes()
        .into_iter()
        .filter(|(path, _)| path.starts_with(INDEX_DIR) && path.ends_with(IWA))
        .collect();
    entries.sort_by(|a, b| utf16::cmp(a.0, b.0));
    let declared = entries
        .iter()
        .fold(0u64, |sum, (_, size)| sum.saturating_add(*size));
    if declared > budget {
        return Err(IndexTooLarge(format!(
            "The index is {declared} bytes; at most {budget} are read"
        )));
    }
    let paths: Vec<&str> = entries.iter().map(|(path, _)| *path).collect();
    let ReadMany { mut read, failed } = source.read_many(&paths);
    let mut graph = ObjectGraph::default();
    let mut merges: Vec<(IwaObject, usize)> = Vec::new();
    let mut remaining = budget;
    for path in paths {
        let Some(bytes) = read.swap_remove(path) else {
            let reason = failed
                .get(path)
                .cloned()
                .unwrap_or_else(|| "not read".into());
            graph.issues.push(GraphIssue::FileUnreadable {
                file: path.to_owned(),
                reason,
            });
            continue;
        };
        let file = graph.files.len();
        graph.files.push(path.to_owned());
        let decoded = unframe_iwa(&bytes, path, remaining).and_then(|data| {
            remaining -= data.len() as u64;
            read_iwa(&data, path, &mut graph.stats, limit)
        });
        match decoded {
            Ok(iwa) => {
                for object in iwa.objects {
                    graph
                        .objects
                        .entry(object.id)
                        .or_insert_with(|| placed(object, file));
                }
                merges.extend(iwa.merges.into_iter().map(|merge| (merge, file)));
            }
            Err(IwaError::TooLarge(message)) => return Err(IndexTooLarge(message)),
            Err(IwaError::Corrupt(reason)) => graph.issues.push(GraphIssue::FileUnreadable {
                file: path.to_owned(),
                reason,
            }),
        }
    }
    for (merge, file) in merges {
        if graph.objects.contains_key(&merge.id) {
            graph.issues.push(GraphIssue::MergeUnapplied {
                id: merge.id,
                file: graph.files.get(file).cloned().unwrap_or_default(),
            });
        } else {
            graph.objects.insert(merge.id, placed(merge, file));
        }
    }
    Ok(graph)
}

fn placed(object: IwaObject, file: usize) -> Object {
    Object {
        id: object.id,
        type_id: object.type_id,
        type_name: object.type_name,
        archive: object.archive,
        file,
    }
}
