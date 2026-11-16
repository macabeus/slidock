//! Media parts as IR media entries: each part read once, hashed as stored, typed by its bytes rather than its name or
//! content type, and sized from its header. The presentation holds hashes only; the store hands the bytes to the
//! writer: a range of the input for a STORED entry, the inflated bytes for a DEFLATE one.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::Arc;

use indexmap::IndexMap;
use slidock_core_api::lowering::LowerError;
use slidock_core_api::{MediaHandle, MediaSource, MediaStore};
use slidock_io::zip::EntryReader;
use slidock_io::{
    RangeReader, check_svg_source, probe_video, read_head, read_image_size, sha256_range,
    sniff_mime,
};
use slidock_ir::ids::Sha256;
use slidock_ir::media::{MediaEntry, Mime};
use slidock_ir::origin::Origin;

use crate::clip;
use crate::package::{Package, PartError};

/// Enough of a file for its type and its header's size.
const HEAD_BYTES: usize = 64 * 1024;

const OCTET_STREAM: &str = "application/octet-stream";

#[derive(Default)]
pub(crate) struct MediaRegistry {
    by_part: RefCell<BTreeMap<String, Result<Sha256, LowerError>>>,
    entries: RefCell<IndexMap<Sha256, MediaEntry>>,
    /// The bytes of each hash, from the part it was first read from.
    sources: RefCell<IndexMap<Sha256, MediaHandle>>,
    /// The length of each movie whose metadata states one, in milliseconds.
    durations: RefCell<BTreeMap<Sha256, f64>>,
}

fn read_failed(error: impl std::fmt::Display) -> LowerError {
    LowerError::Other(error.to_string())
}

impl MediaRegistry {
    /// The hash of a part's media entry, recorded on its first read; a part error when it is missing or
    /// unreadable.
    pub fn add(&self, pkg: &Package<'_>, part: &str) -> Result<Sha256, LowerError> {
        if let Some(read) = self.by_part.borrow().get(part) {
            return read.clone();
        }
        let read = self.read(pkg, part);
        self.by_part
            .borrow_mut()
            .insert(part.to_owned(), read.clone());
        read
    }

    fn read(&self, pkg: &Package<'_>, part: &str) -> Result<Sha256, LowerError> {
        let entry = pkg
            .entry(part)
            .ok_or_else(|| PartError::new(part, "missing"))?;
        let reader = pkg
            .archive()
            .open_entry(entry)
            .map_err(|error| PartError::new(part, error.message))?;
        let size = reader.len();
        let head = read_head(&reader, HEAD_BYTES)
            .map_err(read_failed)?
            .into_owned();
        let hex = sha256_range(&reader, 0, size).map_err(read_failed)?;
        let sha256 = Sha256::parse(&hex).map_err(read_failed)?;
        let mime = sniff_mime(&head);
        let mime_text = mime.map_or(OCTET_STREAM, |mime| mime.as_str());
        let image = match mime {
            Some(mime) => read_image_size(&head, mime, &reader).map_err(read_failed)?,
            None => None,
        };
        let video = if mime_text.starts_with("video/") {
            Some(probe_video(&reader).map_err(read_failed)?)
        } else {
            None
        };
        if self.entries.borrow().contains_key(&sha256) {
            return Ok(sha256);
        }
        if let Some(duration) = video.as_ref().and_then(|facts| facts.duration_ms) {
            self.durations.borrow_mut().insert(sha256, duration);
        }
        let entry = MediaEntry {
            sha256,
            mime: Mime::new(mime_text).map_err(read_failed)?,
            bytes: size,
            width: image.as_ref().map(|size| u64::from(size.width)),
            height: image.as_ref().map(|size| u64::from(size.height)),
            frames: image
                .as_ref()
                .and_then(|size| size.frames)
                .filter(|frames| *frames != 0)
                .map(u64::from),
            codec: video
                .and_then(|facts| facts.codec)
                .filter(|codec| !codec.is_empty())
                .map(|codec| clip(&codec, 32)),
            duration_ms: None,
            origin: Origin::source(part),
            plan: None,
        };
        let source = match reader {
            EntryReader::Slice(slice) => MediaSource::InputRange {
                offset: slice.offset(),
                len: size,
            },
            EntryReader::Memory(bytes) => {
                MediaSource::Bytes(Arc::from(bytes.into_inner().into_owned()))
            }
        };
        self.entries.borrow_mut().insert(sha256, entry);
        self.sources.borrow_mut().insert(
            sha256,
            MediaHandle {
                bytes: size,
                source,
            },
        );
        Ok(sha256)
    }

    /// Why a part is not an SVG the output shows (one that passes the allow-list), read without recording it; `None`
    /// when it is one.
    pub fn svg_refusal(&self, pkg: &Package<'_>, part: &str) -> Option<String> {
        let Some(entry) = pkg.entry(part) else {
            return Some("the part is missing".to_owned());
        };
        let reader = match pkg.archive().open_entry(entry) {
            Ok(reader) => reader,
            Err(error) => return Some(clip(&error.to_string(), 256)),
        };
        let head = match read_head(&reader, HEAD_BYTES) {
            Ok(head) => head,
            Err(error) => return Some(clip(&error.to_string(), 256)),
        };
        if sniff_mime(&head) != Some(slidock_io::Mime::Svg) {
            return Some("the part is not an SVG".to_owned());
        }
        match check_svg_source(&head, &reader) {
            Ok(Ok(_)) => None,
            Ok(Err(refusal)) => Some(refusal.to_string()),
            Err(error) => Some(clip(&error.to_string(), 256)),
        }
    }

    pub fn entry(&self, sha256: &Sha256) -> Option<MediaEntry> {
        self.entries.borrow().get(sha256).cloned()
    }

    pub fn duration(&self, sha256: &Sha256) -> Option<f64> {
        self.durations.borrow().get(sha256).copied()
    }

    /// The entries recorded, in the order they were first read.
    pub fn entries(&self) -> IndexMap<Sha256, MediaEntry> {
        self.entries.borrow().clone()
    }

    /// The store of the recorded media's bytes.
    pub fn store(self) -> PptxMedia {
        PptxMedia(self.sources.into_inner())
    }
}

/// The bytes of a PPTX's media, by hash.
#[derive(Clone, Debug, Default)]
pub struct PptxMedia(IndexMap<Sha256, MediaHandle>);

impl MediaStore for PptxMedia {
    fn open(&self, sha256: &Sha256) -> Option<MediaHandle> {
        self.0.get(sha256).cloned()
    }
}
