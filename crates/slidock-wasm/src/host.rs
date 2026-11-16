//! What the compile worker runs, with no JavaScript in it: one open presentation at a time, driven stage by stage
//! (`open` → `font_needs` / `add_font` and `media_needs` / `add_media` → `finish` → `print` → `bundle`),
//! reading its input only through a
//! [`RangeReader`]. The wasm bindings wrap it for the worker; native tests drive it directly.
//!
//! Each stage is the [`slidock`] facade's: the same session options, generator and bundle path as
//! [`slidock::compile`], so a deck bundled here is byte for byte the deck the CLI writes.

use std::cell::OnceCell;
use std::collections::BTreeMap;
use std::sync::Arc;

use slidock::alternates::{
    AlternateMedia, AlternateVideo, alternate_sources, check_alternate, plan_alternates,
};
use slidock::deck::{Deck, DeckDiagnostic};
use slidock::fonts::{bundled_faces, with_fonts};
use slidock::images::{
    CompressedImage, CompressedMedia, DrawnSizes, ImagePlan, check_image, drawn_sizes, plan_images,
    replacements,
};
use slidock::shipping::ShippedFace;
use slidock::videos::{CompressedVideo, VideoPlan, check_video, plan_videos, video_replacements};
use slidock::{
    BundleError, BundleOptions, BundleReport, BundleSink, CompileError, CompiledDeck, DeckMedia,
    Diagnostic, Documents, FaceRefusal, FinishedDeck, FontIssue, FontNeed, FontSource,
    FrontendRegistry, Input, MediaAction, MediaNeed, MediaPolicy, MediaSource, Packaging, Pipeline,
    RangeReader, Session, SessionError, SessionMedia, SessionOptions, Sha256, SlideTemplate,
    StoreMedia, SubstituteFace, finish_deck, slidock_generator, write_media_file,
};
use slidock_fonts::google::{RemoteFace, UnicodeRange};
use slidock_fonts::policy::Exclusion;
use slidock_fonts::resolve::{LocalFontMeta, LocalPick};
use slidock_fonts::{Decision, FaceRepair, FontFaceRule, Target, Toggle};
use slidock_ir::fonts::FaceStyle;
use thiserror::Error;

use crate::protocol::{ErrorKind, FontReport, ServedFace, WorkerError};

/// The most bytes one output chunk holds: every chunk of a bundle but the last is exactly this long.
pub const CHUNK_BYTES: usize = 1024 * 1024;

/// Why a stage failed: its [`ErrorKind`], a message for a bug report and the diagnostics behind it.
#[derive(Clone, Debug, PartialEq, Error)]
#[error("{kind:?}: {message}")]
pub struct HostError {
    pub kind: ErrorKind,
    pub message: String,
    pub diagnostics: Vec<Diagnostic>,
}

impl HostError {
    fn new(kind: ErrorKind, message: impl ToString) -> HostError {
        HostError {
            kind,
            message: message.to_string(),
            diagnostics: Vec::new(),
        }
    }

    fn request(message: &str) -> HostError {
        HostError::new(ErrorKind::Request, message)
    }
}

impl From<&HostError> for WorkerError {
    fn from(error: &HostError) -> Self {
        WorkerError {
            kind: error.kind,
            message: error.message.clone(),
            diagnostics: error.diagnostics.iter().map(DeckDiagnostic::from).collect(),
        }
    }
}

/// A session's error: misuse is a request out of order, an unreadable font a file that is no font.
fn session_error(error: SessionError) -> HostError {
    HostError::new(ErrorKind::Request, error)
}

/// A media need's id: its SHA-256 in lowercase hex.
fn need_id(need: &str) -> Result<Sha256, HostError> {
    Sha256::parse(need).map_err(|_| HostError::request(&format!("{need} is no media need")))
}

/// The frontends and the pass pipeline, built once per thread and kept for its lifetime: a session borrows them.
struct Shared {
    registry: FrontendRegistry,
    pipeline: Pipeline,
}

fn shared() -> Result<&'static Shared, HostError> {
    thread_local! {
        static SHARED: OnceCell<&'static Shared> = const { OnceCell::new() };
    }
    SHARED.with(|cell| {
        if let Some(shared) = cell.get() {
            return Ok(*shared);
        }
        let internal = |error: &dyn std::fmt::Display| HostError::new(ErrorKind::Internal, error);
        let shared: &'static Shared = Box::leak(Box::new(Shared {
            registry: slidock::Frontends::default()
                .registry()
                .map_err(|error| internal(&error))?,
            pipeline: slidock::pipeline().map_err(|error| internal(&error))?,
        }));
        Ok(*cell.get_or_init(|| shared))
    })
}

/// What `open` found.
#[derive(Clone, Debug, PartialEq)]
pub struct Opened {
    /// The frontend's, verify-source's and the passes' before fonts, in output order.
    pub diagnostics: Vec<Diagnostic>,
    pub needs: Vec<FontNeed>,
    /// The media the host may decode (HEIC photos), with their plans.
    pub media: Vec<OpenedMedia>,
}

/// A medium the host may decode, as the worker reports it.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenedMedia {
    /// The source item's: the need's id.
    pub sha256: Sha256,
    pub mime: String,
    /// Where the source file's bytes are: a range of the input, or bytes [`Host::need_bytes`] reads.
    pub bytes: MediaBytes,
    /// The decode plan, or why there is none.
    pub plan: Result<slidock_io::HeifImage, String>,
}

impl From<&MediaNeed> for OpenedMedia {
    fn from(need: &MediaNeed) -> Self {
        OpenedMedia {
            sha256: need.sha256,
            mime: need.mime.clone(),
            bytes: match need.source.source {
                MediaSource::InputRange { offset, len } => MediaBytes::Range { offset, len },
                MediaSource::Bytes(_) | MediaSource::Reader(_) => MediaBytes::Written,
            },
            plan: need.plan.clone().map_err(|error| error.to_string()),
        }
    }
}

/// Where a printed medium's (or a media need's source) bytes are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaBytes {
    /// A range of the input, copied as it is: the host slices its own copy of the input.
    Range { offset: u64, len: u64 },
    /// Bytes the compiler writes ([`Host::media_bytes`]): a transcoded raster, or a medium the input holds deflated.
    Written,
}

/// A file under `media/` of a printed deck.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrintedMedia {
    /// `media/<sha256>.<ext>`.
    pub path: String,
    pub mime: String,
    pub bytes: MediaBytes,
}

/// A face of a font the session holds that never binds: browsers would refuse it, or its `OS/2.fsType` keeps it
/// out of the preview (bitmap-only embedding). Its substitute draws instead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefusedFace {
    /// As the file names it.
    pub family: String,
    pub postscript_name: String,
    pub weight: u32,
    pub style: FaceStyle,
    pub source: FontSource,
    pub refusal: FaceRefusal,
    pub repair: Option<FaceRepair>,
}

/// A finished deck, printed: what the app shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Printed {
    /// The deck JSON, with no player stamp.
    pub deck: Deck,
    /// Each slide composed with its layout, in show order.
    pub slides: Vec<SlideTemplate>,
    pub media: Vec<PrintedMedia>,
    /// Every diagnostic of the compile and the printer, in output order.
    pub diagnostics: Vec<Diagnostic>,
    /// The faces the deck ships (none when the presentation was opened without fonts).
    pub fonts: Vec<ShippedFace>,
    /// The faces of the fonts held that never bind.
    pub refused: Vec<RefusedFace>,
    /// The problems with the fonts held, as `finish` met them.
    pub font_issues: Vec<FontIssue>,
    /// The bundled substitute faces the deck draws with no file yet.
    pub substitutes: Vec<SubstituteFace>,
}

impl Printed {
    /// The shipped faces' `@font-face` rules, in face order: what `font_bytes` indexes.
    pub fn font_rules(&self) -> impl Iterator<Item = &FontFaceRule> {
        self.fonts.iter().flat_map(|face| &face.rules)
    }

    /// The fonts panel's faces: every shipped face, then every face of the fonts held that never binds.
    pub fn faces(&self) -> Vec<FontReport> {
        let shipped = self.fonts.iter().map(|face| FontReport {
            family: face.licence.family.clone(),
            postscript_name: face.postscript_name.clone(),
            weight: face.weight,
            style: face.style,
            source: face.source.into(),
            shipped: true,
            bytes: face.bytes,
            subset: face.subset,
            demoted: face.demoted,
            restricted: matches!(
                face.licence.download(Toggle::Default),
                Decision::Exclude(Exclusion::Restricted | Exclusion::BitmapOnly)
            ),
            unstated: face.repair == Some(FaceRepair::Os2),
            repaired: face.repair.map(|repair| repair.table().to_owned()),
            refused: None,
            notice: face.notice,
            serves: face
                .serves
                .iter()
                .map(|served| ServedFace {
                    family: served.family.clone(),
                    weight: served.weight,
                    style: served.style,
                })
                .collect(),
            licence: Some(face.licence.text()),
        });
        let refused = self.refused.iter().map(|face| FontReport {
            family: face.family.clone(),
            postscript_name: face.postscript_name.clone(),
            weight: face.weight,
            style: face.style,
            source: face.source.into(),
            shipped: false,
            bytes: 0,
            subset: false,
            demoted: false,
            restricted: matches!(face.refusal, FaceRefusal::Licence(_)),
            unstated: face.repair == Some(FaceRepair::Os2)
                && matches!(face.refusal, FaceRefusal::Licence(_)),
            repaired: face.repair.map(|repair| repair.table().to_owned()),
            refused: Some(face.refusal.reason(face.repair)),
            notice: false,
            serves: Vec::new(),
            licence: None,
        });
        shipped.chain(refused).collect()
    }
}

/// An open presentation.
struct OpenDeck {
    session: Session<'static>,
    /// The input, which the session's media ranges point into.
    reader: Box<dyn RangeReader>,
    /// Finished, not printed yet.
    finished: Option<slidock::Finished>,
    /// The last printed deck: what `bundle` writes.
    printed: Option<FinishedDeck>,
    /// The faces the last printed deck ships.
    shipped: Vec<ShippedFace>,
    /// The largest box each medium of the last printed deck is drawn in.
    drawn: DrawnSizes,
    /// The pictures of the printed deck the host may re-encode, as `image_plan` last listed them.
    images: Vec<ImagePlan>,
    /// The host's re-encoded pictures taken since, by original path: what a bundle with compressed images writes.
    compressed: BTreeMap<String, CompressedImage>,
    /// The videos of the printed deck, as `video_plan` last listed them.
    videos: Vec<VideoPlan>,
    /// The host's re-encoded videos taken since, by original path: what a bundle with compressed videos writes.
    compressed_videos: BTreeMap<String, CompressedVideo>,
    /// The videos of the printed deck that get a copy, as `alternate_plan` last listed them.
    alternate_plans: Vec<VideoPlan>,
    /// The host's copies taken since, by original path: what a bundle with alternates writes beside the originals.
    alternates: BTreeMap<String, AlternateVideo>,
}

/// What a bundle writes in place of the deck's media: the pictures and the videos the host re-encoded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Swaps {
    pub images: bool,
    pub videos: bool,
    /// The copies `add_alternate` took, written beside their originals.
    pub alternates: bool,
}

/// The files a bundle swaps in, by original path.
struct Swapped {
    images: BTreeMap<String, CompressedImage>,
    videos: BTreeMap<String, CompressedVideo>,
    alternates: BTreeMap<String, AlternateVideo>,
}

impl OpenDeck {
    fn printed(&self) -> Result<&FinishedDeck, HostError> {
        self.printed
            .as_ref()
            .ok_or_else(|| HostError::request("finish the presentation first"))
    }

    fn media(&self, path: &str) -> Result<Vec<u8>, HostError> {
        let plan =
            self.printed()?.documents.media.get(path).ok_or_else(|| {
                HostError::request(&format!("{path} is not a medium of the deck"))
            })?;
        // A copy is as long as its source; a transcode grows the buffer as it writes.
        let copied = match plan.action {
            MediaAction::Copy => self.session.media(&plan.sha256).map(|handle| handle.bytes),
            MediaAction::Transcode => self
                .session
                .decoded_file(&plan.sha256)
                .map(|handle| handle.bytes),
        };
        let mut bytes = Vec::with_capacity(
            copied
                .and_then(|len| usize::try_from(len).ok())
                .unwrap_or(0),
        );
        write_media_file(
            plan,
            &SessionMedia(&self.session),
            self.reader.as_ref(),
            MediaPolicy::default().transcode_pixels,
            &mut bytes,
        )
        .map_err(|error| HostError::new(ErrorKind::Internal, error))?;
        Ok(bytes)
    }

    /// Runs `f` on the printed deck with its media read from the input, the `compressed` pictures and videos in
    /// place of theirs.
    fn with_compiled<T>(
        &mut self,
        compressed: Option<&Swapped>,
        f: impl FnOnce(&CompiledDeck<'_>) -> T,
    ) -> Result<T, HostError> {
        let FinishedDeck {
            deck,
            documents,
            diagnostics,
        } = self
            .printed
            .take()
            .ok_or_else(|| HostError::request("finish the presentation first"))?;
        let store = SessionMedia(&self.session);
        let files = StoreMedia::new(
            &store,
            self.reader.as_ref(),
            &documents,
            MediaPolicy::default().transcode_pixels,
        );
        let swapped = compressed.map(|swapped| CompressedMedia {
            media: &files,
            images: &swapped.images,
            videos: &swapped.videos,
        });
        let media: &dyn DeckMedia = match &swapped {
            Some(swapped) => swapped,
            None => &files,
        };
        let with_alternates = compressed.map(|swapped| AlternateMedia {
            media,
            alternates: &swapped.alternates,
        });
        let compiled = CompiledDeck {
            deck,
            documents,
            media: match &with_alternates {
                Some(alternates) => alternates,
                None => media,
            },
            fonts: bundled_faces(&self.shipped),
        };
        let result = f(&compiled);
        let CompiledDeck {
            deck, documents, ..
        } = compiled;
        self.printed = Some(FinishedDeck {
            deck,
            documents,
            diagnostics,
        });
        Ok(result)
    }
}

/// The compile worker's state: at most one open presentation.
#[derive(Default)]
pub struct Host {
    open: Option<OpenDeck>,
}

impl std::fmt::Debug for Host {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Host")
            .field("open", &self.open.as_ref().map(|open| &open.session))
            .finish()
    }
}

impl Host {
    pub fn new() -> Host {
        Host::default()
    }

    fn deck(&self) -> Result<&OpenDeck, HostError> {
        self.open
            .as_ref()
            .ok_or_else(|| HostError::request("no presentation is open"))
    }

    fn deck_mut(&mut self) -> Result<&mut OpenDeck, HostError> {
        self.open
            .as_mut()
            .ok_or_else(|| HostError::request("no presentation is open"))
    }

    /// Opens `reader` as the presentation `name`, replacing any open one: sniffs, lowers, verifies and runs the
    /// passes that need no font. With `fonts`, the presentation's embedded fonts are added (as the CLI's deck path
    /// adds them) and `finish` ships the faces the deck draws; without, it binds against the built-in fonts only and
    /// ships nothing (the parity path, `--emit documents`).
    pub fn open(
        &mut self,
        name: &str,
        reader: Box<dyn RangeReader>,
        fonts: bool,
    ) -> Result<Opened, HostError> {
        self.open = None;
        let shared = shared()?;
        // The app shows the deck on the computer that holds its fonts; its download leaves out what a preview may draw.
        let options = if fonts {
            SessionOptions {
                target: Target::Preview,
                ..with_fonts(&shared.registry, &shared.pipeline)
            }
        } else {
            SessionOptions::new(&shared.registry, &shared.pipeline)
        };
        let session =
            Session::open(&Input::new(name, reader.as_ref()), options).map_err(|error| {
                let (kind, message) = match &error.refusal {
                    Some(refusal) => (ErrorKind::Refused, refusal.message.clone()),
                    None => (ErrorKind::Failed, "the input cannot be opened".to_owned()),
                };
                HostError {
                    kind,
                    message,
                    diagnostics: error.diagnostics,
                }
            })?;
        let opened = Opened {
            diagnostics: session.diagnostics(),
            needs: session.font_needs(),
            media: session
                .media_needs(reader.as_ref())
                .iter()
                .map(OpenedMedia::from)
                .collect(),
        };
        self.open = Some(OpenDeck {
            session,
            reader,
            finished: None,
            printed: None,
            shipped: Vec::new(),
            drawn: DrawnSizes::default(),
            images: Vec::new(),
            compressed: BTreeMap::new(),
            videos: Vec::new(),
            compressed_videos: BTreeMap::new(),
            alternate_plans: Vec::new(),
            alternates: BTreeMap::new(),
        });
        Ok(opened)
    }

    /// The faces no font provides yet.
    pub fn font_needs(&self) -> Result<Vec<FontNeed>, HostError> {
        Ok(self.deck()?.session.font_needs())
    }

    /// Adds a font file for the next `finish` (reopening a finished session); returns what is still missing. With
    /// `unicode_range` (a CSS `unicode-range` value), the file is one per-script chunk of a face, as Google Fonts
    /// serves browsers: the chunks of a face bind as one face and ship together.
    pub fn add_font(
        &mut self,
        bytes: Vec<u8>,
        source: FontSource,
        unicode_range: Option<&str>,
    ) -> Result<Vec<FontNeed>, HostError> {
        let range = unicode_range
            .map(|value| {
                UnicodeRange::parse(value)
                    .filter(|range| !range.is_empty())
                    .ok_or_else(|| HostError::request(&format!("{value} is not a unicode-range")))
            })
            .transpose()?;
        let deck = self.deck_mut()?;
        if deck.session.is_finished() {
            deck.session.resume().map_err(session_error)?;
        }
        match range {
            Some(range) => deck.session.add_font_chunk(bytes, source, range),
            None => deck.session.add_font(bytes, source),
        }
        .map_err(session_error)?;
        Ok(deck.session.font_needs())
    }

    /// The media the host may decode (HEIC photos), each with its decode plan or why it has none.
    pub fn media_needs(&self) -> Result<Vec<OpenedMedia>, HostError> {
        let deck = self.deck()?;
        Ok(deck
            .session
            .media_needs(deck.reader.as_ref())
            .iter()
            .map(OpenedMedia::from)
            .collect())
    }

    /// The bytes of the source file of the media need `need` (a SHA-256 in hex): what its plan's ranges index.
    pub fn need_bytes(&self, need: &str) -> Result<Vec<u8>, HostError> {
        let deck = self.deck()?;
        let sha256 = need_id(need)?;
        let handle = deck
            .session
            .media_needs(deck.reader.as_ref())
            .iter()
            .find(|found| found.sha256 == sha256)
            .map(|found| found.source.clone())
            .ok_or_else(|| HostError::request(&format!("{need} is no media need")))?;
        let read = |error: slidock::RangeError| HostError::new(ErrorKind::Internal, error);
        let len = usize::try_from(handle.bytes)
            .map_err(|_| HostError::request(&format!("media {need} is too large to read")))?;
        Ok(match handle.source {
            MediaSource::InputRange { offset, len } => deck
                .reader
                .read_at(offset, len as usize)
                .map_err(read)?
                .into_owned(),
            MediaSource::Bytes(bytes) => bytes.to_vec(),
            MediaSource::Reader(reader) => reader.read_at(0, len).map_err(read)?.into_owned(),
        })
    }

    /// Adds the host's decode of the media need `need` (a PNG or JPEG of its plan's output size) for the next
    /// `finish`; a file of another type or size is refused, and the medium stays omitted.
    pub fn add_media(&mut self, need: &str, bytes: Vec<u8>) -> Result<(), HostError> {
        let sha256 = need_id(need)?;
        let deck = self.deck_mut()?;
        if deck.session.is_finished() {
            deck.session.resume().map_err(session_error)?;
        }
        deck.session
            .add_media(&sha256, bytes, deck.reader.as_ref())
            .map_err(session_error)?;
        Ok(())
    }

    /// The installed faces (Local Font Access metadata) worth reading for the faces still open, by index into
    /// `fonts`.
    pub fn select_local(&self, fonts: &[LocalFontMeta]) -> Result<Vec<LocalPick>, HostError> {
        Ok(self.deck()?.session.select_local(fonts))
    }

    /// The faces of a Google Fonts `css2` response worth fetching: of a browser's response, the per-script chunks
    /// that hold a code point the text draws (each fetched whole: the deck's text never leaves the browser).
    pub fn google_faces(&self, css: &str) -> Result<Vec<RemoteFace>, HostError> {
        self.deck()?
            .session
            .google_faces(css)
            .map_err(|error| HostError::request(&error.to_string()))
    }

    /// Runs the passes from bind-fonts with the fonts added so far; again after more fonts.
    pub fn finish(&mut self) -> Result<(), HostError> {
        let deck = self.deck_mut()?;
        if deck.session.is_finished() {
            deck.session.resume().map_err(session_error)?;
        }
        let finished = deck.session.finish().map_err(session_error)?;
        deck.finished = Some(finished);
        Ok(())
    }

    /// Prints what `finish` resolved: the deck JSON, each slide composed with its layout, and where each medium's
    /// bytes are. The printed deck replaces the one `bundle` writes.
    pub fn print(&mut self) -> Result<Printed, HostError> {
        let deck = self.deck_mut()?;
        let finished = deck
            .finished
            .take()
            .ok_or_else(|| HostError::request("finish the presentation before printing it"))?;
        let FinishedDeck {
            deck: json,
            documents,
            diagnostics,
        } = finish_deck(&deck.session, &finished, slidock_generator()).map_err(
            |error| match error {
                CompileError::Failed { diagnostics } => HostError {
                    kind: ErrorKind::Failed,
                    message: "the presentation did not compile".to_owned(),
                    diagnostics,
                },
                other => HostError::new(ErrorKind::Internal, other),
            },
        )?;
        let mut media = Vec::with_capacity(documents.media.len());
        for (path, plan) in &documents.media {
            let range = match (plan.action, deck.session.media(&plan.sha256)) {
                (MediaAction::Copy, Some(handle)) => match handle.source {
                    MediaSource::InputRange { offset, len } => {
                        Some(MediaBytes::Range { offset, len })
                    }
                    MediaSource::Bytes(_) | MediaSource::Reader(_) => None,
                },
                _ => None,
            };
            media.push(PrintedMedia {
                path: path.clone(),
                mime: plan.mime.as_str().to_owned(),
                bytes: range.unwrap_or(MediaBytes::Written),
            });
        }
        let session = &deck.session;
        let refused = session
            .added_fonts()
            .into_iter()
            .flat_map(|font| {
                font.faces
                    .iter()
                    .enumerate()
                    .filter_map(move |(index, face)| {
                        let refusal = session.refusal(font, index);
                        Some(RefusedFace {
                            family: face.family.clone(),
                            postscript_name: face.postscript_name.clone(),
                            weight: face.weight,
                            style: if face.italic {
                                FaceStyle::Italic
                            } else {
                                FaceStyle::Normal
                            },
                            source: font.source,
                            refusal: refusal?.clone(),
                            repair: font.repair(index),
                        })
                    })
            })
            .collect();
        let printed = Printed {
            deck: json.clone(),
            slides: Vec::new(),
            media,
            diagnostics: diagnostics.clone(),
            fonts: finished.fonts.clone(),
            refused,
            font_issues: finished.font_issues.clone(),
            substitutes: finished.substitutes.clone(),
        };
        deck.printed = Some(FinishedDeck {
            deck: json,
            documents,
            diagnostics,
        });
        deck.shipped = finished.fonts;
        deck.drawn = finished
            .presentation
            .as_ref()
            .map(drawn_sizes)
            .unwrap_or_default();
        deck.images.clear();
        deck.compressed.clear();
        deck.videos.clear();
        deck.compressed_videos.clear();
        deck.alternate_plans.clear();
        deck.alternates.clear();
        let slides = deck
            .with_compiled(None, slidock::slide_templates)?
            .map_err(|error| HostError::new(ErrorKind::Internal, error))?;
        Ok(Printed { slides, ..printed })
    }

    /// The document set of the last printed deck (`--emit documents`), for parity checks.
    pub fn documents(&self) -> Result<&Documents, HostError> {
        Ok(&self.deck()?.printed()?.documents)
    }

    /// The file of the printed deck's `index`th shipped `@font-face` rule ([`Printed::font_rules`]).
    pub fn font_bytes(&self, index: usize) -> Result<Vec<u8>, HostError> {
        self.deck()?
            .shipped
            .iter()
            .flat_map(|face| &face.rules)
            .nth(index)
            .map(|rule| rule.bytes.clone())
            .ok_or_else(|| HostError::request(&format!("the deck ships no font file {index}")))
    }

    /// The bytes of the printed deck's file `path` under `media/`, copied or transcoded.
    pub fn media_bytes(&self, path: &str) -> Result<Vec<u8>, HostError> {
        self.deck()?.media(path)
    }

    /// The pictures of the printed deck the host may re-encode smaller for a download, each with the size to encode
    /// it at (`slidock::images`). It forgets the pictures taken before.
    pub fn image_plan(&mut self) -> Result<Vec<ImagePlan>, HostError> {
        let deck = self.deck_mut()?;
        let store = SessionMedia(&deck.session);
        let images = plan_images(
            &deck.printed()?.documents,
            &deck.drawn,
            &store,
            deck.reader.as_ref(),
            MediaPolicy::default().transcode_pixels,
        );
        deck.images.clone_from(&images);
        deck.compressed.clear();
        Ok(images)
    }

    /// Takes the host's re-encode of the planned picture `path` for the bundles with compressed images, replacing
    /// an earlier one; a file `slidock::images::check_image` refuses is a request error, and the picture stays as it
    /// is.
    pub fn add_image(&mut self, path: &str, bytes: Vec<u8>) -> Result<(), HostError> {
        let deck = self.deck_mut()?;
        let plan = deck
            .images
            .iter()
            .find(|plan| plan.path == path)
            .ok_or_else(|| HostError::request(&format!("{path} is no planned picture")))?;
        let image = check_image(plan, bytes)
            .map_err(|refusal| HostError::request(&format!("{path}: {refusal}")))?;
        deck.compressed.insert(path.to_owned(), image);
        Ok(())
    }

    /// The videos of the printed deck, each with the size to encode it at or why it stays as it is
    /// (`slidock::videos`). It forgets the videos taken before.
    pub fn video_plan(&mut self) -> Result<Vec<VideoPlan>, HostError> {
        let deck = self.deck_mut()?;
        let store = SessionMedia(&deck.session);
        let videos = plan_videos(
            &deck.printed()?.documents,
            &deck.drawn,
            &store,
            deck.reader.as_ref(),
            MediaPolicy::default().transcode_pixels,
        );
        deck.videos.clone_from(&videos);
        deck.compressed_videos.clear();
        Ok(videos)
    }

    /// Takes the host's re-encode of the planned video `path`, read from `reader` (its movie boxes now, the rest as a
    /// bundle writes it), for the bundles with compressed videos, replacing an earlier one; a file
    /// `slidock::videos::check_video` refuses is a request error, and the video stays as it is.
    pub fn add_video(&mut self, path: &str, reader: Box<dyn RangeReader>) -> Result<(), HostError> {
        let deck = self.deck_mut()?;
        let plan = deck
            .videos
            .iter()
            .find(|plan| plan.path == path)
            .ok_or_else(|| HostError::request(&format!("{path} is no planned video")))?;
        let video = check_video(plan, Arc::from(reader))
            .map_err(|refusal| HostError::request(&format!("{path}: {refusal}")))?;
        deck.compressed_videos.insert(path.to_owned(), video);
        Ok(())
    }

    /// The videos of the printed deck that get a copy other browsers play, each with the size to make it at
    /// (`slidock::alternates`). It forgets the copies taken before.
    pub fn alternate_plan(&mut self) -> Result<Vec<VideoPlan>, HostError> {
        let deck = self.deck_mut()?;
        let store = SessionMedia(&deck.session);
        let videos = plan_videos(
            &deck.printed()?.documents,
            &deck.drawn,
            &store,
            deck.reader.as_ref(),
            MediaPolicy::default().transcode_pixels,
        );
        let plans = plan_alternates(&videos);
        deck.alternate_plans.clone_from(&plans);
        deck.alternates.clear();
        Ok(plans)
    }

    /// Takes the host's copy of the video `path`, read from `reader` (its element headers now, the rest as a bundle
    /// writes it), for the bundles with alternates, replacing an earlier one; a file
    /// `slidock::alternates::check_alternate` refuses is a request error, and the video has no copy.
    pub fn add_alternate(
        &mut self,
        path: &str,
        reader: Box<dyn RangeReader>,
    ) -> Result<(), HostError> {
        let deck = self.deck_mut()?;
        let plan = deck
            .alternate_plans
            .iter()
            .find(|plan| plan.path == path)
            .ok_or_else(|| HostError::request(&format!("{path} is no video that gets a copy")))?;
        let video = check_alternate(plan, Arc::from(reader))
            .map_err(|refusal| HostError::request(&format!("{path}: {refusal}")))?;
        deck.alternates.insert(path.to_owned(), video);
        Ok(())
    }

    /// Bundles the last printed deck as a zip, handing `out` its bytes in order, in chunks of [`CHUNK_BYTES`] (the
    /// last one shorter). The media are read from the input by range as the zip reaches them; with `swaps`, the
    /// pictures `add_image` and the videos `add_video` took are written in their originals' place, and the copies
    /// `add_alternate` took beside theirs.
    pub fn bundle(
        &mut self,
        options: &BundleOptions,
        swaps: Swaps,
        out: &mut dyn FnMut(&[u8]) -> Result<(), BundleError>,
    ) -> Result<BundleReport, HostError> {
        let deck = self.deck_mut()?;
        let swapped = Swapped {
            images: if swaps.images {
                std::mem::take(&mut deck.compressed)
            } else {
                BTreeMap::new()
            },
            videos: if swaps.videos {
                std::mem::take(&mut deck.compressed_videos)
            } else {
                BTreeMap::new()
            },
            alternates: if swaps.alternates {
                std::mem::take(&mut deck.alternates)
            } else {
                BTreeMap::new()
            },
        };
        let mut replaced = replacements(&swapped.images);
        replaced.extend(video_replacements(&swapped.videos));
        let options = BundleOptions {
            packaging: Packaging::Zip,
            replaced,
            alternates: alternate_sources(&swapped.alternates),
            ..options.clone()
        };
        let mut chunks = Chunks::new(out);
        let any = swaps.images || swaps.videos || swaps.alternates;
        let result = deck.with_compiled(any.then_some(&swapped), |compiled| {
            let report = slidock::bundle(compiled, &options, &mut chunks)?;
            chunks.flush()?;
            Ok(report)
        });
        let Swapped {
            images,
            videos,
            alternates,
        } = swapped;
        if swaps.images {
            deck.compressed = images;
        }
        if swaps.videos {
            deck.compressed_videos = videos;
        }
        if swaps.alternates {
            deck.alternates = alternates;
        }
        result?.map_err(|error: BundleError| HostError::new(ErrorKind::Bundle, error))
    }
}

/// A sink that hands on its bytes in chunks of exactly [`CHUNK_BYTES`], and the rest on [`Chunks::flush`].
pub struct Chunks<'o> {
    buffer: Vec<u8>,
    out: &'o mut dyn FnMut(&[u8]) -> Result<(), BundleError>,
}

impl<'o> Chunks<'o> {
    pub fn new(out: &'o mut dyn FnMut(&[u8]) -> Result<(), BundleError>) -> Chunks<'o> {
        Chunks {
            buffer: Vec::with_capacity(CHUNK_BYTES),
            out,
        }
    }

    /// Hands on what is buffered, if anything.
    pub fn flush(&mut self) -> Result<(), BundleError> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        (self.out)(&self.buffer)?;
        self.buffer.clear();
        Ok(())
    }
}

impl BundleSink for Chunks<'_> {
    fn write(&mut self, _path: &str, mut bytes: &[u8]) -> Result<(), BundleError> {
        while !bytes.is_empty() {
            let room = CHUNK_BYTES - self.buffer.len();
            let (head, rest) = bytes.split_at(room.min(bytes.len()));
            self.buffer.extend_from_slice(head);
            bytes = rest;
            if self.buffer.len() == CHUNK_BYTES {
                self.flush()?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunked(writes: &[usize]) -> Vec<usize> {
        let mut sizes = Vec::new();
        let mut all = Vec::new();
        let mut out = |chunk: &[u8]| {
            sizes.push(chunk.len());
            all.extend_from_slice(chunk);
            Ok(())
        };
        let mut chunks = Chunks::new(&mut out);
        let mut written = Vec::new();
        for (index, &len) in writes.iter().enumerate() {
            let bytes: Vec<u8> = (0..len).map(|i| (i + index) as u8).collect();
            chunks.write("a", &bytes).unwrap();
            written.extend_from_slice(&bytes);
        }
        chunks.flush().unwrap();
        drop(chunks);
        assert_eq!(all, written);
        sizes
    }

    #[test]
    fn hands_on_whole_chunks_then_the_rest() {
        // The worker posts the zip in chunks of 1 MiB.
        assert_eq!(chunked(&[3 << 20, 5]), [1 << 20, 1 << 20, 1 << 20, 5]);
        assert_eq!(chunked(&[]), Vec::<usize>::new());
        assert_eq!(chunked(&[10, 20]), [30]);
        assert_eq!(chunked(&[CHUNK_BYTES]), [CHUNK_BYTES]);
        assert_eq!(
            chunked(&[CHUNK_BYTES - 1, 2, 3 * CHUNK_BYTES]),
            [CHUNK_BYTES, CHUNK_BYTES, CHUNK_BYTES, CHUNK_BYTES, 1]
        );
    }

    #[test]
    fn stops_at_the_first_refused_chunk() {
        let mut calls = 0;
        let mut out = |_: &[u8]| {
            calls += 1;
            Err(BundleError::Cancelled)
        };
        let mut chunks = Chunks::new(&mut out);
        assert_eq!(
            chunks.write("a", &vec![0; 2 * CHUNK_BYTES]),
            Err(BundleError::Cancelled)
        );
        drop(chunks);
        assert_eq!(calls, 1);
    }

    #[test]
    fn refuses_stages_out_of_order() {
        let mut host = Host::new();
        let kind = |error: HostError| error.kind;
        assert_eq!(host.font_needs().map_err(kind), Err(ErrorKind::Request));
        assert_eq!(host.finish().map_err(kind), Err(ErrorKind::Request));
        assert_eq!(host.print().map_err(kind), Err(ErrorKind::Request));
        assert_eq!(
            host.bundle(&BundleOptions::default(), Swaps::default(), &mut |_| Ok(()))
                .map_err(kind),
            Err(ErrorKind::Request)
        );
    }
}
