//! Files the document references (pictures, fill images, media) as IR media entries: each package path read once,
//! hashed, typed by its bytes and sized from its header, whose stated resolution ODF crops are measured against. The
//! presentation holds hashes only; the store hands the bytes to the writer.

use std::collections::BTreeMap;
use std::sync::Arc;

use indexmap::IndexMap;
use slidock_canon::utf16;
use slidock_core_api::{MediaHandle, MediaSource, MediaStore};
use slidock_io::zip::EntryReader;
use slidock_io::{
    Mime, RangeReader, check_svg_source, probe_video, read_head, read_image_size, sha256_range,
    sniff_mime,
};
use slidock_ir::ids::Sha256;
use slidock_ir::media::{MediaEntry, Mime as MediaType};
use slidock_ir::origin::Origin;

use crate::package::{OdpPackage, PartError};

/// Enough of a file for its type and its header's size.
const HEAD_BYTES: usize = 64 * 1024;

const DEFAULT_DPI: f64 = 96.0;

const OCTET_STREAM: &str = "application/octet-stream";

/// A picture's size in points: its pixels at its stated resolution, else 96 dpi (as LibreOffice).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ImageExtent {
    pub w: f64,
    pub h: f64,
}

/// Why a package file is not media.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MediaError {
    /// Missing, or the zip cannot give its bytes.
    Part(PartError),
    /// The input failed to read a range inside it.
    Read(String),
}

struct Read {
    entry: MediaEntry,
    extent: Option<ImageExtent>,
    length: Option<f64>,
    source: MediaSource,
}

/// The media a presentation's containers record, each package path read once.
#[derive(Default)]
pub(crate) struct MediaRegistry {
    by_path: BTreeMap<String, Result<Sha256, MediaError>>,
    /// The path each hash was first read from; every path with those bytes is the same media.
    path_of: BTreeMap<Sha256, String>,
    extents: BTreeMap<Sha256, ImageExtent>,
    sources: BTreeMap<Sha256, MediaHandle>,
    /// A movie's length in milliseconds, from its `mvhd`; the entries leave it out, as the parity path's do.
    lengths: BTreeMap<Sha256, f64>,
    pub entries: IndexMap<Sha256, MediaEntry>,
}

impl MediaRegistry {
    /// The media entry of a package path; an error when it is missing or unreadable.
    pub(crate) fn add(&mut self, pkg: &OdpPackage<'_>, path: &str) -> Result<Sha256, MediaError> {
        if let Some(known) = self.by_path.get(path) {
            return known.clone();
        }
        let result = read(pkg, path).map(|read| {
            let sha256 = read.entry.sha256;
            if let std::collections::btree_map::Entry::Vacant(slot) = self.path_of.entry(sha256) {
                slot.insert(path.to_owned());
                self.sources.insert(
                    sha256,
                    MediaHandle {
                        bytes: read.entry.bytes,
                        source: read.source,
                    },
                );
                self.entries.insert(sha256, read.entry);
                if let Some(extent) = read.extent {
                    self.extents.insert(sha256, extent);
                }
                if let Some(length) = read.length {
                    self.lengths.insert(sha256, length);
                }
            }
            sha256
        });
        self.by_path.insert(path.to_owned(), result.clone());
        result
    }

    pub(crate) fn entry(&self, sha256: &Sha256) -> Option<&MediaEntry> {
        self.entries.get(sha256)
    }

    /// A movie's length in milliseconds, when its metadata gives one.
    pub(crate) fn length(&self, sha256: &Sha256) -> Option<f64> {
        self.lengths.get(sha256).copied()
    }

    /// The intrinsic size of a picture, when its header gives its pixels.
    pub(crate) fn extent(&self, sha256: &Sha256) -> Option<ImageExtent> {
        self.extents.get(sha256).copied()
    }

    /// The bytes of the recorded media, for the writer.
    pub(crate) fn store(&self) -> OdpMedia {
        OdpMedia {
            handles: self.sources.clone(),
        }
    }
}

/// Whether a package path holds an SVG, and whether the output shows it (it passes the allow-list), read without
/// recording it.
pub(crate) enum SvgCheck {
    Shown,
    /// An SVG the allow-list refuses, and why.
    Refused(String),
    /// Not an SVG, or not readable.
    Other,
}

pub(crate) fn check_svg_part(pkg: &OdpPackage<'_>, path: &str) -> SvgCheck {
    let Some(Ok(reader)) = pkg.entry(path).map(|entry| pkg.archive.open_entry(entry)) else {
        return SvgCheck::Other;
    };
    let Ok(head) = read_head(&reader, HEAD_BYTES) else {
        return SvgCheck::Other;
    };
    if sniff_mime(&head) != Some(Mime::Svg) {
        return SvgCheck::Other;
    }
    match check_svg_source(&head, &reader) {
        Ok(Ok(_)) => SvgCheck::Shown,
        Ok(Err(refusal)) => SvgCheck::Refused(refusal.to_string()),
        Err(error) => SvgCheck::Refused(error.to_string()),
    }
}

fn read(pkg: &OdpPackage<'_>, path: &str) -> Result<Read, MediaError> {
    let part = |reason: String| MediaError::Part(PartError::new(path, reason));
    let zip_entry = pkg
        .entry(path)
        .ok_or_else(|| part("missing".to_owned()))?
        .clone();
    let reader = pkg
        .archive
        .open_entry(&zip_entry)
        .map_err(|error| part(error.message))?;
    let failed = |error: slidock_io::RangeError| MediaError::Read(error.to_string());
    let size = reader.len();
    let head = read_head(&reader, HEAD_BYTES).map_err(failed)?;
    let sha256 = sha256_range(&reader, 0, size).map_err(failed)?;
    let sha256 = Sha256::parse(&sha256).map_err(|error| MediaError::Read(error.to_string()))?;
    let sniffed = sniff_mime(&head);
    let image = match sniffed {
        Some(mime) => read_image_size(&head, mime, &reader).map_err(failed)?,
        None => None,
    };
    let mime_text = sniffed.map_or(OCTET_STREAM, Mime::as_str);
    let (codec, length) = if mime_text.starts_with("video/") {
        let facts = probe_video(&reader).map_err(failed)?;
        (facts.codec, facts.duration_ms)
    } else {
        (None, None)
    };
    let mime = MediaType::new(mime_text).map_err(|error| MediaError::Read(error.to_string()))?;
    let entry = MediaEntry {
        sha256,
        mime,
        bytes: size,
        width: image.map(|size| u64::from(size.width)),
        height: image.map(|size| u64::from(size.height)),
        frames: image
            .and_then(|size| size.frames)
            .filter(|&frames| frames != 0)
            .map(u64::from),
        codec: codec.map(|codec| utf16::clip(&codec, 32).to_owned()),
        duration_ms: None,
        origin: Origin::source(path),
        plan: None,
    };
    let extent = image.map(|size| {
        let dpi = |stated: Option<f64>| {
            stated
                .filter(|&dpi| dpi != 0.0 && !dpi.is_nan())
                .unwrap_or(DEFAULT_DPI)
        };
        ImageExtent {
            w: (f64::from(size.width) / dpi(size.dpi.map(|d| d.x))) * 72.0,
            h: (f64::from(size.height) / dpi(size.dpi.map(|d| d.y))) * 72.0,
        }
    });
    let source = match reader {
        EntryReader::Slice(slice) => MediaSource::InputRange {
            offset: slice.offset(),
            len: size,
        },
        EntryReader::Memory(memory) => {
            MediaSource::Bytes(Arc::from(memory.into_inner().into_owned()))
        }
    };
    Ok(Read {
        entry,
        extent,
        length,
        source,
    })
}

/// The bytes of a lowered presentation's media.
#[derive(Clone, Debug, Default)]
pub(crate) struct OdpMedia {
    handles: BTreeMap<Sha256, MediaHandle>,
}

impl MediaStore for OdpMedia {
    fn open(&self, sha256: &Sha256) -> Option<MediaHandle> {
        self.handles.get(sha256).cloned()
    }
}
