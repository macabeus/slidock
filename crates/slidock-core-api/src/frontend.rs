//! The contract every format frontend implements. A frontend lowers untrusted bytes into a source-phase presentation:
//! it reads only through the [`RangeReader`] it is given, reports content problems as diagnostics, and refuses a whole
//! input with [`FrontendError::Refused`].

use std::fmt;
use std::sync::Arc;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use slidock_io::RangeReader;
use slidock_ir::diagnostics::{Diagnostic, DiagnosticSink};
use slidock_ir::features::FeatureId;
use slidock_ir::fonts::FaceRequest;
use slidock_ir::formats::SourceFormat;
use slidock_ir::ids::{Sha256, SlideId};
use slidock_ir::presentation::SlideSize;
use slidock_ir::source;
use thiserror::Error;

use crate::deadline::{Deadline, Stopped};
use crate::limits::{Limits, Output};

/// The input a frontend reads: its basename and its bytes, by range.
#[derive(Clone, Copy)]
pub struct Input<'a> {
    pub name: &'a str,
    pub reader: &'a dyn RangeReader,
}

impl fmt::Debug for Input<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Input")
            .field("name", &self.name)
            .field("size", &self.size())
            .finish()
    }
}

impl<'a> Input<'a> {
    pub fn new(name: &'a str, reader: &'a dyn RangeReader) -> Self {
        Input { name, reader }
    }

    /// The input's size in bytes.
    pub fn size(&self) -> u64 {
        self.reader.len()
    }
}

/// Format-neutral services. Services one frontend needs are injected at its construction.
pub struct FrontendCtx<'a> {
    pub limits: &'a Limits,
    /// Stamped with stage `frontend` and the frontend's format.
    pub diag: &'a mut dyn DiagnosticSink,
    /// Checked between parts and slides; the pipeline sets it to the stage's time limit.
    pub deadline: Deadline<'a>,
    /// What the containers lowered so far hold, charged to [`Limits::output`] by [`crate::isolate_container`].
    pub spent: Output,
}

impl fmt::Debug for FrontendCtx<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrontendCtx")
            .field("limits", &self.limits)
            .field("deadline", &self.deadline)
            .field("spent", &self.spent)
            .finish_non_exhaustive()
    }
}

/// A slide as `analyze` sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlideFacts {
    pub id: SlideId,
    pub hidden: bool,
    pub title: Option<String>,
}

/// What `analyze` learns from metadata alone, before any media is hashed.
#[derive(Clone, Debug, PartialEq)]
pub struct Analysis {
    pub format: SourceFormat,
    pub size: SlideSize,
    /// The full show order, hidden slides included.
    pub slides: Vec<SlideFacts>,
    pub features: Vec<FeatureId>,
    /// The bytes of the media the presentation may show, at least those it does: a partial compile runs only when
    /// they fit the presentation budget.
    pub media_bytes: u64,
    /// Set when the input is refused, for the review step to show.
    pub refusal: Option<Diagnostic>,
}

impl Analysis {
    /// The presentation-wide values `lower` uses, whether it lowers every slide or only some.
    pub fn facts(&self) -> PresentationFacts {
        PresentationFacts {
            size: self.size.clone(),
            slides: self
                .slides
                .iter()
                .map(|slide| ShownSlide {
                    id: slide.id.clone(),
                    hidden: slide.hidden,
                })
                .collect(),
            media_bytes: self.media_bytes,
        }
    }
}

/// A slide of the show order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShownSlide {
    pub id: SlideId,
    pub hidden: bool,
}

/// Presentation-wide values from `analyze`, which `lower` and the passes use whether every slide is lowered or only
/// some.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PresentationFacts {
    pub size: SlideSize,
    pub slides: Vec<ShownSlide>,
    pub media_bytes: u64,
}

impl PresentationFacts {
    /// Every slide of the show, which links may target even when only some were lowered.
    pub fn linkable(&self) -> Vec<SlideId> {
        self.slides.iter().map(|slide| slide.id.clone()).collect()
    }

    /// The 1-based show position of a slide, `0` when it is not in the show.
    pub fn show_index(&self, id: &SlideId) -> usize {
        self.slides
            .iter()
            .position(|slide| &slide.id == id)
            .map_or(0, |index| index + 1)
    }
}

/// Where the bytes of a media item or an embedded font are.
#[derive(Clone)]
pub enum MediaSource {
    /// A range of the input itself (a STORED zip entry): a host slices its own copy of the input, so the bytes never
    /// pass through the compiler.
    InputRange { offset: u64, len: u64 },
    /// Bytes the frontend holds (an inflated entry).
    Bytes(Arc<[u8]>),
    /// Another source (a media folder beside an IR file).
    Reader(Arc<dyn RangeReader>),
}

impl fmt::Debug for MediaSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MediaSource::InputRange { offset, len } => f
                .debug_struct("InputRange")
                .field("offset", offset)
                .field("len", len)
                .finish(),
            MediaSource::Bytes(bytes) => f.debug_tuple("Bytes").field(&bytes.len()).finish(),
            MediaSource::Reader(reader) => f.debug_tuple("Reader").field(&reader.len()).finish(),
        }
    }
}

/// One media item's bytes.
#[derive(Clone, Debug)]
pub struct MediaHandle {
    pub bytes: u64,
    pub source: MediaSource,
}

/// The bytes of the media a lowered presentation references, by SHA-256. The presentation holds only the hashes.
pub trait MediaStore {
    fn open(&self, sha256: &Sha256) -> Option<MediaHandle>;

    /// The file a host decoded the item to ([`HostDecoded`]), which its transcode writes as it is; `None` for an item
    /// the compiler transcodes itself.
    fn decoded(&self, _sha256: &Sha256) -> Option<MediaHandle> {
        None
    }
}

/// The source types a host may decode for the compiler ([`HostDecoded`]): HEIC photos, which no browser but Safari
/// shows and the compiler does not decode.
pub const HOST_DECODED: [&str; 1] = ["image/heic"];

/// A medium a host decoded for the compiler (a HEIC photo its browser drew), already checked against the source:
/// plan-media transcodes the item to this file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostDecoded {
    /// The source item's.
    pub sha256: Sha256,
    /// `image/png` or `image/jpeg`.
    pub mime: &'static str,
    pub width: u32,
    pub height: u32,
    /// The decoded file's size.
    pub bytes: u64,
}

/// A store that holds nothing.
#[derive(Clone, Copy, Debug, Default)]
pub struct EmptyMediaStore;

impl MediaStore for EmptyMediaStore {
    fn open(&self, _sha256: &Sha256) -> Option<MediaHandle> {
        None
    }
}

/// The application that wrote the source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct App {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// Where a presentation came from. It travels beside the presentation, out of the passes' reach.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub format: SourceFormat,
    /// The input's basename.
    pub name: String,
    pub sha256: Sha256,
    pub bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<App>,
}

/// How an embedded font's bytes are packaged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FontContainer {
    /// TrueType, OpenType or a collection, as is.
    Sfnt,
    /// Embedded OpenType (PPTX `.fntdata`), possibly MicroType Express compressed.
    Eot,
    Woff,
    Woff2,
}

/// A font the source embeds.
#[derive(Clone, Debug)]
pub struct EmbeddedFont {
    /// The family the source names it by.
    pub family: String,
    /// The faces the source says the file holds.
    pub faces: Vec<FaceRequest>,
    pub container: FontContainer,
    pub source: MediaSource,
}

/// A lowered presentation before verify-source: typed when a frontend built it, the input's JSON as it came when
/// an agent wrote it.
#[derive(Clone, Debug, PartialEq)]
pub enum Unverified {
    Typed(Box<source::Presentation>),
    Json(serde_json::Value),
}

impl From<source::Presentation> for Unverified {
    fn from(presentation: source::Presentation) -> Self {
        Unverified::Typed(Box::new(presentation))
    }
}

impl From<serde_json::Value> for Unverified {
    fn from(value: serde_json::Value) -> Self {
        Unverified::Json(value)
    }
}

pub struct LowerResult {
    pub provenance: Provenance,
    pub presentation: Unverified,
    pub media: Box<dyn MediaStore>,
    /// The fonts the source embeds.
    pub fonts: Vec<EmbeddedFont>,
    /// What the frontend detected in the source, per container id and under `presentation`: the loss check's first
    /// input.
    pub features: IndexMap<String, Vec<FeatureId>>,
}

impl fmt::Debug for LowerResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LowerResult")
            .field("provenance", &self.provenance)
            .field("presentation", &self.presentation)
            .field("fonts", &self.fonts)
            .field("features", &self.features)
            .finish_non_exhaustive()
    }
}

/// What sniffing sees besides the head: the input's name and, for a zip, its entry names.
#[derive(Clone, Copy, Debug, Default)]
pub struct SniffHints<'a> {
    pub name: &'a str,
    pub zip_names: Option<&'a [String]>,
}

/// What `lower` is asked for.
#[derive(Clone, Copy, Debug)]
pub struct LowerOptions<'a> {
    pub facts: &'a PresentationFacts,
    /// Lower just these slides and their layout, master and theme chain, with the same ids and bytes.
    pub only: Option<&'a [SlideId]>,
    /// Read the source's motion natively (the deck path): what the common effect vocabulary adds, and the timing
    /// a frontend lowers only for it. Off on the parity path, whose outputs stay those before the vocabulary.
    pub native_motion: bool,
}

/// How a frontend stage ends without a result.
#[derive(Clone, Debug, PartialEq, Error)]
pub enum FrontendError {
    /// The whole input is refused: not this format, encrypted, legacy, over a limit.
    #[error("{}", .0.message)]
    Refused(Box<Diagnostic>),
    /// The stage's deadline passed.
    #[error("stopped: over its time limit")]
    Stopped,
    /// Anything else: a bug, or input the frontend could not survive.
    #[error("{0}")]
    Crashed(String),
}

impl From<Stopped> for FrontendError {
    fn from(_: Stopped) -> Self {
        FrontendError::Stopped
    }
}

impl FrontendError {
    pub fn refused(diagnostic: Diagnostic) -> Self {
        FrontendError::Refused(Box::new(diagnostic))
    }
}

/// A format frontend. It is synchronous and does no I/O of its own: it reads the input by range.
pub trait Frontend {
    fn format(&self) -> SourceFormat;

    /// Informational.
    fn version(&self) -> &str;

    /// Whether this frontend reads the input, from its first 4 KiB and, for a zip, its entry names.
    fn sniff(&self, head: &[u8], hints: &SniffHints<'_>) -> bool;

    fn analyze(
        &self,
        input: &Input<'_>,
        ctx: &mut FrontendCtx<'_>,
    ) -> Result<Analysis, FrontendError>;

    /// With `options.only`, lowers just those slides and their layout, master and theme chain, with the same ids and
    /// bytes as in the whole presentation.
    fn lower(
        &self,
        input: &Input<'_>,
        ctx: &mut FrontendCtx<'_>,
        options: &LowerOptions<'_>,
    ) -> Result<LowerResult, FrontendError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn show_index_is_one_based_and_zero_for_a_slide_not_in_the_show() {
        let facts: PresentationFacts = serde_json::from_value(serde_json::json!({
            "size": { "w": 720, "h": 405 },
            "slides": [{ "id": "ir-s1", "hidden": false }, { "id": "ir-s2", "hidden": true }],
            "mediaBytes": 0,
        }))
        .unwrap();
        let slide = |text: &str| SlideId::new(text).unwrap();
        assert_eq!(facts.show_index(&slide("ir-s1")), 1);
        assert_eq!(facts.show_index(&slide("ir-s2")), 2);
        assert_eq!(facts.show_index(&slide("ir-s9")), 0);
    }
}
