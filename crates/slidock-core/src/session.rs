//! The two-phase compile. [`Session::open`] sniffs, analyzes and lowers the input once, verifies it
//! and runs the passes that need no font. The host then asks [`Session::font_needs`], adds font files from its
//! sources with [`Session::add_font`], and calls [`Session::finish`], which runs the passes from bind-fonts on with
//! what was added. `finish` can be called again whenever more fonts arrive; parsing never repeats. Media take the same
//! path: [`Session::media_needs`] lists the HEIC photos a host may decode, and [`Session::add_media`] takes its checked
//! files for the next `finish` ([`crate::host_media`]).
//!
//! Fonts resolve per face in the user's order: embedded → installed → Google Fonts → dropped, with
//! the bundled substitutes as the floor: the base catalog's metrics until a host adds their files
//! ([`FontSource::Bundled`], the faces [`Finished::substitutes`] names). The source's embedded fonts are taken only
//! when [`SessionOptions::embedded_fonts`] is on; they are read and unwrapped on first use, never at open, and each
//! must prove its identity. A subset file must also hold every code point the presentation draws with its face.
//!
//! A font file is repaired as it is added where that is safe ([`slidock_fonts::sfnt::repair`]): a face OTS refuses
//! only for want of an `OS/2` table gets one written from its other tables, which states restricted embedding (the
//! face states no permissions, and an unknown value fails closed). A face no browser would load (OTS refuses it) or
//! whose `OS/2.fsType` forbids shipping it in the session's [`SessionOptions::target`] (the licence policy: bitmap-only
//! embedding anywhere, restricted embedding in a download) never enters the binding catalog: its substitute is bound
//! and measured instead, so the deck never draws with a face it does not ship, and bind-fonts says why the family it
//! asks for is not drawn (MID5404). An emoji family needs no file ([`slidock_fonts::catalog::emoji`]): it is never a
//! need, and the viewer's emoji font draws it. A preview binds a restricted face, and its download leaves the face out
//! ([`slidock_fonts::LicenceEntry::download`]). With a glyph collector ([`SessionOptions::glyphs`]), `finish` also
//! ships the faces the deck draws ([`crate::shipping`]).

use std::cell::{Cell, OnceCell};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use indexmap::IndexMap;
use serde::Serialize;
use slidock_core_api::frontend::FontContainer;
use slidock_core_api::visit::walk_elements;
use slidock_core_api::{
    Analysis, Clock, ContainerOrder, Deadline, EmbeddedFont, HostDecoded, Input, Limits,
    LowerOptions, LowerResult, MediaHandle, MediaPolicy, MediaSource, MediaStore, PassEnv,
    PresentationFacts, Provenance, sort_diagnostics,
};
use slidock_fonts::bind::WeightRange;
use slidock_fonts::catalog::builtin::floor_catalog;
use slidock_fonts::catalog::emoji::is_emoji_family;
use slidock_fonts::catalog::substitutes::{LAST_RESORT, SUBSTITUTES, generic_family};
use slidock_fonts::container::{self, Hint};
use slidock_fonts::glyphs::{bundled_covers, draws_no_glyph};
use slidock_fonts::google::{
    Css2Error, GoogleCatalog, RemoteFace, UnicodeRange, google_catalog, parse_css2,
};
use slidock_fonts::policy::Exclusion;
use slidock_fonts::resolve::{
    self, Candidate, Declared, FaceTraits, FontUse, GooglePlan, Identity, LocalBudget,
    LocalFontMeta, LocalPick, LocalQuery, LocalRefusal, Mismatch, ResolvedFamily, ResolvedSlot,
    SourceKind, SourceOrder, face_traits, missing_code_points,
};
use slidock_fonts::sfnt::{FaceRepair, Repaired, read_details, repair, sanitizer_refusal};
use slidock_fonts::{
    CatalogFace, Decision, FaceFile, FamilyKey, FontCatalog, FontLimits, FontOrigin, FsType,
    LicencePolicy, SfntError, SfntFace, Target, Toggle, builtin_catalog, decide, read_sfnt,
};
use slidock_io::RangeReader;
use slidock_ir::diagnostics::{Diagnostic, codes};
use slidock_ir::features::FeatureId;
use slidock_ir::fonts::{FaceRequest, FaceStyle, GenericFamily};
use slidock_ir::ids::{Sha256, SlideId};
use slidock_ir::resolved;
use slidock_ir::source::Presentation;
use slidock_ir::text::{Bullet, FontRef, Inline, PartialRunProps};
use slidock_ir::verify::element_texts;
use thiserror::Error;

use crate::compile::{FrontendChoice, InputOptions, Stats, check_only, lower, open};
use crate::foreign::foreign_diagnostics;
use crate::host_media::{MediaNeed, MediaRefusal, check_decoded, media_needs};
use crate::manager::{FailureDump, PassSnapshot, Progress, RunOptions, VerifyMode};
use crate::pipeline::Pipeline;
use crate::registry::FrontendRegistry;
use crate::shipping::{CollectGlyphs, DemotedSlots, GlyphUse, ShippedFace, ship_faces};
use crate::verify::verify_source_motion;

/// Where a font file came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FontSource {
    /// A file the user dropped.
    User,
    Google,
    /// Installed on the viewer's computer.
    Local,
    /// Embedded in the source.
    Embedded,
    /// One of slidock's substitute families (Liberation, Carlito, DejaVu Sans, Caladea): the floor under every
    /// source. It fills no face slot of a family the presentation asks for; its faces replace the base catalog's
    /// metrics-only faces of the same family.
    Bundled,
}

impl FontSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            FontSource::User => "user",
            FontSource::Google => "google",
            FontSource::Local => "local",
            FontSource::Embedded => "embedded",
            FontSource::Bundled => "bundled",
        }
    }

    /// The licence policy's origin.
    pub const fn origin(self) -> FontOrigin {
        match self {
            FontSource::User => FontOrigin::User,
            FontSource::Google => FontOrigin::Google,
            FontSource::Local => FontOrigin::Local,
            FontSource::Embedded => FontOrigin::Embedded,
            FontSource::Bundled => FontOrigin::Bundled,
        }
    }

    /// The resolution source it is; `None` for the bundled floor, which is no source.
    pub const fn kind(self) -> Option<SourceKind> {
        match self {
            FontSource::User => Some(SourceKind::UserDropped),
            FontSource::Google => Some(SourceKind::Google),
            FontSource::Local => Some(SourceKind::ViewerInstalled),
            FontSource::Embedded => Some(SourceKind::Embedded),
            FontSource::Bundled => None,
        }
    }
}

/// An embedded font out of its container.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnwrappedFont {
    /// TrueType or OpenType.
    pub sfnt: Vec<u8>,
    /// Made of only the glyphs its document used (an EOT's `TTEMBED_SUBSET`).
    pub subset: bool,
}

/// Unwraps an embedded font's container (EOT and MicroType Express, WOFF, WOFF2) into TrueType/OpenType bytes.
pub trait FontUnwrap {
    fn unwrap_font(&self, bytes: &[u8], container: FontContainer) -> Result<UnwrappedFont, String>;
}

/// Takes TrueType/OpenType files as they are and refuses every other container.
#[derive(Clone, Copy, Debug, Default)]
pub struct SfntOnly;

impl FontUnwrap for SfntOnly {
    fn unwrap_font(&self, bytes: &[u8], container: FontContainer) -> Result<UnwrappedFont, String> {
        match container {
            FontContainer::Sfnt => Ok(UnwrappedFont {
                sfnt: bytes.to_vec(),
                subset: false,
            }),
            other => Err(format!("{other:?} fonts cannot be unwrapped yet")),
        }
    }
}

/// Unwraps every container slidock reads (EOT with or without MicroType Express, WOFF, WOFF2, obfuscated OOXML fonts)
/// with `slidock-fonts`' decoders. The container the source declares is a hint only: the bytes decide.
#[derive(Clone, Copy, Debug, Default)]
pub struct Containers;

impl FontUnwrap for Containers {
    fn unwrap_font(
        &self,
        bytes: &[u8],
        _container: FontContainer,
    ) -> Result<UnwrappedFont, String> {
        let unwrapped = container::unwrap(bytes, &Hint::default()).map_err(|e| e.to_string())?;
        Ok(UnwrappedFont {
            sfnt: unwrapped.sfnt,
            subset: unwrapped.subset,
        })
    }
}

/// Why a face of a font the session holds never binds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FaceRefusal {
    /// Browsers' font sanitizer would refuse it, and draw a fallback in its place.
    Browser(String),
    /// Its `OS/2.fsType` keeps it out of the session's target.
    Licence(Exclusion),
}

impl FaceRefusal {
    /// Why the face is not drawn, as a diagnostic's `reason` says it; `repair` is what was written into the face.
    pub fn reason(&self, repair: Option<FaceRepair>) -> String {
        match (self, repair) {
            (FaceRefusal::Browser(reason), _) => format!("browsers would refuse its file ({reason})"),
            (FaceRefusal::Licence(Exclusion::BitmapOnly), _) => {
                "its licence (fsType) allows embedding its bitmaps only".to_owned()
            }
            (FaceRefusal::Licence(_), Some(FaceRepair::Os2)) => {
                "it states no embedding permissions (it has no OS/2 table), so it is treated as restricted and a \
                 download leaves it out"
                    .to_owned()
            }
            (FaceRefusal::Licence(_), None) => {
                "its licence (fsType) restricts embedding it, so a download leaves it out".to_owned()
            }
        }
    }
}

/// A font file the session holds: one the host added, or one the source embeds.
#[derive(Clone, Debug, PartialEq)]
pub struct AddedFont {
    pub source: FontSource,
    /// Of `bytes`.
    pub sha256: Sha256,
    pub faces: Vec<SfntFace>,
    /// Per face: its legacy family and `wght` range.
    pub traits: Vec<FaceTraits>,
    /// For an embedded file: the faces the source says it holds.
    pub declared: Vec<Declared>,
    /// An embedded file made of only the glyphs its document used.
    pub subset: bool,
    /// For a subset file, per face and per declared face: how many code points the presentation draws with that face
    /// the file has no glyph for. Empty for a whole file.
    pub missing: Vec<Vec<u32>>,
    /// Per face: why it never binds in a session that ships fonts ([`Session::refusal`]).
    pub refused: Vec<Option<FaceRefusal>>,
    /// Per face: what was written into it so that browsers load it (empty when nothing was).
    pub repairs: Vec<Option<FaceRepair>>,
    /// For one `unicode-range` chunk of a face (Google Fonts serves browsers a face as per-script chunks): the code
    /// points it is for. The chunks of a face bind as one face and ship together, each with its own rule.
    pub unicode_range: Option<UnicodeRange>,
    /// TrueType or OpenType (an embedded file or a WOFF/WOFF2 unwrapped).
    pub bytes: Arc<[u8]>,
}

impl AddedFont {
    /// What was written into face `index` so that browsers load it.
    pub fn repair(&self, index: usize) -> Option<FaceRepair> {
        self.repairs.get(index).copied().flatten()
    }
}

/// A font the source embeds, as its frontend declares it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbeddedFontInfo {
    /// The family the source names it by.
    pub family: String,
    /// The faces the source says the file holds.
    pub faces: Vec<FaceRequest>,
    pub container: FontContainer,
}

/// A family the presentation's text uses, with the faces no source has exactly yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontNeed {
    /// As the source records it, else as the text names it.
    pub family: String,
    pub key: FamilyKey,
    /// The faces still open, as the text asks for them, by weight, then style.
    pub faces: Vec<FaceRequest>,
    pub postscript_names: Vec<String>,
    /// The source embeds the font.
    pub embedded: bool,
    /// Every face slot of the family and how it is filled.
    pub slots: Vec<ResolvedSlot>,
    /// The Google Fonts request for the open faces, for a catalog family when Google Fonts is on.
    pub google: Option<GooglePlan>,
    /// What installed fonts are matched against, when installed fonts are on.
    pub local: Option<LocalQuery>,
}

/// A problem with a font the session holds; the compile goes on without the face.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FontIssue {
    /// An embedded font that could not be read, unwrapped or parsed (MID5411).
    EmbeddedUnreadable { family: String, reason: String },
    /// An embedded font larger than the limit, left unread (MID5411).
    EmbeddedTooLarge {
        family: String,
        bytes: u64,
        limit: u64,
    },
    /// An embedded file whose own tables name another face than the source declares (MID5410): it fills that slot
    /// only when no source has the face.
    EmbeddedMislabeled {
        family: String,
        weight: u32,
        style: FaceStyle,
        file_family: String,
        file_weight: u32,
        file_italic: bool,
        mismatch: Mismatch,
    },
    /// A subset embedded file without glyphs for `missing` of the code points the presentation draws with the face it
    /// is declared as (MID5410): it fills that slot only when no source has the face.
    EmbeddedSubset {
        family: String,
        weight: u32,
        style: FaceStyle,
        missing: u32,
    },
    /// A face browsers' font sanitizer would refuse (MID5419): it never binds, and its substitute draws instead.
    BrowserRefused {
        family: String,
        postscript_name: String,
        source: FontSource,
        reason: String,
    },
    /// A face whose `OS/2.fsType` keeps it out of a download (MID5413): restricted, or bitmap-only, embedding, or
    /// `unstated` (the face has no `OS/2` table, so it states no permissions and is treated as restricted). Its
    /// substitute draws instead, in the download only when `in_preview`: the preview draws with the face.
    LicenceExcluded {
        family: String,
        postscript_name: String,
        source: FontSource,
        bitmap_only: bool,
        unstated: bool,
        in_preview: bool,
    },
    /// A face browsers would refuse that was repaired so that they load it (MID5420): `repair` says what it lacked.
    Repaired {
        family: String,
        postscript_name: String,
        source: FontSource,
        repair: FaceRepair,
    },
    /// A shipped face that could not be subset (MID5418): it ships whole.
    ShippedWhole {
        family: String,
        postscript_name: String,
        reason: String,
    },
    /// A shipped face that could not be encoded as WOFF2 (MID5419): it ships as its raw TrueType/OpenType file.
    ShippedRaw {
        family: String,
        postscript_name: String,
        reason: String,
    },
    /// A face the deck draws with that cannot ship (MID5419): the viewer's own fonts draw it.
    NotShipped {
        family: String,
        postscript_name: String,
        reason: String,
    },
    /// The code points the deck draws could not be collected (MID5419): no face ships.
    GlyphsFailed { reason: String },
    /// Code points the deck draws with a shipped face that neither it nor the last-resort face has (MID5415): the
    /// viewer's fonts draw them. `code_points` quotes the first of the `missing`.
    CoverageGap {
        family: String,
        postscript_name: String,
        missing: u64,
        code_points: Vec<u32>,
    },
}

impl FontIssue {
    /// The issue's font diagnostic code.
    pub const fn code(&self) -> &'static str {
        match self {
            FontIssue::EmbeddedUnreadable { .. } | FontIssue::EmbeddedTooLarge { .. } => "MID5411",
            FontIssue::EmbeddedMislabeled { .. } | FontIssue::EmbeddedSubset { .. } => "MID5410",
            FontIssue::LicenceExcluded { .. } => "MID5413",
            FontIssue::Repaired { .. } => "MID5420",
            FontIssue::CoverageGap { .. } => "MID5415",
            FontIssue::ShippedWhole { .. } => "MID5418",
            FontIssue::BrowserRefused { .. }
            | FontIssue::ShippedRaw { .. }
            | FontIssue::NotShipped { .. }
            | FontIssue::GlyphsFailed { .. } => "MID5419",
        }
    }
}

impl fmt::Display for FontIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let face = |family: &str, weight: u32, italic: bool| {
            let slant = if italic { " italic" } else { "" };
            format!("{family} {weight}{slant}")
        };
        match self {
            FontIssue::EmbeddedUnreadable { family, reason } => write!(f, "{family}: {reason}"),
            FontIssue::EmbeddedTooLarge {
                family,
                bytes,
                limit,
            } => write!(f, "{family}: {bytes} bytes, over the {limit} limit"),
            FontIssue::EmbeddedMislabeled {
                family,
                weight,
                style,
                file_family,
                file_weight,
                file_italic,
                mismatch,
            } => {
                let why = match mismatch {
                    Mismatch::Family { .. } => "another family",
                    Mismatch::Style { file_italic: true } => "italic, for an upright face",
                    Mismatch::Style { file_italic: false } => "upright, for an italic face",
                    Mismatch::Weight { .. } => "another weight",
                    Mismatch::Subset { .. } => "glyphs missing",
                };
                write!(
                    f,
                    "{}: the file is {} ({why}); used only if no source has the face",
                    face(family, *weight, *style == FaceStyle::Italic),
                    face(file_family, *file_weight, *file_italic),
                )
            }
            FontIssue::EmbeddedSubset {
                family,
                weight,
                style,
                missing,
            } => write!(
                f,
                "{}: the subset file has no glyph for {missing} of the characters drawn with it; used only if no \
                 source has the face",
                face(family, *weight, *style == FaceStyle::Italic),
            ),
            FontIssue::BrowserRefused {
                family,
                postscript_name,
                source,
                reason,
            } => write!(
                f,
                "{family} ({postscript_name}, {}): browsers would refuse the face ({reason}); its substitute draws \
                 instead",
                source.as_str()
            ),
            FontIssue::LicenceExcluded {
                family,
                postscript_name,
                source,
                bitmap_only,
                unstated,
                in_preview,
            } => {
                let why = if *bitmap_only {
                    "allows embedding its bitmaps only"
                } else if *unstated {
                    "states no embedding permissions (it has no OS/2 table), so it is treated as restricted"
                } else {
                    "is restricted from embedding"
                };
                let source = source.as_str();
                if *in_preview {
                    write!(
                        f,
                        "{family} ({postscript_name}, {source}): the face {why}; the preview draws with it, and the \
                         download leaves it out unless its family is included"
                    )
                } else {
                    write!(
                        f,
                        "{family} ({postscript_name}, {source}): the face {why}; its substitute draws instead"
                    )
                }
            }
            FontIssue::Repaired {
                family,
                postscript_name,
                source,
                repair,
            } => write!(
                f,
                "{family} ({postscript_name}, {}): {repair}; slidock wrote one from its other tables so they load it",
                source.as_str()
            ),
            FontIssue::ShippedWhole {
                family,
                postscript_name,
                reason,
            } => write!(
                f,
                "{family} ({postscript_name}): ships whole, not subset ({reason})"
            ),
            FontIssue::ShippedRaw {
                family,
                postscript_name,
                reason,
            } => write!(
                f,
                "{family} ({postscript_name}): ships as TrueType/OpenType, not WOFF2 ({reason})"
            ),
            FontIssue::NotShipped {
                family,
                postscript_name,
                reason,
            } => write!(
                f,
                "{family} ({postscript_name}): does not ship ({reason}); the viewer's fonts draw it"
            ),
            FontIssue::GlyphsFailed { reason } => {
                write!(
                    f,
                    "no face ships: the drawn characters are unknown ({reason})"
                )
            }
            FontIssue::CoverageGap {
                family,
                postscript_name,
                missing,
                code_points,
            } => {
                let quoted: Vec<String> = code_points
                    .iter()
                    .map(|point| format!("U+{point:04X}"))
                    .collect();
                let more = if *missing > code_points.len() as u64 {
                    ", …"
                } else {
                    ""
                };
                write!(
                    f,
                    "{family} ({postscript_name}): no shipped face has {missing} of the characters drawn with it ({}{more}); \
                     the viewer's fonts draw them",
                    quoted.join(", ")
                )
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Error)]
pub enum SessionError {
    #[error("The font file cannot be read: {0}")]
    UnreadableFont(#[from] SfntError),
    #[error("The font file cannot be unwrapped: {0}")]
    UnreadableContainer(String),
    #[error("The font file has {bytes} bytes, more than the {limit} a font may have")]
    FontTooLarge { bytes: u64, limit: u64 },
    #[error("The installed font is not read: {0}")]
    LocalRefused(#[from] LocalRefusal),
    #[error("The decoded medium is not taken: {0}")]
    MediaRefused(#[from] MediaRefusal),
}

#[derive(Clone, Copy)]
pub struct SessionOptions<'a> {
    pub registry: &'a FrontendRegistry,
    pub frontend: FrontendChoice,
    /// Lower and compile only these slides (with their layout, master and theme chain).
    pub only: Option<&'a [SlideId]>,
    pub pipeline: &'a Pipeline,
    pub verify: VerifyMode,
    pub limits: Limits,
    /// The fonts every compile can bind to (the built-in metrics, the bundled substitutes), below every source. A
    /// session that ships fonts keeps only its bundled families ([`Session::catalog`]).
    pub fonts: &'a FontCatalog,
    /// The order font sources fill face slots in; a source left out is ignored.
    pub font_order: SourceOrder,
    /// Read the source's embedded fonts and add them (off, the parity path, binds as before fonts shipped). Fonts a
    /// host adds as [`FontSource::Embedded`] count either way.
    pub embedded_fonts: bool,
    /// Unwraps embedded fonts.
    pub unwrap: &'a dyn FontUnwrap,
    /// Collects the code points the deck draws with each face: with one, `finish` ships the faces the deck draws
    /// (subset, WOFF2, `@font-face`); without one (the parity path), nothing ships.
    pub glyphs: Option<&'a dyn CollectGlyphs>,
    /// What the finished deck is for, which decides the faces its `fsType` lets bind: a download (the default) binds
    /// no restricted face; a preview, shown on the computer that holds the fonts, does.
    pub target: Target,
    pub policy: MediaPolicy,
    pub clock: Option<&'a dyn Clock>,
    pub trace: bool,
    /// Keep the native motion data the frontend lowers (the effect vocabulary, timing and match keys). Off, the
    /// parity path, it is left out once the presentation verifies, as [`crate::CompileOptions::native_motion`] does.
    pub native_motion: bool,
}

impl fmt::Debug for SessionOptions<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionOptions")
            .field("frontend", &self.frontend)
            .field("only", &self.only)
            .field("pipeline", &self.pipeline)
            .field("verify", &self.verify)
            .field("font_order", &self.font_order)
            .field("embedded_fonts", &self.embedded_fonts)
            .field("glyphs", &self.glyphs.is_some())
            .field("target", &self.target)
            .field("clock", &self.clock.is_some())
            .field("trace", &self.trace)
            .field("native_motion", &self.native_motion)
            .finish_non_exhaustive()
    }
}

impl<'a> SessionOptions<'a> {
    /// The built-in fonts, every source in the default order, embedded fonts off, every container unwrapped, nothing
    /// shipped, faces bound as a download ships them, native motion left out.
    pub fn new(registry: &'a FrontendRegistry, pipeline: &'a Pipeline) -> Self {
        SessionOptions {
            registry,
            frontend: FrontendChoice::Auto,
            only: None,
            pipeline,
            verify: VerifyMode::Boundaries,
            limits: Limits::default(),
            fonts: builtin_catalog(),
            font_order: SourceOrder::default(),
            embedded_fonts: false,
            unwrap: &Containers,
            glyphs: None,
            target: Target::Download,
            policy: MediaPolicy::default(),
            clock: None,
            trace: false,
            native_motion: false,
        }
    }

    fn input(&self, deadline: Deadline<'a>) -> InputOptions<'_> {
        InputOptions {
            registry: self.registry,
            frontend: self.frontend,
            limits: &self.limits,
            clock: self.clock,
            deadline,
        }
    }

    fn run<'r>(&'r self, env: PassEnv<'r>, deadline: Deadline<'a>) -> RunOptions<'r> {
        RunOptions {
            verify: self.verify,
            env,
            clock: self.clock,
            deadline,
            visit_order: None,
            trace: self.trace,
        }
    }

    /// The most bytes one font file may have.
    fn font_bytes(&self) -> u64 {
        self.limits.zip.entry_bytes
    }
}

/// An input that could not be opened: refused as a whole, or a frontend stage that crashed or ran out of time.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenError {
    /// In output order.
    pub diagnostics: Vec<Diagnostic>,
    /// Set when the input was refused as a whole.
    pub refusal: Option<Diagnostic>,
}

/// What `finish` resolved.
#[derive(Clone, Debug, PartialEq)]
pub struct Finished {
    /// The verified source presentation, failed containers marked.
    pub source: Option<Presentation>,
    /// None when the presentation failed as a whole.
    pub presentation: Option<resolved::Presentation>,
    /// Every diagnostic of the compile, the frontend's included, in output order.
    pub diagnostics: Vec<Diagnostic>,
    /// The problems with the fonts the session holds, then with the faces shipped.
    pub font_issues: Vec<FontIssue>,
    /// The faces the deck ships, when the session has a glyph collector.
    pub fonts: Vec<ShippedFace>,
    /// The faces of the bundled substitutes the deck draws with that no file provides yet (metrics only): a host that
    /// has them adds them ([`FontSource::Bundled`]) and finishes again, and they ship. It also names the last-resort
    /// face ([`LAST_RESORT`]) when the session lacks its file and a shipped face lacks code points the deck draws, or
    /// a face still waiting for its file may (it draws code points outside what every bundled face maps).
    pub substitutes: Vec<SubstituteFace>,
    pub failures: Vec<FailureDump>,
    pub stats: Option<Stats>,
    pub trace: Option<Vec<PassSnapshot>>,
}

/// An embedded font's bytes as open left them: in hand, still to read, or refused.
enum EmbeddedBytes {
    Ready(Arc<[u8]>),
    /// Read (and inflated, for a compressed entry) on first use.
    Deferred(Arc<dyn RangeReader>),
    Refused(FontIssue),
}

/// The embedded fonts once read: the files and the problems.
#[derive(Default)]
struct Embedded {
    fonts: Vec<AddedFont>,
    issues: Vec<FontIssue>,
}

pub struct Session<'a> {
    options: SessionOptions<'a>,
    /// The compile's, from `open` ([`Limits::compile_ms`]).
    deadline: Deadline<'a>,
    analysis: Analysis,
    facts: PresentationFacts,
    provenance: Provenance,
    media: Box<dyn MediaStore>,
    embedded: Vec<EmbeddedFontInfo>,
    /// Per embedded font, when embedded fonts are on; taken (and each dropped once read) on first use.
    embedded_bytes: Cell<Vec<EmbeddedBytes>>,
    embedded_fonts: OnceCell<Embedded>,
    features: IndexMap<String, Vec<FeatureId>>,
    /// The frontend's, verify-source's and the Foreign nodes' diagnostics.
    opening: Vec<Diagnostic>,
    source: Option<Presentation>,
    /// The run of the passes before fonts.
    before_fonts: Option<Progress>,
    fonts: Vec<AddedFont>,
    /// The installed files added so far.
    local: LocalBudget,
    /// The media a host may decode, planned once.
    media_needs: OnceCell<Vec<MediaNeed>>,
    /// The media a host decoded, checked, and their files.
    decoded: Vec<HostDecoded>,
    decoded_files: IndexMap<Sha256, Arc<[u8]>>,
    /// The files the last `finish` planned with: what its deck prints.
    finished_files: IndexMap<Sha256, Arc<[u8]>>,
    finishes: usize,
}

impl fmt::Debug for Session<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("provenance", &self.provenance)
            .field("finishes", &self.finishes)
            .field("fonts", &self.fonts.len())
            .field("embedded", &self.embedded.len())
            .finish_non_exhaustive()
    }
}

impl<'a> Session<'a> {
    /// Sniffs, analyzes and lowers the input, verifies it and runs the passes before fonts.
    pub fn open(input: &Input<'_>, options: SessionOptions<'a>) -> Result<Self, Box<OpenError>> {
        let mut collected = Vec::new();
        let stopped = |mut collected: Vec<Diagnostic>, stop: crate::compile::Stop| {
            collected.push((*stop.diagnostic).clone());
            Box::new(OpenError {
                diagnostics: sort_diagnostics(collected, &ContainerOrder::default()),
                refusal: stop.refused.then_some(*stop.diagnostic),
            })
        };
        let deadline = Deadline::start(options.clock, options.limits.compile_ms);
        let input_options = options.input(deadline);
        let opened = match open(input, &input_options, &mut collected) {
            Ok(opened) => opened,
            Err(stop) => return Err(stopped(collected, stop)),
        };
        let facts = opened.analysis.facts();
        if let Err(stop) = check_only(options.only, &facts, &options.policy) {
            return Err(stopped(collected, stop));
        }
        let lower_options = LowerOptions {
            facts: &facts,
            only: options.only,
            native_motion: options.native_motion,
        };
        let lowered = match lower(
            input,
            &opened,
            &input_options,
            &lower_options,
            &mut collected,
        ) {
            Ok(lowered) => lowered,
            Err(stop) => return Err(stopped(collected, stop)),
        };
        let LowerResult {
            provenance,
            presentation,
            media,
            fonts,
            features,
        } = lowered;
        let verified = verify_source_motion(&presentation, Some(&facts), options.native_motion);
        if let Some(source) = &verified.presentation {
            collected.extend(foreign_diagnostics(source));
        }
        collected.extend(verified.diagnostics);
        if options.embedded_fonts && options.glyphs.is_some() {
            // The embedded fonts are read and shipped: each is unwrapped, or its problem reported (MID5411).
            collected.retain(|diagnostic| diagnostic.code != codes::PX2501);
        }
        let before_fonts = verified.presentation.as_ref().map(|source| {
            let env = PassEnv {
                limits: &options.limits,
                fonts: options.fonts,
                policy: &options.policy,
                facts: &facts,
                host_media: &[],
            };
            let run = options.run(env, deadline);
            let mut progress = Progress::new(source, &run);
            let boundary = options.pipeline.font_boundary();
            options
                .pipeline
                .manager()
                .advance(&mut progress, 0..boundary, options.verify, &run);
            progress
        });
        let mut budget = options.limits.zip.memory_bytes;
        let mut embedded = Vec::with_capacity(fonts.len());
        let mut embedded_bytes = Vec::new();
        for font in fonts {
            if options.embedded_fonts {
                embedded_bytes.push(read_at_open(
                    &font,
                    input.reader,
                    options.font_bytes(),
                    &mut budget,
                ));
            }
            embedded.push(EmbeddedFontInfo {
                family: font.family,
                faces: font.faces,
                container: font.container,
            });
        }
        Ok(Session {
            options,
            deadline,
            analysis: opened.analysis,
            facts,
            provenance,
            media,
            embedded,
            embedded_bytes: Cell::new(embedded_bytes),
            embedded_fonts: OnceCell::new(),
            features,
            opening: collected,
            source: verified.presentation,
            before_fonts,
            fonts: Vec::new(),
            local: LocalBudget::default(),
            media_needs: OnceCell::new(),
            decoded: Vec::new(),
            decoded_files: IndexMap::new(),
            finished_files: IndexMap::new(),
            finishes: 0,
        })
    }

    pub fn analysis(&self) -> &Analysis {
        &self.analysis
    }

    /// The compile's deadline, from `open`: what printing and bundling the deck keep within too.
    pub fn deadline(&self) -> Deadline<'a> {
        self.deadline
    }

    pub fn facts(&self) -> &PresentationFacts {
        &self.facts
    }

    pub fn provenance(&self) -> &Provenance {
        &self.provenance
    }

    pub fn features(&self) -> &IndexMap<String, Vec<FeatureId>> {
        &self.features
    }

    /// The fonts the source embeds, as the frontend declared them.
    pub fn embedded_fonts(&self) -> &[EmbeddedFontInfo] {
        &self.embedded
    }

    pub fn media(&self, sha256: &Sha256) -> Option<MediaHandle> {
        self.media.open(sha256)
    }

    /// The media the presentation shows that a host may decode (HEIC photos), each with its decode plan
    /// ([`crate::host_media`]), planned on the first call. `input` is the input the session was opened on.
    pub fn media_needs(&self, input: &dyn RangeReader) -> &[MediaNeed] {
        self.media_needs.get_or_init(|| {
            let Some(presentation) = self.before_fonts() else {
                return Vec::new();
            };
            media_needs(
                presentation,
                self.media.as_ref(),
                input,
                &self.options.policy,
            )
        })
    }

    /// Adds a host's decode of the need `sha256` (a PNG or JPEG of its plan's output size) for the next `finish`,
    /// replacing an earlier one: plan-media transcodes the item to it (MID5608) instead of omitting it. A deck
    /// finished before prints with the files its own `finish` planned with until `finish` runs again.
    pub fn add_media(
        &mut self,
        sha256: &Sha256,
        bytes: Vec<u8>,
        input: &dyn RangeReader,
    ) -> Result<HostDecoded, SessionError> {
        let needs = self.media_needs(input);
        let Some(need) = needs.iter().find(|need| &need.sha256 == sha256) else {
            return Err(MediaRefusal::NotNeeded(sha256.to_hex()).into());
        };
        let decoded = check_decoded(need, &bytes, &self.options.policy)?;
        self.decoded.retain(|held| &held.sha256 != sha256);
        self.decoded.push(decoded.clone());
        self.decoded_files.insert(*sha256, Arc::from(bytes));
        Ok(decoded)
    }

    /// The media a host decoded, in the order they were added.
    pub fn decoded_media(&self) -> &[HostDecoded] {
        &self.decoded
    }

    /// The file a host decoded `sha256` to, as the last `finish` planned with it: the one its deck prints.
    pub fn decoded_file(&self, sha256: &Sha256) -> Option<MediaHandle> {
        let bytes = self.finished_files.get(sha256)?;
        Some(MediaHandle {
            bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            source: MediaSource::Bytes(Arc::clone(bytes)),
        })
    }

    /// Every font file the session holds: the source's embedded ones (when on) that could be read, then the host's,
    /// in the order they were added.
    pub fn added_fonts(&self) -> Vec<&AddedFont> {
        self.embedded().fonts.iter().chain(&self.fonts).collect()
    }

    /// The problems with the fonts the session holds: the embedded fonts' (in order), then the faces that were
    /// repaired and the faces that never bind, in the order their fonts were added.
    pub fn font_issues(&self) -> Vec<FontIssue> {
        let mut issues = self.embedded().issues.clone();
        for font in self.added_fonts() {
            for (index, face) in font.faces.iter().enumerate() {
                let refused = self.refusal(font, index);
                let (family, postscript_name, source) = (
                    face.family.clone(),
                    face.postscript_name.clone(),
                    font.source,
                );
                let repair = font.repair(index);
                if let Some(repair) = repair
                    && self.options.glyphs.is_some()
                {
                    issues.push(FontIssue::Repaired {
                        family: family.clone(),
                        postscript_name: postscript_name.clone(),
                        source,
                        repair,
                    });
                }
                match refused {
                    Some(FaceRefusal::Browser(reason)) => issues.push(FontIssue::BrowserRefused {
                        family,
                        postscript_name,
                        source,
                        reason: reason.clone(),
                    }),
                    Some(FaceRefusal::Licence(exclusion)) => {
                        issues.push(FontIssue::LicenceExcluded {
                            family,
                            postscript_name,
                            source,
                            bitmap_only: *exclusion == Exclusion::BitmapOnly,
                            unstated: repair == Some(FaceRepair::Os2),
                            in_preview: false,
                        });
                    }
                    None => {}
                }
            }
        }
        issues
    }

    /// Why face `index` of `font` never binds. Only a session that ships fonts refuses faces: without shipping (the
    /// parity path) the deck relies on the viewer's fonts, and every face binds as an installed font does.
    pub fn refusal<'f>(&self, font: &'f AddedFont, index: usize) -> Option<&'f FaceRefusal> {
        self.options.glyphs?;
        font.refused.get(index)?.as_ref()
    }

    /// Whether `finish` has run at least once.
    pub fn is_finished(&self) -> bool {
        self.finishes > 0
    }

    /// `finish` takes more fonts at any time; nothing needs reopening.
    pub fn resume(&mut self) -> Result<(), SessionError> {
        Ok(())
    }

    /// The presentation the passes before fonts handed on, if it did not fail as a whole.
    pub fn before_fonts(&self) -> Option<&Presentation> {
        self.before_fonts.as_ref().and_then(Progress::presentation)
    }

    /// Every diagnostic so far (the frontend's, verify-source's and the passes before fonts), in output order.
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        let mut all = self.opening.clone();
        if let Some(progress) = &self.before_fonts {
            all.extend(progress.diagnostics().iter().cloned());
        }
        let order = ContainerOrder::of(self.before_fonts().or(self.source.as_ref()));
        sort_diagnostics(all, &order)
    }

    fn embedded(&self) -> &Embedded {
        self.embedded_fonts.get_or_init(|| {
            read_embedded(
                &self.embedded,
                self.embedded_bytes.take(),
                self.options.unwrap,
                &self.options.limits,
                self.before_fonts(),
                self.options.target,
            )
        })
    }

    /// Every face of every font the session holds that may bind, as resolution candidates, with where each came from
    /// and its index in its file. The bundled floor is no source: its faces are no candidates.
    fn candidates(&self) -> (Vec<Candidate>, Vec<(&AddedFont, &SfntFace, u32)>) {
        let mut candidates = Vec::new();
        let mut faces = Vec::new();
        for font in self.added_fonts() {
            let Some(source) = font.source.kind() else {
                continue;
            };
            for (index, face) in font.faces.iter().enumerate() {
                if self.refusal(font, index).is_some() {
                    continue;
                }
                let traits = font.traits.get(index).cloned().unwrap_or_default();
                let mut candidate = Candidate {
                    source,
                    family: face.family.clone(),
                    legacy_family: traits.legacy_family,
                    postscript_name: face.postscript_name.clone(),
                    weight: traits.wght.unwrap_or(WeightRange::point(face.weight)),
                    style: style_of(face),
                    declared: Vec::new(),
                };
                if font.source == FontSource::Embedded {
                    declare(&mut candidate, font, index);
                }
                candidates.push(candidate);
                faces.push((font, face, u32::try_from(index).unwrap_or(u32::MAX)));
            }
        }
        (candidates, faces)
    }

    /// Every family the presentation uses, resolved slot by slot against the fonts the session holds.
    pub fn resolution(&self) -> Vec<ResolvedFamily> {
        let Some(presentation) = self.before_fonts() else {
            return Vec::new();
        };
        let uses: Vec<FontUse> = font_uses(presentation)
            .into_values()
            .map(|need| FontUse {
                family: need.family,
                faces: need.faces,
                postscript_names: need.postscript_names,
            })
            .collect();
        let (candidates, _) = self.candidates();
        resolve::resolve(&uses, &candidates, &self.options.font_order)
    }

    /// The families the presentation's text uses (runs, fields, styled breaks, paragraph marks, list labels) and
    /// the fonts it records, with the faces no source has exactly yet, what Google Fonts could fetch for them and
    /// what installed fonts are matched against. A family whose every face is filled is left out; a recorded family
    /// with no face is listed (with no faces) until some font names it.
    pub fn font_needs(&self) -> Vec<FontNeed> {
        self.font_needs_from(google_catalog())
    }

    fn font_needs_from(&self, catalog: &GoogleCatalog) -> Vec<FontNeed> {
        let Some(presentation) = self.before_fonts() else {
            return Vec::new();
        };
        let order = self.options.font_order;
        let (candidates, _) = self.candidates();
        let uses = font_uses(presentation);
        let mut needs = Vec::new();
        for (key, need) in uses {
            let font = FontUse {
                family: need.family.clone(),
                faces: need.faces.clone(),
                postscript_names: need.postscript_names.clone(),
            };
            if need.requested_none {
                let named = candidates
                    .iter()
                    .filter(|c| order.contains(c.source))
                    .any(|c| resolve::names_family(c, &key, &need.family));
                if !named {
                    needs.push(FontNeed {
                        google: order
                            .contains(SourceKind::Google)
                            .then(|| regular_plan(catalog, &need.family))
                            .flatten(),
                        local: order
                            .contains(SourceKind::ViewerInstalled)
                            .then(|| LocalQuery {
                                postscript_names: need.postscript_names.clone(),
                                families: vec![key.clone()],
                            }),
                        family: need.family,
                        key,
                        faces: Vec::new(),
                        postscript_names: need.postscript_names,
                        embedded: need.embedded,
                        slots: Vec::new(),
                    });
                }
                continue;
            }
            let resolved = resolve::resolve(std::slice::from_ref(&font), &candidates, &order);
            for family in resolve::font_needs(&resolved, &order, catalog) {
                let faces: Vec<FaceRequest> = family
                    .open_slots()
                    .map(|slot| slot.requested.clone())
                    .collect();
                needs.push(FontNeed {
                    family: need.family.clone(),
                    key: key.clone(),
                    faces,
                    postscript_names: need.postscript_names.clone(),
                    embedded: need.embedded,
                    slots: family.slots,
                    google: family.google,
                    local: family.local,
                });
            }
        }
        needs
    }

    /// The faces of a Google Fonts `css2` response (for a [`FontNeed::google`] request) worth fetching. A browser's
    /// response splits each face into per-script chunks: only the chunks whose `unicode-range` holds a code point
    /// the text draws with a family of that request are kept, so every chunk is fetched whole and the deck's text
    /// never leaves the host. A response without ranges (the CLI's static files) is kept whole.
    pub fn google_faces(&self, css: &str) -> Result<Vec<RemoteFace>, Css2Error> {
        let faces = parse_css2(css)?;
        let drawn = self.before_fonts().map(drawn_points).unwrap_or_default();
        let needs = self.font_needs();
        let points = |family: &str| -> BTreeSet<u32> {
            let google = FamilyKey::of(family);
            let keys: BTreeSet<&FamilyKey> = needs
                .iter()
                .filter(|need| {
                    need.google
                        .as_ref()
                        .is_some_and(|plan| FamilyKey::of(&plan.family) == google)
                })
                .map(|need| &need.key)
                .collect();
            drawn
                .iter()
                .filter(|(face, _)| keys.contains(&face.key))
                .flat_map(|(_, points)| points.iter().copied())
                .collect()
        };
        Ok(faces
            .into_iter()
            .filter(|face| match &face.unicode_range {
                Some(range) => range.intersects(points(&face.family)),
                None => true,
            })
            .collect())
    }

    /// The installed faces worth reading for the faces still open (Local Font Access metadata, or a system font
    /// scan's), at most [`resolve::MAX_LOCAL_PICKS`].
    pub fn select_local(&self, metas: &[LocalFontMeta]) -> Vec<LocalPick> {
        let Some(presentation) = self.before_fonts() else {
            return Vec::new();
        };
        let order = self.options.font_order;
        let uses: Vec<FontUse> = font_uses(presentation)
            .into_values()
            .map(|need| FontUse {
                family: need.family,
                faces: need.faces,
                postscript_names: need.postscript_names,
            })
            .collect();
        let (candidates, _) = self.candidates();
        let resolved = resolve::resolve(&uses, &candidates, &order);
        let needs = resolve::font_needs(&resolved, &order, google_catalog());
        resolve::select_local(metas, &needs)
    }

    /// Adds a font file (TrueType, OpenType, a collection, WOFF or WOFF2, or any container [`Containers`] reads)
    /// from `source`; returns how many faces it holds. In a session that ships fonts, installed files count against
    /// the session's budget for them ([`LocalBudget`]): a file holding a PostScript name the presentation asks for may
    /// be larger than one file's share. A session that ships nothing (the parity path) takes installed files as
    /// catalog fonts, as the CLI's `--fonts` folders, with no budget.
    pub fn add_font(&mut self, bytes: Vec<u8>, source: FontSource) -> Result<usize, SessionError> {
        self.add_font_file(bytes, source, None)
    }

    /// Adds one `unicode-range` chunk of a face, as [`Session::add_font`] adds a whole file: the chunks of a face
    /// (Google Fonts' per-script WOFF2 files for browsers) bind as one face, and each ships the code points of its
    /// range the deck draws.
    pub fn add_font_chunk(
        &mut self,
        bytes: Vec<u8>,
        source: FontSource,
        unicode_range: UnicodeRange,
    ) -> Result<usize, SessionError> {
        self.add_font_file(bytes, source, Some(unicode_range))
    }

    fn add_font_file(
        &mut self,
        bytes: Vec<u8>,
        source: FontSource,
        unicode_range: Option<UnicodeRange>,
    ) -> Result<usize, SessionError> {
        let limit = self.options.font_bytes();
        let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if size > limit {
            return Err(SessionError::FontTooLarge { bytes: size, limit });
        }
        let bytes = match container::sniff(&bytes, &Hint::default()) {
            Some(container::Format::Sfnt | container::Format::Collection) | None => bytes,
            Some(_) => {
                let unwrapped = container::unwrap(&bytes, &Hint::default())
                    .map_err(|error| SessionError::UnreadableContainer(error.to_string()))?;
                let unwrapped_size = u64::try_from(unwrapped.sfnt.len()).unwrap_or(u64::MAX);
                if unwrapped_size > limit {
                    return Err(SessionError::FontTooLarge {
                        bytes: unwrapped_size,
                        limit,
                    });
                }
                unwrapped.sfnt
            }
        };
        let (bytes, repairs) = repaired(bytes);
        let faces = read_sfnt(&bytes)?;
        if source == FontSource::Local && self.options.glyphs.is_some() {
            let asked = self.asked_postscript_names();
            let exact = faces
                .iter()
                .any(|face| asked.contains(face.postscript_name.as_str()));
            self.local.spend(size, exact)?;
        }
        let count = faces.len();
        self.fonts.push(AddedFont {
            unicode_range,
            ..added_font(
                source,
                bytes,
                faces,
                repairs,
                Vec::new(),
                self.options.target,
            )
        });
        Ok(count)
    }

    /// The PostScript names the presentation's fonts record.
    fn asked_postscript_names(&self) -> BTreeSet<String> {
        self.before_fonts()
            .map(font_uses)
            .into_iter()
            .flat_map(BTreeMap::into_values)
            .flat_map(|need| need.postscript_names)
            .collect()
    }

    /// The catalog the passes after fonts bind against: the base fonts, then the bundled substitutes' files, then
    /// every face of the sources that are on, so that of equal faces the one resolution chose wins. A variable face
    /// carries its `wght` axis and its metrics at each weight the presentation draws.
    ///
    /// A session that ships fonts measures with the files it ships: of the base fonts it keeps only the bundled
    /// families, with the floor table's (the floor, metrics only until their files arrive), and of a family any file
    /// provides, only the files' faces. A family no file provides then binds to its substitute or its generic family, as an absent one
    /// does.
    pub fn catalog(&self) -> FontCatalog {
        let order = self.options.font_order;
        let (candidates, faces) = self.candidates();
        let resolved = self.resolution();
        let weights: Vec<u32> = resolved
            .iter()
            .flat_map(|family| &family.slots)
            .map(|slot| slot.weight)
            .collect::<BTreeSet<u32>>()
            .into_iter()
            .collect();
        let limits = FontLimits::default();
        let bundled = self
            .added_fonts()
            .into_iter()
            .filter(|font| font.source == FontSource::Bundled);
        let bundled = FontCatalog::build(bundled.flat_map(|font| {
            font.faces
                .iter()
                .enumerate()
                .filter(|&(index, _)| self.refusal(font, index).is_none())
                .map(|(index, face)| {
                    let index = u32::try_from(index).unwrap_or(u32::MAX);
                    (
                        face.family.clone(),
                        CatalogFace::from_file(face, &font.bytes, index, &weights, &limits)
                            .with_file(FaceFile::new(font.bytes.clone(), index)),
                    )
                })
        }));
        let entries = resolve::catalog_entries(&resolved, &candidates, &order);
        let added = FontCatalog::build(entries.iter().filter_map(|entry| {
            let (font, face, index) = faces.get(entry.candidate)?;
            Some((
                entry.family.clone(),
                CatalogFace::from_file(face, &font.bytes, *index, &weights, &limits)
                    .with_file(FaceFile::new(font.bytes.clone(), *index)),
            ))
        }));
        let files = FontCatalog::merge([&bundled, &added]);
        if self.options.glyphs.is_none() {
            return FontCatalog::merge([self.options.fonts, &files]);
        }
        let mut refused: Vec<(&str, String)> = Vec::new();
        for font in self.added_fonts() {
            for (index, face) in font.faces.iter().enumerate() {
                if let Some(refusal) = self.refusal(font, index) {
                    let reason = refusal.reason(font.repair(index));
                    refused.push((&face.family, reason.clone()));
                    refused.push((&face.postscript_name, reason));
                }
            }
        }
        let base = self.options.fonts;
        let floor = FontCatalog::build(
            base.families()
                .filter(|(key, _)| is_bundled_family(key))
                .chain(
                    floor_catalog()
                        .families()
                        .filter(|(key, _)| base.get(key).is_none()),
                )
                .filter(|(key, _)| files.get(key).is_none())
                .flat_map(|(_, family)| {
                    family
                        .faces
                        .iter()
                        .map(|face| (family.family.as_str(), face.clone()))
                }),
        );
        FontCatalog::merge([&floor, &files]).with_refused(refused)
    }

    /// The slots resolution left to an embedded file that failed its identity check: the requested family (by key),
    /// the drawn weight, italic, and the PostScript name of the file's face.
    fn demoted(&self) -> DemotedSlots {
        let (_, faces) = self.candidates();
        let mut demoted = DemotedSlots::new();
        for family in self.resolution() {
            for slot in &family.slots {
                if let resolve::Fill::Demoted { candidate, .. } = slot.fill
                    && let Some((_, face, _)) = faces.get(candidate)
                {
                    demoted.insert((
                        family.key.clone(),
                        slot.weight,
                        slot.style == FaceStyle::Italic,
                        face.postscript_name.clone(),
                    ));
                }
            }
        }
        demoted
    }

    /// Whether the session keeps the native motion data ([`SessionOptions::native_motion`]).
    pub fn native_motion(&self) -> bool {
        self.options.native_motion
    }

    /// The session reads the source's embedded fonts and ships the faces drawn with them.
    pub fn ships_embedded_fonts(&self) -> bool {
        self.options.embedded_fonts && self.options.glyphs.is_some()
    }

    /// Runs the passes from bind-fonts on with the fonts held so far, then verify-resolved. Callable again after
    /// more fonts arrive: it starts from the passes before fonts each time.
    pub fn finish(&mut self) -> Result<Finished, SessionError> {
        let catalog = self.catalog();
        let mut font_issues = self.font_issues();
        let options = &self.options;
        let env = PassEnv {
            limits: &options.limits,
            fonts: &catalog,
            policy: &options.policy,
            facts: &self.facts,
            host_media: &self.decoded,
        };
        let run = options.run(env, self.deadline);
        let mut collected = self.opening.clone();
        let mut finished = Finished {
            source: self.source.clone(),
            presentation: None,
            diagnostics: Vec::new(),
            font_issues: Vec::new(),
            fonts: Vec::new(),
            substitutes: Vec::new(),
            failures: Vec::new(),
            stats: None,
            trace: None,
        };
        if let (Some(source), Some(before)) = (&self.source, &self.before_fonts) {
            let manager = options.pipeline.manager();
            let mut progress = before.clone();
            let range = options.pipeline.font_boundary()..manager.passes().len();
            manager.advance(&mut progress, range, options.verify, &run);
            let result = manager.conclude(progress, source, &run);
            collected.extend(result.diagnostics);
            finished.presentation = result.presentation;
            finished.failures = result.failures;
            finished.stats = result.timings.map(|passes| Stats { passes });
            finished.trace = result.trace;
        }
        let order = match &finished.presentation {
            Some(presentation) => ContainerOrder::of_resolved(presentation),
            None => ContainerOrder::of(finished.source.as_ref()),
        };
        finished.diagnostics = sort_diagnostics(collected, &order);
        if let (Some(glyphs), Some(presentation)) = (options.glyphs, &finished.presentation) {
            match glyphs.collect(presentation, &catalog) {
                Ok(uses) => {
                    let demoted = self.demoted();
                    let added = self.added_fonts();
                    let shipping = ship_faces(
                        &added,
                        &uses,
                        &demoted,
                        &options.font_order,
                        google_catalog(),
                    );
                    finished.fonts = shipping.faces;
                    finished.substitutes = substitutes(&uses);
                    if (shipping.wants_last_resort || may_want_last_resort(&uses))
                        && !holds_last_resort(&added)
                    {
                        add_last_resort(&mut finished.substitutes);
                    }
                    font_issues.extend(shipping.issues);
                }
                Err(reason) => font_issues.push(FontIssue::GlyphsFailed { reason }),
            }
        }
        finished.font_issues = font_issues;
        self.finished_files = self.decoded_files.clone();
        self.finishes += 1;
        Ok(finished)
    }
}

/// A face of a bundled substitute family the deck draws with and no file provides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubstituteFace {
    pub family: String,
    /// The face's PostScript name, as the base catalog records it: the file to add.
    pub postscript_name: String,
    pub weight: u32,
    pub style: FaceStyle,
}

/// One of the families the bundled substitutes provide: a metric-compatible stand-in, or the family a CSS generic
/// renders with.
pub fn is_bundled_family(key: &FamilyKey) -> bool {
    let generics = [
        GenericFamily::SansSerif,
        GenericFamily::Serif,
        GenericFamily::Monospace,
        GenericFamily::Cursive,
        GenericFamily::Fantasy,
    ];
    SUBSTITUTES
        .iter()
        .map(|&(_, family)| family)
        .chain(generics.map(generic_family))
        .any(|family| FamilyKey::of(family) == *key)
}

/// A face the deck draws with that waits for its bundled file draws code points some bundled face may lack, so the
/// last-resort face may be needed once the file arrives.
fn may_want_last_resort(uses: &[GlyphUse]) -> bool {
    uses.iter()
        .filter(|glyph| glyph.sha256.is_none() && is_bundled_family(&FamilyKey::of(&glyph.family)))
        .any(|glyph| {
            glyph.codepoints.iter().any(|point| {
                !bundled_covers(point) && !draws_no_glyph(point) && !glyph.forced.contains(point)
            })
        })
}

/// The session holds the bundled last-resort file.
fn holds_last_resort(fonts: &[&AddedFont]) -> bool {
    let (family, postscript_name) = LAST_RESORT;
    fonts.iter().any(|font| {
        font.source == FontSource::Bundled
            && font
                .faces
                .iter()
                .any(|face| face.family == family && face.postscript_name == postscript_name)
    })
}

/// Names the last-resort face among the bundled faces to add, once.
fn add_last_resort(substitutes: &mut Vec<SubstituteFace>) {
    let (family, postscript_name) = LAST_RESORT;
    if substitutes
        .iter()
        .any(|face| face.postscript_name == postscript_name)
    {
        return;
    }
    substitutes.push(SubstituteFace {
        family: family.to_owned(),
        postscript_name: postscript_name.to_owned(),
        weight: 400,
        style: FaceStyle::Normal,
    });
}

/// The faces of bundled families the deck draws with that have no file, each once, in the order drawn.
fn substitutes(uses: &[GlyphUse]) -> Vec<SubstituteFace> {
    let mut faces: Vec<SubstituteFace> = Vec::new();
    for glyph in uses {
        if glyph.sha256.is_none()
            && is_bundled_family(&FamilyKey::of(&glyph.family))
            && !faces
                .iter()
                .any(|face| face.postscript_name == glyph.postscript_name)
        {
            faces.push(SubstituteFace {
                family: glyph.family.clone(),
                postscript_name: glyph.postscript_name.clone(),
                weight: glyph.weight,
                style: if glyph.italic {
                    FaceStyle::Italic
                } else {
                    FaceStyle::Normal
                },
            });
        }
    }
    faces
}

/// The file with the faces browsers would refuse repaired where that is safe, and what each face had repaired.
fn repaired(bytes: Vec<u8>) -> (Vec<u8>, Vec<Option<FaceRepair>>) {
    match repair(&bytes, &FontLimits::default()) {
        Some(Repaired { bytes, faces }) => (bytes, faces),
        None => (bytes, Vec::new()),
    }
}

fn added_font(
    source: FontSource,
    bytes: Vec<u8>,
    faces: Vec<SfntFace>,
    repairs: Vec<Option<FaceRepair>>,
    declared: Vec<Declared>,
    target: Target,
) -> AddedFont {
    let indexes = || (0..faces.len()).map(|index| u32::try_from(index).unwrap_or(u32::MAX));
    let traits = indexes().map(|index| face_traits(&bytes, index)).collect();
    let refused = indexes()
        .map(|index| face_refusal(&bytes, index, source, target))
        .collect();
    AddedFont {
        source,
        sha256: Sha256::of(&bytes),
        faces,
        traits,
        declared,
        subset: false,
        missing: Vec::new(),
        refused,
        repairs,
        unicode_range: None,
        bytes: bytes.into(),
    }
}

/// Why face `index` of a font from `source` never binds in `target`: browsers would refuse it, or its `OS/2.fsType`
/// keeps it out of `target` (the licence policy; the user's per-family toggle is the bundle's, and binds nothing
/// differently).
fn face_refusal(
    bytes: &[u8],
    index: u32,
    source: FontSource,
    target: Target,
) -> Option<FaceRefusal> {
    if let Some(refusal) = sanitizer_refusal(bytes, index) {
        return Some(FaceRefusal::Browser(refusal.to_string()));
    }
    let fs_type = match read_details(bytes, index) {
        Ok(details) => details.fs_type,
        Err(error) => return Some(FaceRefusal::Browser(error.to_string())),
    };
    let policy = LicencePolicy::default();
    match decide(
        source.origin(),
        FsType(fs_type),
        &policy,
        target,
        Toggle::Default,
    ) {
        Decision::Exclude(exclusion @ (Exclusion::Restricted | Exclusion::BitmapOnly)) => {
            Some(FaceRefusal::Licence(exclusion))
        }
        Decision::Ship | Decision::ShipWithNotice | Decision::Exclude(Exclusion::LeftOut) => None,
    }
}

/// An embedded face as a candidate checks it: its identity against each declared face, then a subset file's coverage.
fn declare(candidate: &mut Candidate, font: &AddedFont, face: usize) {
    resolve::declare(candidate, font.declared.iter().cloned());
    if let Some(missing) = font.missing.get(face) {
        resolve::require_coverage(candidate, missing);
    }
}

/// An embedded font's bytes at open: a range of the input is copied (the input is not kept), bytes the frontend
/// holds are kept, and a reader (an entry still compressed) is left for first use. Over `limit`, or a copy over
/// what is left of `budget`, is refused unread.
fn read_at_open(
    font: &EmbeddedFont,
    input: &dyn RangeReader,
    limit: u64,
    budget: &mut u64,
) -> EmbeddedBytes {
    let too_large = |bytes: u64, limit: u64| {
        EmbeddedBytes::Refused(FontIssue::EmbeddedTooLarge {
            family: font.family.clone(),
            bytes,
            limit,
        })
    };
    match &font.source {
        MediaSource::InputRange { offset, len } => {
            if *len > limit || *len > *budget {
                return too_large(*len, limit.min(*budget));
            }
            let read = usize::try_from(*len)
                .map_err(|error| error.to_string())
                .and_then(|len| {
                    input
                        .read_at(*offset, len)
                        .map_err(|error| error.to_string())
                });
            match read {
                Ok(bytes) => {
                    *budget -= *len;
                    EmbeddedBytes::Ready(Arc::from(bytes.into_owned()))
                }
                Err(reason) => EmbeddedBytes::Refused(FontIssue::EmbeddedUnreadable {
                    family: font.family.clone(),
                    reason,
                }),
            }
        }
        MediaSource::Bytes(bytes) => {
            let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
            if size > limit {
                return too_large(size, limit);
            }
            EmbeddedBytes::Ready(bytes.clone())
        }
        MediaSource::Reader(reader) => {
            if reader.len() > limit {
                return too_large(reader.len(), limit);
            }
            EmbeddedBytes::Deferred(reader.clone())
        }
    }
}

/// Reads, unwraps and identifies the embedded fonts, within the per-file limit and the archive's memory budget. Each
/// font's bytes are dropped once it is unwrapped, so while one is unwrapped the budget holds the files kept, the bytes
/// still to unwrap, the font's own bytes (twice for a reader, which may hold its own copy) and the result.
fn read_embedded(
    fonts: &[EmbeddedFontInfo],
    bytes: Vec<EmbeddedBytes>,
    unwrap: &dyn FontUnwrap,
    limits: &Limits,
    presentation: Option<&Presentation>,
    target: Target,
) -> Embedded {
    let size_of = |bytes: &EmbeddedBytes| match bytes {
        EmbeddedBytes::Ready(bytes) => u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        EmbeddedBytes::Deferred(reader) => reader.len(),
        EmbeddedBytes::Refused(_) => 0,
    };
    let mut embedded = Embedded::default();
    let mut pending: u64 = bytes.iter().map(size_of).fold(0, u64::saturating_add);
    let mut kept: u64 = 0;
    let points = OnceCell::new();
    for (font, bytes) in fonts.iter().zip(bytes) {
        let raw_size = size_of(&bytes);
        pending = pending.saturating_sub(raw_size);
        let too_large = |bytes: u64, left: u64| FontIssue::EmbeddedTooLarge {
            family: font.family.clone(),
            bytes,
            limit: limits.zip.entry_bytes.min(left),
        };
        let unreadable = |reason: String| FontIssue::EmbeddedUnreadable {
            family: font.family.clone(),
            reason,
        };
        if let EmbeddedBytes::Refused(issue) = bytes {
            embedded.issues.push(issue);
            continue;
        }
        let reading = match &bytes {
            EmbeddedBytes::Deferred(_) => raw_size.saturating_mul(2),
            _ => raw_size,
        };
        let held = kept.saturating_add(pending);
        let Some(left) = limits
            .zip
            .memory_bytes
            .checked_sub(held)
            .and_then(|left| left.checked_sub(reading))
        else {
            let left = limits.zip.memory_bytes.saturating_sub(held);
            embedded.issues.push(too_large(raw_size, left));
            continue;
        };
        let unwrapped = match &bytes {
            EmbeddedBytes::Ready(raw) => unwrap.unwrap_font(raw, font.container),
            EmbeddedBytes::Deferred(reader) => {
                let read = usize::try_from(raw_size)
                    .map_err(|error| error.to_string())
                    .and_then(|len| reader.read_at(0, len).map_err(|error| error.to_string()));
                match read {
                    Ok(raw) => unwrap.unwrap_font(&raw, font.container),
                    Err(reason) => {
                        embedded.issues.push(unreadable(reason));
                        continue;
                    }
                }
            }
            EmbeddedBytes::Refused(_) => continue,
        };
        drop(bytes);
        let UnwrappedFont { sfnt, subset } = match unwrapped {
            Ok(unwrapped) => unwrapped,
            Err(reason) => {
                embedded.issues.push(unreadable(reason));
                continue;
            }
        };
        let size = u64::try_from(sfnt.len()).unwrap_or(u64::MAX);
        if size > limits.zip.entry_bytes || size > left {
            embedded.issues.push(too_large(size, left));
            continue;
        }
        let (sfnt, repairs) = repaired(sfnt);
        let faces = match read_sfnt(&sfnt) {
            Ok(faces) => faces,
            Err(error) => {
                embedded.issues.push(unreadable(error.to_string()));
                continue;
            }
        };
        kept += size;
        let declared: Vec<Declared> = font
            .faces
            .iter()
            .map(|face| Declared {
                family: font.family.clone(),
                weight: face.weight,
                style: face.style,
            })
            .collect();
        let mut added = added_font(FontSource::Embedded, sfnt, faces, repairs, declared, target);
        if subset {
            let points = points.get_or_init(|| presentation.map(drawn_points).unwrap_or_default());
            added.subset = true;
            added.missing = (0..added.faces.len())
                .map(|face| {
                    let index = u32::try_from(face).unwrap_or(u32::MAX);
                    added
                        .declared
                        .iter()
                        .map(|declared| {
                            let drawn = points.get(&DrawnFace::of(declared));
                            missing_code_points(
                                &added.bytes,
                                index,
                                drawn.into_iter().flatten().copied(),
                            )
                        })
                        .collect()
                })
                .collect();
        }
        embedded.issues.extend(mislabeled(&added));
        embedded.fonts.push(added);
    }
    embedded
}

/// A declared face no face of the file passes the identity check for.
fn mislabeled(font: &AddedFont) -> Vec<FontIssue> {
    let checked: Vec<Candidate> = font
        .faces
        .iter()
        .enumerate()
        .map(|(index, face)| {
            let traits = font.traits.get(index).cloned().unwrap_or_default();
            let mut candidate = Candidate {
                source: SourceKind::Embedded,
                family: face.family.clone(),
                legacy_family: traits.legacy_family,
                postscript_name: face.postscript_name.clone(),
                weight: traits.wght.unwrap_or(WeightRange::point(face.weight)),
                style: style_of(face),
                declared: Vec::new(),
            };
            declare(&mut candidate, font, index);
            candidate
        })
        .collect();
    font.declared
        .iter()
        .enumerate()
        .filter_map(|(slot, declared)| {
            let identities: Vec<(&Candidate, &Identity)> = checked
                .iter()
                .filter_map(|c| c.declared.get(slot).map(|(_, identity)| (c, identity)))
                .collect();
            if identities
                .iter()
                .any(|(_, identity)| identity.is_accepted())
            {
                return None;
            }
            let (candidate, identity) = identities.first()?;
            let Identity::Demoted(mismatch) = identity else {
                return None;
            };
            Some(match mismatch {
                Mismatch::Subset { missing } => FontIssue::EmbeddedSubset {
                    family: declared.family.clone(),
                    weight: declared.weight,
                    style: declared.style,
                    missing: *missing,
                },
                mismatch => FontIssue::EmbeddedMislabeled {
                    family: declared.family.clone(),
                    weight: declared.weight,
                    style: declared.style,
                    file_family: candidate.family.clone(),
                    file_weight: candidate.weight.min,
                    file_italic: candidate.style == FaceStyle::Italic,
                    mismatch: mismatch.clone(),
                },
            })
        })
        .collect()
}

/// The Google Fonts request for a family's regular face.
fn regular_plan(catalog: &GoogleCatalog, family: &str) -> Option<GooglePlan> {
    let found = catalog.find(family)?;
    let slots = [(400, FaceStyle::Normal)];
    Some(GooglePlan {
        family: found.family.family.clone(),
        licence: found.family.licence,
        faces: slidock_fonts::google::css2_faces(catalog, family, &slots),
        url: slidock_fonts::google::css2_url(catalog, family, &slots)?,
    })
}

fn style_of(face: &SfntFace) -> FaceStyle {
    if face.italic {
        FaceStyle::Italic
    } else {
        FaceStyle::Normal
    }
}

fn style_name(style: FaceStyle) -> &'static str {
    match style {
        FaceStyle::Normal => "normal",
        FaceStyle::Italic => "italic",
    }
}

/// A family the presentation uses or records.
#[derive(Default)]
struct Use {
    family: String,
    faces: BTreeMap<(u32, &'static str), FaceRequest>,
    postscript_names: Vec<String>,
    embedded: bool,
    /// The source records the family without naming a face.
    requested_none: bool,
}

struct Need {
    family: String,
    faces: Vec<FaceRequest>,
    postscript_names: Vec<String>,
    embedded: bool,
    requested_none: bool,
}

/// Every run of the presentation's text with its text (runs, fields, and styled breaks and paragraph marks with none),
/// then every list label's family and text.
fn walk_text(
    presentation: &Presentation,
    mut run: impl FnMut(&PartialRunProps, &str),
    mut label: impl FnMut(&str, &str),
) {
    let mut labels: Vec<(&str, &str)> = Vec::new();
    for container in presentation.containers() {
        for element in walk_elements(container.elements()) {
            for (text, _) in element_texts(element) {
                for paragraph in &text.paragraphs {
                    for inline in &paragraph.inlines {
                        match inline {
                            Inline::Run { props, text, .. } => run(props, text),
                            Inline::Field { props, cached, .. } => run(props, cached),
                            Inline::Break { props: Some(props) } => run(props, ""),
                            Inline::Break { props: None } | Inline::Tab {} => {}
                        }
                    }
                    if let Some(end) = &paragraph.end_props {
                        run(end, "");
                    }
                    if let Some(Bullet::Label { font, text, .. }) = &paragraph.props.bullet {
                        labels.push((&font.family, text));
                    }
                }
            }
        }
    }
    for (family, text) in labels {
        label(family, text);
    }
}

fn run_style(props: &PartialRunProps) -> FaceStyle {
    if props.italic.unwrap_or(false) {
        FaceStyle::Italic
    } else {
        FaceStyle::Normal
    }
}

/// A face as the code points a subset file must hold are kept: the family, the side of the bold line, the slant.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct DrawnFace {
    key: FamilyKey,
    bold: bool,
    italic: bool,
}

impl DrawnFace {
    fn new(family: &str, weight: u32, style: FaceStyle) -> Self {
        DrawnFace {
            key: FamilyKey::of(family),
            bold: weight >= resolve::identity::BOLD_LINE,
            italic: style == FaceStyle::Italic,
        }
    }

    fn of(declared: &Declared) -> Self {
        DrawnFace::new(&declared.family, declared.weight, declared.style)
    }
}

/// The code points the text draws with each face, white space and controls left out.
fn drawn_points(presentation: &Presentation) -> BTreeMap<DrawnFace, BTreeSet<u32>> {
    let mut points: BTreeMap<DrawnFace, BTreeSet<u32>> = BTreeMap::new();
    let mut add = |face: DrawnFace, text: &str| {
        let drawn = text
            .chars()
            .filter(|c| !c.is_whitespace() && !c.is_control())
            .map(u32::from);
        points.entry(face).or_default().extend(drawn);
    };
    let mut labels: Vec<(DrawnFace, String)> = Vec::new();
    walk_text(
        presentation,
        |props, text| {
            if let Some(FontRef::Family(font)) = &props.font
                && !text.is_empty()
            {
                let face =
                    DrawnFace::new(&font.family, props.weight.unwrap_or(400), run_style(props));
                add(face, text);
            }
        },
        |family, text| {
            labels.push((
                DrawnFace::new(family, 400, FaceStyle::Normal),
                text.to_owned(),
            ));
        },
    );
    for (face, text) in labels {
        add(face, &text);
    }
    points
}

/// Every family the presentation's text uses and its records name, by key, but the emoji families: no file is wanted
/// for them, as the viewer's emoji font draws them.
fn font_uses(presentation: &Presentation) -> BTreeMap<FamilyKey, Need> {
    let mut uses: BTreeMap<FamilyKey, Use> = BTreeMap::new();
    let mut add = |family: &str, weight: u32, style: FaceStyle| {
        let entry = uses.entry(FamilyKey::of(family)).or_default();
        if entry.family.is_empty() {
            entry.family = family.to_owned();
        }
        entry
            .faces
            .insert((weight, style_name(style)), FaceRequest { weight, style });
    };
    let mut labels: Vec<String> = Vec::new();
    walk_text(
        presentation,
        |props, _| {
            if let Some(FontRef::Family(font)) = &props.font {
                add(&font.family, props.weight.unwrap_or(400), run_style(props));
            }
        },
        |family, _| labels.push(family.to_owned()),
    );
    for family in labels {
        add(&family, 400, FaceStyle::Normal);
    }
    for (key, entry) in &presentation.fonts {
        let request = &entry.request;
        let slot = uses.entry(FamilyKey::of(key)).or_default();
        slot.family = request.family.clone();
        slot.embedded = request.embedded;
        slot.postscript_names = request.postscript_names.clone();
        slot.requested_none = request.faces.is_empty();
        for face in &request.faces {
            slot.faces
                .insert((face.weight, style_name(face.style)), face.clone());
        }
    }
    uses.into_iter()
        .filter(|(key, _)| !is_emoji_family(key))
        .map(|(key, used)| {
            let family = if used.family.is_empty() {
                key.as_str().to_owned()
            } else {
                used.family
            };
            let need = Need {
                family,
                requested_none: used.requested_none && used.faces.is_empty(),
                faces: used.faces.into_values().collect(),
                postscript_names: used.postscript_names,
                embedded: used.embedded,
            };
            (key, need)
        })
        .collect()
}
