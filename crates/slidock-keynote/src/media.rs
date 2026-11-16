//! The package's data files (images, movies, posters) as IR media entries, by the data ids the object graph uses.
//! Each file the lowered containers reference is read once: typed by its bytes whatever its name, sized from its
//! header and hashed as stored, streaming, so a gigabyte of movies stays out of memory. The package's own digests are
//! never trusted. The reads go through a view of the package apart from the index's, so each is measured on its own.
//! A file the package cannot give reads as missing.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use indexmap::IndexMap;
use slidock_core_api::{HOST_DECODED, MediaHandle, MediaSource, MediaStore};
use slidock_io::zip::EntryReader;
use slidock_io::{
    Mime, RangeReader, SvgRefusal, check_svg_source, read_head, read_image_size, sha256_range,
    sniff_mime,
};
use slidock_ir::ids::Sha256;
use slidock_ir::media::{MediaEntry, Mime as IrMime};
use slidock_ir::origin::Origin;

use crate::context::{DataId, MediaLookup};
use crate::graph::ObjectGraph;
use crate::iwa::Archive;
use crate::iwa::schema::{FieldKind, MESSAGES, WireReader, message_index};
use crate::package::KeynotePackage;
use crate::proto::tsp;

/// Enough of a file for its type and its header's size.
const HEAD_BYTES: usize = 64 * 1024;

/// What the output can show as an image, directly or transcoded.
const IMAGE_TYPES: [Mime; 7] = [
    Mime::Png,
    Mime::Jpeg,
    Mime::Gif,
    Mime::Webp,
    Mime::Avif,
    Mime::Tiff,
    Mime::Bmp,
];

const DATA_DIR: &str = "Data/";
const OCTET_STREAM: &str = "application/octet-stream";

/// The package path of each data id, from the package metadata's table.
pub fn data_paths(graph: &ObjectGraph) -> BTreeMap<DataId, String> {
    let mut paths = BTreeMap::new();
    for (_, metadata, _) in graph.of_type::<tsp::PackageMetadata>() {
        for data in &metadata.datas {
            let name = data
                .file_name
                .as_deref()
                .or(data.preferred_file_name.as_deref());
            let (Some(identifier), Some(name)) = (data.identifier, name) else {
                continue;
            };
            if name.is_empty() || name.contains('/') {
                continue;
            }
            paths
                .entry(DataId(identifier))
                .or_insert_with(|| format!("{DATA_DIR}{name}"));
        }
    }
    paths
}

/// The fields that point at a data file a slide can show.
const MEDIA_FIELDS: [&str; 8] = [
    "data",
    "adjustedImageData",
    "originalSVGData",
    "thumbnailData",
    "movieData",
    "posterImageData",
    "audioOnlyImageData",
    "imagedata",
];

/// How deep the walk for media fields goes, counting a repeated field's list as a level.
const MAX_DEPTH: usize = 32;

/// A message's fields, by number, from its wire bytes: each singular field's last value, each repeated field's values.
fn fields(index: usize, bytes: &[u8]) -> Option<BTreeMap<u32, Vec<&[u8]>>> {
    let schema = MESSAGES.get(index)?;
    let mut out: BTreeMap<u32, Vec<&[u8]>> = BTreeMap::new();
    let mut reader = WireReader::new(bytes);
    while !reader.done() {
        let tag = reader.uint32().ok()?;
        let (number, wire_type) = (tag >> 3, tag & 7);
        let field = schema.field(number)?;
        let body: &[u8] = match wire_type {
            0 => {
                reader.varint64().ok()?;
                &[]
            }
            1 => reader.take(8).ok()?,
            5 => reader.take(4).ok()?,
            2 => {
                let length = reader.uint32().ok()?;
                reader.take(length).ok()?
            }
            _ => return None,
        };
        let values = out.entry(number).or_default();
        if !field.repeated {
            values.clear();
        }
        values.push(body);
    }
    Some(out)
}

/// A reference message's `identifier`, when set.
fn identifier(index: usize, bytes: &[u8]) -> Option<u64> {
    let schema = MESSAGES.get(index)?;
    let number = schema
        .fields
        .iter()
        .find(|field| field.name == "identifier" && field.kind == FieldKind::Uint64)?
        .number;
    let mut reader = WireReader::new(bytes);
    let mut found = None;
    while !reader.done() {
        let tag = reader.uint32().ok()?;
        match tag & 7 {
            0 => {
                let value = reader.varint64().ok()?;
                if tag >> 3 == number {
                    found = Some(value);
                }
            }
            1 => {
                reader.take(8).ok()?;
            }
            5 => {
                reader.take(4).ok()?;
            }
            2 => {
                let length = reader.uint32().ok()?;
                reader.take(length).ok()?;
            }
            _ => return None,
        }
    }
    found
}

/// Data ids under media fields anywhere in a decoded message (fills sit deep inside styles).
fn collect_data(index: usize, bytes: &[u8], depth: usize, out: &mut BTreeSet<u64>) {
    if depth > MAX_DEPTH {
        return;
    }
    let (Some(schema), Some(fields)) = (MESSAGES.get(index), fields(index, bytes)) else {
        return;
    };
    for (number, values) in fields {
        let Some(field) = schema.field(number) else {
            continue;
        };
        let FieldKind::Message(child) = field.kind else {
            continue;
        };
        if field.repeated {
            // The list is a level of its own.
            if depth + 1 > MAX_DEPTH {
                continue;
            }
            for value in values {
                collect_data(child, value, depth + 2, out);
            }
            continue;
        }
        for value in values {
            match identifier(child, value) {
                Some(id) if MEDIA_FIELDS.contains(&field.name) => {
                    out.insert(id);
                }
                _ => collect_data(child, value, depth + 1, out),
            }
        }
    }
}

/// The data ids an object's message names under media fields.
fn media_data_of(archive: &Archive, out: &mut BTreeSet<u64>) {
    if let Some(index) = message_index(archive.type_name()) {
        collect_data(index, &archive.encode_to_vec(), 0, out);
    }
}

/// The bytes of every data file the index refers to as media, each file once: at least the media the presentation
/// shows (`Analysis::media_bytes`). Slide thumbnails are left out.
pub fn media_bytes<R: RangeReader>(pkg: &KeynotePackage<R>, graph: &ObjectGraph) -> u64 {
    let paths = data_paths(graph);
    let mut used = BTreeSet::new();
    for archive in graph.archives() {
        media_data_of(archive, &mut used);
    }
    let files: BTreeSet<&String> = used
        .into_iter()
        .filter_map(|id| paths.get(&DataId(id)))
        .collect();
    files
        .into_iter()
        .map(|path| pkg.entry(path).map_or(0, |entry| entry.size))
        .fold(0, u64::saturating_add)
}

/// The media the lowered containers reference, read through a view of the package of its own.
pub struct MediaRegistry<'a> {
    pkg: KeynotePackage<&'a dyn RangeReader>,
    paths: BTreeMap<DataId, String>,
    heads: BTreeMap<String, Option<Vec<u8>>>,
    by_path: BTreeMap<String, Option<Sha256>>,
    /// The path each hash was first read from; every path with those bytes is the same media.
    path_of: IndexMap<Sha256, String>,
    pub entries: IndexMap<Sha256, MediaEntry>,
}

impl std::fmt::Debug for MediaRegistry<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaRegistry")
            .field("paths", &self.paths.len())
            .field("entries", &self.entries.len())
            .finish_non_exhaustive()
    }
}

impl<'a> MediaRegistry<'a> {
    pub fn new(pkg: KeynotePackage<&'a dyn RangeReader>, graph: &ObjectGraph) -> Self {
        MediaRegistry {
            pkg,
            paths: data_paths(graph),
            heads: BTreeMap::new(),
            by_path: BTreeMap::new(),
            path_of: IndexMap::new(),
            entries: IndexMap::new(),
        }
    }

    fn path(&self, data: Option<DataId>) -> Option<String> {
        let path = self.paths.get(&data?)?;
        self.pkg.entry(path).is_some().then(|| path.clone())
    }

    fn open(&self, path: &str) -> Option<EntryReader<'_, &'a dyn RangeReader>> {
        self.pkg.open_entry(path)?.ok()
    }

    fn head(&mut self, path: &str) -> Option<&[u8]> {
        if !self.heads.contains_key(path) {
            let head = self.open(path).and_then(|reader| {
                read_head(&reader, HEAD_BYTES)
                    .ok()
                    .map(|head| head.into_owned())
            });
            self.heads.insert(path.to_owned(), head);
        }
        self.heads.get(path).and_then(Option::as_deref)
    }

    fn read(&mut self, path: &str) -> Option<MediaEntry> {
        let head = self.head(path)?.to_vec();
        let reader = self.open(path)?;
        let sha256 = sha256_range(&reader, 0, reader.len()).ok()?;
        let sha256 = Sha256::parse(&sha256).ok()?;
        let mime = sniff_mime(&head);
        let size = mime.and_then(|mime| read_image_size(&head, mime, &reader).ok().flatten());
        let codec = mime
            .filter(|mime| mime.as_str().starts_with("video/"))
            .and_then(|_| slidock_io::probe_video(&reader).ok())
            .and_then(|facts| facts.codec)
            .map(|codec| slidock_canon::utf16::clip(&codec, 32).to_owned());
        let mime = IrMime::new(mime.map_or(OCTET_STREAM, |mime| mime.as_str())).ok()?;
        Some(MediaEntry {
            sha256,
            mime,
            bytes: reader.len(),
            width: size.map(|size| u64::from(size.width)),
            height: size.map(|size| u64::from(size.height)),
            frames: size
                .and_then(|size| size.frames)
                .filter(|frames| *frames > 0)
                .map(u64::from),
            codec,
            duration_ms: None,
            origin: Origin::source(path),
            plan: None,
        })
    }

    /// The bytes of the recorded media, read again from the input when the writer asks.
    pub fn store(&self) -> KeynoteMedia {
        let mut handles = IndexMap::new();
        for (sha256, path) in &self.path_of {
            let Some(reader) = self.open(path) else {
                continue;
            };
            let bytes = reader.len();
            let source = match reader {
                EntryReader::Slice(slice) => MediaSource::InputRange {
                    offset: slice.offset(),
                    len: bytes,
                },
                EntryReader::Memory(memory) => {
                    let Ok(data) = memory.read_at(0, memory.len() as usize) else {
                        continue;
                    };
                    MediaSource::Bytes(Arc::from(data.into_owned()))
                }
            };
            handles.insert(*sha256, MediaHandle { bytes, source });
        }
        KeynoteMedia { handles }
    }
}

impl MediaLookup for MediaRegistry<'_> {
    fn is_image(&mut self, data: Option<DataId>) -> bool {
        let Some(path) = self.path(data) else {
            return false;
        };
        let Some(head) = self.head(&path).map(<[u8]>::to_vec) else {
            return false;
        };
        match sniff_mime(&head) {
            // Sized only when the whole document passes the allow-list.
            Some(Mime::Svg) => self.open(&path).is_some_and(|reader| {
                read_image_size(&head, Mime::Svg, &reader).is_ok_and(|size| size.is_some())
            }),
            Some(mime) => IMAGE_TYPES.contains(&mime),
            None => false,
        }
    }

    fn host_decodes(&mut self, data: Option<DataId>) -> bool {
        let Some(path) = self.path(data) else {
            return false;
        };
        self.head(&path)
            .and_then(sniff_mime)
            .is_some_and(|mime| HOST_DECODED.contains(&mime.as_str()))
    }

    fn svg_refusal(&mut self, data: Option<DataId>) -> Option<SvgRefusal> {
        let path = self.path(data)?;
        let head = self.head(&path)?.to_vec();
        if sniff_mime(&head) != Some(Mime::Svg) {
            return None;
        }
        let reader = self.open(&path)?;
        check_svg_source(&head, &reader).ok()?.err()
    }

    fn media(&mut self, data: Option<DataId>) -> Option<Sha256> {
        let path = self.path(data)?;
        if let Some(read) = self.by_path.get(&path) {
            return *read;
        }
        let read = self.read(&path).map(|entry| {
            let sha256 = entry.sha256;
            if !self.path_of.contains_key(&sha256) {
                self.path_of.insert(sha256, path.clone());
                self.entries.insert(sha256, entry);
            }
            sha256
        });
        self.by_path.insert(path, read);
        read
    }
}

/// The Keynote frontend's media store: each recorded item's bytes, a range of the input when the package stores it.
#[derive(Clone, Debug, Default)]
pub struct KeynoteMedia {
    handles: IndexMap<Sha256, MediaHandle>,
}

impl MediaStore for KeynoteMedia {
    fn open(&self, sha256: &Sha256) -> Option<MediaHandle> {
        self.handles.get(sha256).cloned()
    }
}
