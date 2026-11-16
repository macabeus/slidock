//! The messages between the app and the compile worker, as data. The TypeScript declarations in
//! `packages/types/src/generated/worker` are generated from these types; `packages/wasm/README.md` describes the
//! conversation.
//!
//! Every request carries an `id` the app picks, and everything the worker sends about a request carries it back.
//! The worker runs one request at a time, each stage as one synchronous call into the compiler: it reads no message
//! while a stage runs, so progress is per stage and `cancel` reaches only requests that have not started.
//! Fields whose TypeScript type is a browser object (`File`, `Blob`, `ArrayBuffer`) cross `postMessage` as
//! themselves, never as JSON: in Rust they are [`JsObject`] placeholders, and the worker's JavaScript side sets
//! them.

use serde::{Deserialize, Serialize};
use slidock::deck::{Deck, DeckDiagnostic};
use slidock::{BundleReport, NotesMode, PageDefaults, SlideTemplate};
use slidock_fonts::catalog::substitutes::substitute;
use slidock_fonts::face_css::weight_value;
use slidock_ir::fonts::{FaceRequest, FaceStyle};
use ts_rs::TS;

/// A browser object a message carries as itself (structured clone or transfer): its TypeScript type is given where
/// a field holds it. Rust never holds the object; it serialises as `null`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsObject;

/// Where a font file comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "worker/")]
pub enum AddedFontSource {
    /// Installed on this computer (Local Font Access).
    Local,
    /// Fetched from Google Fonts.
    Google,
    /// A file the user dropped.
    User,
    /// Embedded in the presentation (`open` adds those itself).
    Embedded,
    /// One of the bundled substitutes (a deck's `substitutes`): the floor under every source.
    Bundled,
}

impl From<AddedFontSource> for slidock::FontSource {
    fn from(source: AddedFontSource) -> Self {
        match source {
            AddedFontSource::Local => slidock::FontSource::Local,
            AddedFontSource::Google => slidock::FontSource::Google,
            AddedFontSource::User => slidock::FontSource::User,
            AddedFontSource::Embedded => slidock::FontSource::Embedded,
            AddedFontSource::Bundled => slidock::FontSource::Bundled,
        }
    }
}

impl From<slidock::FontSource> for AddedFontSource {
    fn from(source: slidock::FontSource) -> Self {
        match source {
            slidock::FontSource::Local => AddedFontSource::Local,
            slidock::FontSource::Google => AddedFontSource::Google,
            slidock::FontSource::User => AddedFontSource::User,
            slidock::FontSource::Embedded => AddedFontSource::Embedded,
            slidock::FontSource::Bundled => AddedFontSource::Bundled,
        }
    }
}

/// How the app wants the deck bundled.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(export_to = "worker/")]
pub struct BundleSettings {
    /// Inline the media into `index.html` (when they fit under the threshold).
    pub single_file: bool,
    /// In bytes; `null` for the default (50 MiB).
    pub single_file_threshold: Option<u64>,
    /// Families to leave out of the download (the fonts panel's per-family toggle), as the deck's `fonts` name
    /// them: they ship no file, and the report lists them in `fontsLeftOut`.
    pub leave_out: Vec<String>,
    /// Families to include in the download whatever their faces' restricted `fsType` says (the same toggle): their
    /// faces from this computer or the user ship, with the notice. `leaveOut` wins over it.
    pub include: Vec<String>,
    /// Write the pictures `addImage` took since the last `imagePlan` in their originals' place; absent or `false`
    /// writes the originals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub compress_images: Option<bool>,
    /// Write the videos `addVideo` took since the last `videoPlan` in their originals' place; absent or `false`
    /// writes the originals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub compress_videos: Option<bool>,
    /// The page's title; absent for the deck's `page.title` default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub title: Option<String>,
    /// The page's description, `""` for none; absent for the deck's `page.description` default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub description: Option<String>,
    /// The deck's public URL, for its canonical link: an absolute `http(s)` URL with no user info and no fragment, or
    /// the bundle fails.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub site_url: Option<String>,
    /// Ship the presenter view; absent for `true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub presenter: Option<bool>,
    /// Which speaker notes ship, and where; absent for `presenter`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub notes: Option<NotesPlacement>,
    /// Write the text version of the slides below them; absent for `true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub text_view: Option<bool>,
    /// Write the copies `addAlternate` took since the last `alternatePlan` beside their originals, each video
    /// offering both; absent or `false` writes none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub alternate_videos: Option<bool>,
}

/// Which speaker notes a bundle carries, and where (`slidock::NotesMode`): they ship only where the page shows them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "worker/")]
pub enum NotesPlacement {
    /// In the presenter view, when the page ships one.
    Presenter,
    /// In the presenter view and on the page, in the text view.
    Page,
    /// Nowhere.
    None,
}

impl From<NotesPlacement> for NotesMode {
    fn from(notes: NotesPlacement) -> Self {
        match notes {
            NotesPlacement::Presenter => NotesMode::Presenter,
            NotesPlacement::Page => NotesMode::Page,
            NotesPlacement::None => NotesMode::None,
        }
    }
}

impl From<&BundleSettings> for slidock::BundleOptions {
    fn from(settings: &BundleSettings) -> Self {
        let defaults = slidock::BundleOptions::default();
        slidock::BundleOptions {
            single_file: settings.single_file,
            single_file_threshold: settings
                .single_file_threshold
                .unwrap_or(defaults.single_file_threshold),
            leave_out: settings.leave_out.iter().cloned().collect(),
            include: settings.include.iter().cloned().collect(),
            text_view: settings.text_view.unwrap_or(defaults.text_view),
            presenter: settings.presenter.unwrap_or(defaults.presenter),
            notes: settings.notes.map_or(defaults.notes, NotesMode::from),
            title: settings.title.clone(),
            description: settings.description.clone(),
            site_url: settings.site_url.clone(),
            ..defaults
        }
    }
}

/// App → worker.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export_to = "worker/")]
pub enum ToWorker {
    /// Opens a presentation, replacing any open one: sniffs, lowers, verifies and runs the passes that need no
    /// font, reading the file by ranges. Answered by `progress`, `diagnostics`, `needs`, `mediaNeeds`, then `done`.
    Open {
        id: u32,
        #[ts(type = "File")]
        file: JsObject,
        /// Takes the presentation's embedded fonts and ships the faces the deck draws (the default); `false` binds
        /// against the built-in fonts only and ships nothing, as `slidock compile --emit documents` does.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        fonts: Option<bool>,
    },
    /// Asks which faces no font provides yet. Answered by `needs`, then `done`.
    FontsNeeded { id: u32 },
    /// Adds a font file (`.ttf`, `.otf`, `.ttc`, `.woff`, `.woff2`) for the next `finish`. Answered by `needs` (what
    /// is still missing), then `done`; a file that is no font is an `error`.
    AddFont {
        id: u32,
        #[ts(type = "ArrayBuffer")]
        bytes: JsObject,
        source: AddedFontSource,
        /// For one per-script chunk of a face (a `googleFaces` face with a range): its `unicode-range`. The chunks of
        /// a face bind as one face and ship together.
        #[serde(
            default,
            rename = "unicodeRange",
            skip_serializing_if = "Option::is_none"
        )]
        #[ts(optional)]
        unicode_range: Option<String>,
    },
    /// Chooses which installed faces to read (Local Font Access metadata, `queryLocalFonts()`) for the faces still
    /// open. Answered by `localPicks`, then `done`.
    SelectLocal { id: u32, fonts: Vec<LocalFontMeta> },
    /// Chooses the faces of a Google Fonts `css2` response (fetched from a need's `google.url`) to fetch: the
    /// per-script chunks that hold a code point the text draws. Answered by `googleFaces`, then `done`.
    GoogleFaces { id: u32, css: String },
    /// Adds the app's decode of a media need (a `mediaNeeds` entry with a plan): a PNG or JPEG of the plan's output
    /// size, for the next `finish`, which then shows the medium. Answered by `done`; a file of another type or size
    /// is an `error` of kind `request`, and the medium stays omitted.
    AddMedia {
        id: u32,
        /// The need's `id`.
        need: String,
        #[ts(type = "ArrayBuffer")]
        bytes: JsObject,
    },
    /// Runs the passes from bind-fonts with the fonts added so far and prints the deck; again after more fonts, to
    /// swap the slides in place. Answered by `progress`, `diagnostics`, `deck`, then `done`.
    Finish { id: u32 },
    /// Lists the pictures of the last finished deck the app may re-encode smaller for a download, forgetting those
    /// it re-encoded before. Answered by `imagePlan`, then `done`.
    ImagePlan { id: u32 },
    /// Hands over the app's re-encode of a planned picture (its `path`): WebP, JPEG or PNG of the planned size,
    /// stored as it shows and smaller than the original, for the bundles with `compressImages`. Answered by `done`;
    /// another file is an `error` of kind `request`, and the picture stays as it is.
    AddImage {
        id: u32,
        path: String,
        #[ts(type = "ArrayBuffer")]
        bytes: JsObject,
    },
    /// Lists the videos of the last finished deck, each with the size the app may re-encode it at or why it stays
    /// as it is, forgetting those it re-encoded before. Answered by `videoPlan`, then `done`.
    VideoPlan { id: u32 },
    /// Hands over the app's re-encode of a planned video (its `path`): an MP4 with its movie boxes first, one H.264
    /// track of the planned size with the original's frames, the original's sound as AAC (or none when it has none, or
    /// when it is never heard), as long as the original and smaller, for the bundles with `compressVideos`. A video
    /// whose sound is never heard may instead be the original's video track copied as it is, without the sound. The worker reads the file as it checks and bundles it. Answered
    /// by `done`; another file is an `error` of kind `request`, and the video stays as it is.
    AddVideo {
        id: u32,
        path: String,
        #[ts(type = "Blob")]
        file: JsObject,
    },
    /// Lists the videos of the last finished deck that get a copy other browsers play (kept for their alpha), each
    /// with the size to make it at, forgetting the copies taken before. Answered by `alternatePlan`, then `done`.
    AlternatePlan { id: u32 },
    /// Hands over the app's copy of a video (its `path`): a WebM of one VP9 or VP8 track with WebM's alpha, of the
    /// planned size, its cues first, with the original's frames and length, for the bundles with `alternateVideos`.
    /// The worker reads the file as it checks and bundles it. Answered by `done`; another file is an `error` of kind
    /// `request`, and the video has no copy.
    AddAlternate {
        id: u32,
        path: String,
        #[ts(type = "Blob")]
        file: JsObject,
    },
    /// Bundles the last finished deck as a zip. Answered by `progress` and `zipChunk`s, then `done` with the report.
    Bundle { id: u32, settings: BundleSettings },
    /// Asks how much memory the compiler holds. Answered by `memory`, then `done`.
    Memory { id: u32 },
    /// Drops the request `id` if it has not started: it ends with an `error` of kind `cancelled`. A request that has
    /// started runs to its end (terminate the worker to stop it, losing the open deck). Not a request itself: it
    /// gets no reply of its own.
    Cancel { id: u32 },
}

/// A stage of a request, each one synchronous call into the compiler.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "worker/")]
pub enum Phase {
    /// Reading and lowering the input (`open`).
    Open,
    /// The passes from bind-fonts (`finish`).
    Finish,
    /// Printing and composing the slides (`finish`).
    Print,
    /// Writing the zip (`bundle`); its `zipChunk`s show how far it is.
    Bundle,
}

/// The Google Fonts request for a family's open faces (open-licence catalog families only).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(export_to = "worker/")]
pub struct GooglePlan {
    /// As the catalog spells it.
    pub family: String,
    /// The family's licence, as an SPDX id.
    pub licence: String,
    /// The faces requested, in `css2` order.
    pub faces: Vec<FaceRequest>,
    /// The `css2` URL: no `text=`, so it names the family and faces only.
    pub url: String,
}

/// What installed faces (Local Font Access) are matched against.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(export_to = "worker/")]
pub struct LocalQuery {
    /// Exact PostScript names, for `queryLocalFonts({ postscriptNames })`.
    pub postscript_names: Vec<String>,
    /// The family keys an installed face's family may have.
    pub families: Vec<String>,
}

/// A family the presentation's text uses, with the faces no font provides yet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(export_to = "worker/")]
pub struct FontNeed {
    /// As the source names it.
    pub family: String,
    /// The family's match key: ASCII lowercased, whitespace and hyphens removed.
    pub key: String,
    /// By weight, then style.
    pub faces: Vec<FaceRequest>,
    /// PostScript names the source records, to find a face by.
    pub postscript_names: Vec<String>,
    /// The input embeds the font.
    pub embedded: bool,
    /// What Google Fonts could fetch for the open faces; absent for a family outside the open catalog.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub google: Option<GooglePlan>,
    /// What installed faces are matched against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub local: Option<LocalQuery>,
    /// The metric-compatible family that draws the faces no source provides, if the family has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub substitute: Option<String>,
}

impl From<&slidock::FontNeed> for FontNeed {
    fn from(need: &slidock::FontNeed) -> Self {
        FontNeed {
            family: need.family.clone(),
            key: need.key.as_str().to_owned(),
            faces: need.faces.clone(),
            postscript_names: need.postscript_names.clone(),
            embedded: need.embedded,
            google: need.google.as_ref().map(|plan| GooglePlan {
                family: plan.family.clone(),
                licence: plan.licence.spdx().to_owned(),
                faces: plan
                    .faces
                    .iter()
                    .map(|&(weight, style)| FaceRequest { weight, style })
                    .collect(),
                url: plan.url.clone(),
            }),
            local: need.local.as_ref().map(|query| LocalQuery {
                postscript_names: query.postscript_names.clone(),
                families: query
                    .families
                    .iter()
                    .map(|key| key.as_str().to_owned())
                    .collect(),
            }),
            substitute: substitute(&need.key).map(str::to_owned),
        }
    }
}

/// An installed face's metadata, as Local Font Access gives it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(export_to = "worker/")]
pub struct LocalFontMeta {
    pub postscript_name: String,
    pub full_name: String,
    pub family: String,
    /// Localized to the viewer's language: never parsed.
    pub style: String,
}

impl From<&LocalFontMeta> for slidock_fonts::resolve::LocalFontMeta {
    fn from(meta: &LocalFontMeta) -> Self {
        slidock_fonts::resolve::LocalFontMeta {
            postscript_name: meta.postscript_name.clone(),
            full_name: meta.full_name.clone(),
            family: meta.family.clone(),
            style: meta.style.clone(),
        }
    }
}

/// An installed face to read (`blob()`) and add as `local`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(export_to = "worker/")]
pub struct LocalPick {
    /// Into the metadata list `selectLocal` was given.
    pub index: u32,
    pub postscript_name: String,
    /// The family of the needs it may fill.
    pub family: String,
    /// One of the PostScript names the presentation asks for.
    pub exact_postscript: bool,
}

impl From<&slidock_fonts::resolve::LocalPick> for LocalPick {
    fn from(pick: &slidock_fonts::resolve::LocalPick) -> Self {
        LocalPick {
            index: u32::try_from(pick.index).unwrap_or(u32::MAX),
            postscript_name: pick.postscript_name.clone(),
            family: pick.family.clone(),
            exact_postscript: pick.exact_postscript,
        }
    }
}

/// A face of a Google Fonts `css2` response to fetch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(export_to = "worker/")]
pub struct GoogleFace {
    pub family: String,
    pub style: FaceStyle,
    /// `font-weight`: one weight, or `min max` for a variable file.
    pub weight: String,
    /// A `fonts.gstatic.com` URL.
    pub url: String,
    pub format: Option<String>,
    /// The chunk's `unicode-range`, to pass on to `addFont`; `null` for a whole face.
    pub unicode_range: Option<String>,
    /// The script the chunk is for (`latin`, `cyrillic-ext`).
    pub subset: Option<String>,
}

impl From<&slidock_fonts::google::RemoteFace> for GoogleFace {
    fn from(face: &slidock_fonts::google::RemoteFace) -> Self {
        GoogleFace {
            family: face.family.clone(),
            style: face.style,
            weight: weight_value(face.weight),
            url: face.url.clone(),
            format: face.format.clone(),
            unicode_range: face.unicode_range.as_ref().and_then(unicode_range),
            subset: face.subset.clone(),
        }
    }
}

/// A `unicode-range` value, as the deck's rules print one.
fn unicode_range(range: &slidock_fonts::google::UnicodeRange) -> Option<String> {
    let mut set = slidock_fonts::CodepointSet::new();
    for &(first, last) in range.ranges() {
        set.insert_range(first, last);
    }
    set.unicode_range()
}

/// A deck face a shipped face draws: the family as the slides name it, the drawn weight and the style.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "worker/")]
pub struct ServedFace {
    pub family: String,
    pub weight: u32,
    pub style: FaceStyle,
}

/// One face of the fonts panel: a face the deck ships, or a face of a font held that never binds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(export_to = "worker/")]
pub struct FontReport {
    /// The CSS family of its rules (a refused face: as its file names it).
    pub family: String,
    pub postscript_name: String,
    /// The face's own weight and style.
    pub weight: u32,
    pub style: FaceStyle,
    pub source: AddedFontSource,
    /// It ships in the deck (unless its family is left out of the download).
    pub shipped: bool,
    /// The shipped files' size; 0 for a face that does not ship.
    pub bytes: u64,
    /// Cut to the code points the deck draws (else whole).
    pub subset: bool,
    /// It draws a face no source has, as the embedded file declared for that face that failed its identity check.
    pub demoted: bool,
    /// Its `OS/2.fsType` keeps it out of a download by default (restricted or bitmap-only embedding). A shipped face
    /// (a preview's restricted face) draws in the preview, and ships in the download only when `include` names its
    /// family; a refused one never binds, and its substitute draws.
    pub restricted: bool,
    /// It is restricted because its file states no embedding permissions (it has no `OS/2` table): unknown fails
    /// closed.
    pub unstated: bool,
    /// The table its file lacked that browsers require (`OS/2`), which slidock wrote so that they load it; `null`
    /// when nothing was.
    pub repaired: Option<String>,
    /// Why it never binds, for a face that does not ship.
    pub refused: Option<String>,
    /// It came from this computer or the user: the download ships it with a notice, and `leaveOut` (or, for a
    /// restricted face, `include`) may name its family.
    pub notice: bool,
    /// The deck faces it draws.
    pub serves: Vec<ServedFace>,
    /// Its `LICENSES.txt` entry; `null` for a face that does not ship.
    pub licence: Option<String>,
}

/// A face of a bundled substitute family the deck draws with and no file provides yet: add its file as `bundled`,
/// then `finish` again, and it ships.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(export_to = "worker/")]
pub struct SubstituteFace {
    pub family: String,
    /// The face's PostScript name: the file to add.
    pub postscript_name: String,
    pub weight: u32,
    pub style: FaceStyle,
}

impl From<&slidock::SubstituteFace> for SubstituteFace {
    fn from(face: &slidock::SubstituteFace) -> Self {
        SubstituteFace {
            family: face.family.clone(),
            postscript_name: face.postscript_name.clone(),
            weight: face.weight,
            style: face.style,
        }
    }
}

/// A problem with a font the session holds; the compile goes on without the face.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "worker/")]
pub struct FontIssue {
    /// The font diagnostic code (`MID54xx`).
    pub code: String,
    pub message: String,
}

impl From<&slidock::FontIssue> for FontIssue {
    fn from(issue: &slidock::FontIssue) -> Self {
        FontIssue {
            code: issue.code().to_owned(),
            message: issue.to_string(),
        }
    }
}

/// One `@font-face` rule of the shipped faces, for the preview's `FontFace`s: the rules `<style id="sd-fonts">`
/// holds, with the file as bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(export_to = "worker/")]
pub struct FontFile {
    pub family: String,
    /// `font-weight`: one weight, or `min max`.
    pub weight: String,
    pub style: FaceStyle,
    /// `unicode-range`; `null` for none.
    pub unicode_range: Option<String>,
    /// The `format()` hint: `woff2`, else `truetype` or `opentype` for a face that could not be encoded.
    pub format: String,
    #[ts(type = "ArrayBuffer")]
    pub bytes: JsObject,
}

/// A range of bytes of a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "worker/")]
pub struct ByteRange {
    pub offset: u64,
    pub length: u64,
}

/// An HEVC decoder configuration, as WebCodecs' `VideoDecoderConfig` takes it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(export_to = "worker/")]
pub struct HevcDecoderConfig {
    /// The RFC 6381 codec string (`hvc1.3.E.L90.B0`).
    pub codec: String,
    /// The `hvcC` record (`description`): the tiles' chunks are length-prefixed NAL units.
    pub description: Vec<u8>,
    /// `chroma_format_idc`: 0 grey, 1 4:2:0, 2 4:2:2, 3 4:4:4.
    pub chroma_format: u8,
    pub bit_depth: u8,
    /// What the configuration's coded pictures state of their colour (the SPS's VUI).
    pub signal: HevcSignal,
}

/// The colour an HEVC stream states for itself (its SPS's VUI video signal type, ISO/IEC 23091-2 code points):
/// `null` where it states nothing. A decoder configured with a colour space takes that one instead, so the app names
/// these values where the file's `nclx` or ICC profile does not.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(export_to = "worker/")]
pub struct HevcSignal {
    pub full_range: Option<bool>,
    pub primaries: Option<u16>,
    pub transfer: Option<u16>,
    pub matrix: Option<u16>,
}

/// One coded picture of a HEIC image: one key chunk.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "worker/")]
pub struct HeicTile {
    /// Into the plan's `configs`.
    pub config: u32,
    /// Where its top-left corner goes in the coded image.
    pub x: u32,
    pub y: u32,
    /// The chunk's bytes: these ranges of the need's file, concatenated.
    pub extents: Vec<ByteRange>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "worker/")]
pub enum MirrorAxis {
    /// Left and right swap.
    Vertical,
    /// Top and bottom swap.
    Horizontal,
}

/// A transform of the image, applied in the plan's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export_to = "worker/")]
pub enum HeicTransform {
    /// Keep this rectangle of the image as it stands.
    Crop {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
    /// Turn anticlockwise by `turns` quarter turns.
    Rotate {
        turns: u8,
    },
    Mirror {
        axis: MirrorAxis,
    },
}

/// The image's colour as its file states it: `nclx` code points (ISO/IEC 23091-2) and its ICC profile. An iPhone's
/// photo has an ICC profile (Display P3) and no `nclx`; its coded pictures' VUI (each configuration's `signal`) may
/// name the primaries or not.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(export_to = "worker/")]
pub struct HeicColour {
    pub primaries: Option<u16>,
    pub transfer: Option<u16>,
    pub matrix: Option<u16>,
    pub full_range: Option<bool>,
    /// The ICC profile.
    pub icc: Option<Vec<u8>>,
    /// The primaries and transfer (code points as `nclx` has them) the ICC profile's colourants and tone curve
    /// match: Display P3 is primaries 12, transfer 13. `null` for a profile of another space.
    pub icc_primaries: Option<u16>,
    pub icc_transfer: Option<u16>,
}

/// How to decode a HEIC photo: its tiles (a single image is one tile), where each goes, then its transforms.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(export_to = "worker/")]
pub struct HeicPlan {
    /// The decoded file's size, once every transform applies: what `addMedia` must give.
    pub width: u32,
    pub height: u32,
    /// The coded image's size, before the transforms: the tiles cover it and may overhang it.
    pub coded_width: u32,
    pub coded_height: u32,
    pub tile_width: u32,
    pub tile_height: u32,
    pub columns: u32,
    pub rows: u32,
    pub configs: Vec<HevcDecoderConfig>,
    /// Row-major.
    pub tiles: Vec<HeicTile>,
    pub transforms: Vec<HeicTransform>,
    pub colour: HeicColour,
}

impl From<&slidock_io::HeifImage> for HeicPlan {
    fn from(image: &slidock_io::HeifImage) -> Self {
        let (width, height) = image.size();
        let icc = image
            .icc
            .as_deref()
            .map(slidock_io::icc_colour)
            .unwrap_or_default();
        HeicPlan {
            width,
            height,
            coded_width: image.coded_width,
            coded_height: image.coded_height,
            tile_width: image.tile_width,
            tile_height: image.tile_height,
            columns: image.columns,
            rows: image.rows,
            configs: image
                .configs
                .iter()
                .map(|config| HevcDecoderConfig {
                    codec: config.codec.clone(),
                    description: config.description.clone(),
                    chroma_format: config.chroma_format,
                    bit_depth: config.bit_depth_luma,
                    signal: HevcSignal {
                        full_range: config.signal.full_range,
                        primaries: config.signal.primaries,
                        transfer: config.signal.transfer,
                        matrix: config.signal.matrix,
                    },
                })
                .collect(),
            tiles: image
                .tiles
                .iter()
                .map(|tile| HeicTile {
                    config: u32::try_from(tile.config).unwrap_or(u32::MAX),
                    x: tile.x,
                    y: tile.y,
                    extents: tile
                        .extents
                        .iter()
                        .map(|extent| ByteRange {
                            offset: extent.offset,
                            length: extent.len,
                        })
                        .collect(),
                })
                .collect(),
            transforms: image
                .transforms
                .iter()
                .map(|transform| match *transform {
                    slidock_io::Transform::Crop(crop) => HeicTransform::Crop {
                        x: crop.x,
                        y: crop.y,
                        width: crop.width,
                        height: crop.height,
                    },
                    slidock_io::Transform::Rotate(turns) => HeicTransform::Rotate { turns },
                    slidock_io::Transform::Mirror(axis) => HeicTransform::Mirror {
                        axis: match axis {
                            slidock_io::MirrorAxis::Vertical => MirrorAxis::Vertical,
                            slidock_io::MirrorAxis::Horizontal => MirrorAxis::Horizontal,
                        },
                    },
                })
                .collect(),
            colour: HeicColour {
                primaries: image.nclx.map(|nclx| nclx.primaries),
                transfer: image.nclx.map(|nclx| nclx.transfer),
                matrix: image.nclx.map(|nclx| nclx.matrix),
                full_range: image.nclx.map(|nclx| nclx.full_range),
                icc: image.icc.clone(),
                icc_primaries: icc.primaries,
                icc_transfer: icc.transfer,
            },
        }
    }
}

/// A medium the deck shows that the compiler cannot draw and the app may decode: a HEIC photo. Answer it with
/// `addMedia` before `finish`; unanswered, it stays omitted (MID5601).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "worker/")]
pub struct MediaNeed {
    /// The source item's SHA-256, lowercase hex: what `addMedia` names.
    pub id: String,
    /// The source's type (`image/heic`).
    pub mime: String,
    /// The source file: a slice of the input `File`, or the bytes the compiler read. The plan's ranges are its.
    #[ts(type = "Blob")]
    pub blob: JsObject,
    /// How to decode it; `null` when the compiler plans none, and `refusal` says why.
    pub plan: Option<HeicPlan>,
    pub refusal: Option<String>,
}

/// A file under `media/`, as the app serves it (a `blob:` URL of `blob`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "worker/")]
pub struct MediaHandle {
    /// `media/<sha256>.<ext>`, as the slides reference it.
    pub path: String,
    pub mime: String,
    /// A slice of the input `File` for a copied medium, the written bytes for a transcoded one.
    #[ts(type = "Blob")]
    pub blob: JsObject,
}

/// A picture of the finished deck the app may re-encode smaller for a download: scaled to `targetWidth` ×
/// `targetHeight` (the size it shows at, its EXIF orientation applied, or smaller: never larger than twice the largest
/// box it is drawn in on a full-HD screen), as WebP (or JPEG or PNG where the browser has no WebP encoder).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(export_to = "worker/")]
pub struct PlannedImage {
    /// `media/<sha256>.<ext>`: the printed deck's medium (`deck.media`).
    pub path: String,
    pub mime: String,
    /// The original's size.
    pub bytes: u64,
    /// The size it shows at.
    pub width: u32,
    pub height: u32,
    /// Its EXIF orientation (1 when it states none): a browser that draws it applies it.
    pub orientation: u16,
    pub target_width: u32,
    pub target_height: u32,
}

impl From<&slidock::images::ImagePlan> for PlannedImage {
    fn from(plan: &slidock::images::ImagePlan) -> Self {
        PlannedImage {
            path: plan.path.clone(),
            mime: plan.mime.clone(),
            bytes: plan.bytes,
            width: plan.width,
            height: plan.height,
            orientation: plan.orientation,
            target_width: plan.target_width,
            target_height: plan.target_height,
        }
    }
}

/// A video's sound track, as its movie describes it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(export_to = "worker/")]
pub struct PlannedAudio {
    /// The sample format: `mp4a`, `sowt`, `twos`, `lpcm`, …
    pub codec: String,
    /// `esds`' MPEG-4 object type, for `mp4a` (64: MPEG-4 audio).
    pub object_type: Option<u8>,
    /// AAC: a re-encode carries it over as it is.
    pub aac: bool,
    pub channels: u16,
    pub sample_rate: u32,
    /// Bits per sample.
    pub sample_size: u16,
    /// The bytes of its media: what leaving it out saves.
    pub bytes: u64,
}

/// A video's colour as its movie states it (ISO/IEC 23091-2 code points; `null` where nothing states it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(export_to = "worker/")]
pub struct PlannedColour {
    pub primaries: Option<u16>,
    pub transfer: Option<u16>,
    pub matrix: Option<u16>,
    pub full_range: Option<bool>,
    /// Primaries wider than BT.709's (BT.2020, Display P3): a re-encode maps them into BT.709.
    pub wide_gamut: bool,
}

/// A video of the finished deck: what its movie holds and the size the app may re-encode it at (H.264 in an MP4, as
/// it shows, never larger than twice the largest box it is drawn in on a full-HD screen), or why it stays as it is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(export_to = "worker/")]
pub struct PlannedVideo {
    /// `media/<sha256>.<ext>`: the printed deck's medium (`deck.media`).
    pub path: String,
    pub mime: String,
    /// The original's size.
    pub bytes: u64,
    /// The video track's sample format (`avc1`, `hvc1`, …).
    pub codec: String,
    /// The size it shows at: its coded size turned by its track matrix.
    pub width: u32,
    pub height: u32,
    /// The clockwise turn its track matrix applies: 0, 90, 180 or 270.
    pub rotation: u16,
    pub duration_ms: f64,
    /// The video track's samples and their bytes.
    pub frames: u32,
    pub video_bytes: u64,
    pub audio: Option<PlannedAudio>,
    pub colour: PlannedColour,
    pub target_width: u32,
    pub target_height: u32,
    /// Why it stays as it is; `null` when the app may re-encode it.
    pub keep: Option<String>,
    /// It has sound and every use plays it muted: its sound is never heard, so the app's file leaves it out, and a
    /// video the app would keep otherwise is copied without it (unless `uncopyable`).
    pub silent: bool,
    /// Why its video track cannot be copied as it is; `null` when it can.
    pub uncopyable: Option<String>,
}

impl From<&slidock::videos::VideoPlan> for PlannedVideo {
    fn from(plan: &slidock::videos::VideoPlan) -> Self {
        PlannedVideo {
            path: plan.path.clone(),
            mime: plan.mime.clone(),
            bytes: plan.bytes,
            codec: plan.codec.clone(),
            width: plan.width,
            height: plan.height,
            rotation: plan.rotation,
            duration_ms: plan.duration_ms,
            frames: plan.frames,
            video_bytes: plan.video_bytes,
            audio: plan.audio.as_ref().map(|audio| PlannedAudio {
                codec: audio.codec.clone(),
                object_type: audio.object_type,
                aac: audio.aac,
                channels: audio.channels,
                sample_rate: audio.sample_rate,
                sample_size: audio.sample_size,
                bytes: audio.bytes,
            }),
            colour: PlannedColour {
                primaries: plan.colour.primaries,
                transfer: plan.colour.transfer,
                matrix: plan.colour.matrix,
                full_range: plan.colour.full_range,
                wide_gamut: plan.colour.wide_gamut(),
            },
            target_width: plan.target_width,
            target_height: plan.target_height,
            keep: plan.keep.as_ref().map(ToString::to_string),
            silent: plan.silent,
            uncopyable: plan.uncopyable.as_ref().map(ToString::to_string),
        }
    }
}

/// Why a request failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "worker/")]
pub enum ErrorKind {
    /// The input is refused as a whole: not a presentation, encrypted, too large or corrupt.
    Refused,
    /// The presentation failed as a whole: nothing prints.
    Failed,
    /// A request out of order (`finish` before `open`, `bundle` before `finish`) or a file that is no font.
    Request,
    /// The bundle could not be written.
    Bundle,
    /// `cancel` stopped it.
    Cancelled,
    /// The compiler crashed.
    Internal,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "worker/")]
pub struct WorkerError {
    pub kind: ErrorKind,
    /// For a bug report; the diagnostics say what a person can act on.
    pub message: String,
    /// The diagnostics behind a refusal or a failure.
    pub diagnostics: Vec<DeckDiagnostic>,
}

/// Worker → app.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export_to = "worker/")]
pub enum FromWorker {
    /// The worker is loaded and takes requests; `version` is the compiler's.
    Ready { version: String },
    /// Request `id` starts `phase`.
    Progress { id: u32, phase: Phase },
    /// Diagnostics of request `id`, in output order; each batch adds to the earlier ones.
    Diagnostics {
        id: u32,
        diagnostics: Vec<DeckDiagnostic>,
    },
    /// The faces no font provides yet, by family.
    Needs { id: u32, needs: Vec<FontNeed> },
    /// The media the app may decode for the compiler (HEIC photos), after `open`'s `needs`.
    MediaNeeds { id: u32, needs: Vec<MediaNeed> },
    /// A finished deck: its JSON, each slide composed with its layout (in show order), its media, and its fonts:
    /// the shipped faces' rules with their files, the fonts panel's faces, the problems with the fonts held and the
    /// bundled substitute faces it draws with no file yet (the worker always sends these four).
    Deck {
        id: u32,
        deck: Box<Deck>,
        slides: Vec<SlideTemplate>,
        media: Vec<MediaHandle>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        fonts: Option<Vec<FontFile>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        faces: Option<Vec<FontReport>>,
        #[serde(
            default,
            rename = "fontIssues",
            skip_serializing_if = "Option::is_none"
        )]
        #[ts(optional)]
        font_issues: Option<Vec<FontIssue>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        substitutes: Option<Vec<SubstituteFace>>,
        /// The page's default title and description, which the CLI writes too (the download dialog prefills them).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        page: Option<PageDefaults>,
    },
    /// The installed faces to read, then add as `local`.
    LocalPicks { id: u32, picks: Vec<LocalPick> },
    /// The pictures the app may re-encode, in path order.
    ImagePlan { id: u32, images: Vec<PlannedImage> },
    /// The deck's videos, in path order.
    VideoPlan { id: u32, videos: Vec<PlannedVideo> },
    /// The deck's videos that get a copy, in path order.
    AlternatePlan { id: u32, videos: Vec<PlannedVideo> },
    /// The faces of a Google Fonts response to fetch, then add as `google`.
    GoogleFaces { id: u32, faces: Vec<GoogleFace> },
    /// The next bytes of the zip, in order (transferred).
    ZipChunk {
        id: u32,
        #[ts(type = "ArrayBuffer")]
        bytes: JsObject,
    },
    /// The compiler's linear memory, in bytes. It only grows, so it is the most the compiler has held since the
    /// worker started.
    Memory { id: u32, bytes: u64 },
    /// Request `id` is over; a bundle's report, `null` for the other requests.
    Done {
        id: u32,
        report: Option<BundleReport>,
    },
    /// Request `id` failed; `id` is `null` for a failure outside any request (the wasm module did not load).
    Error { id: Option<u32>, error: WorkerError },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_each_message_with_its_type_and_names_fields_in_camel_case() {
        let open = ToWorker::Open {
            id: 1,
            file: JsObject,
            fonts: None,
        };
        assert_eq!(
            serde_json::to_string(&open).unwrap(),
            r#"{"type":"open","id":1,"file":null}"#
        );
        let parity: ToWorker =
            serde_json::from_str(r#"{"type":"open","id":1,"file":null,"fonts":false}"#).unwrap();
        assert!(matches!(
            parity,
            ToWorker::Open {
                fonts: Some(false),
                ..
            }
        ));
        let add = ToWorker::AddFont {
            id: 2,
            bytes: JsObject,
            source: AddedFontSource::Google,
            unicode_range: None,
        };
        assert_eq!(
            serde_json::to_string(&add).unwrap(),
            r#"{"type":"addFont","id":2,"bytes":null,"source":"google"}"#
        );
        let chunk: ToWorker = serde_json::from_str(
            r#"{"type":"addFont","id":2,"bytes":null,"source":"google","unicodeRange":"U+0-FF"}"#,
        )
        .unwrap();
        assert!(matches!(
            chunk,
            ToWorker::AddFont { unicode_range: Some(range), .. } if range == "U+0-FF"
        ));
        let bundle: ToWorker = serde_json::from_str(
            r#"{"type":"bundle","id":3,"settings":{"singleFile":true,"singleFileThreshold":null,"leaveOut":["Calibri"],"include":["Optima"]}}"#,
        )
        .unwrap();
        assert_eq!(
            bundle,
            ToWorker::Bundle {
                id: 3,
                settings: BundleSettings {
                    single_file: true,
                    single_file_threshold: None,
                    leave_out: vec!["Calibri".to_owned()],
                    include: vec!["Optima".to_owned()],
                    compress_images: None,
                    compress_videos: None,
                    title: None,
                    description: None,
                    site_url: None,
                    presenter: None,
                    notes: None,
                    text_view: None,
                    alternate_videos: None,
                },
            }
        );
        let compressed: ToWorker = serde_json::from_str(
            r#"{"type":"bundle","id":4,"settings":{"singleFile":false,"singleFileThreshold":null,"leaveOut":[],"include":[],"compressImages":true}}"#,
        )
        .unwrap();
        assert!(matches!(
            compressed,
            ToWorker::Bundle {
                settings: BundleSettings {
                    compress_images: Some(true),
                    ..
                },
                ..
            }
        ));
        let videos: ToWorker = serde_json::from_str(
            r#"{"type":"bundle","id":7,"settings":{"singleFile":false,"singleFileThreshold":null,"leaveOut":[],"include":[],"compressVideos":true}}"#,
        )
        .unwrap();
        assert!(matches!(
            videos,
            ToWorker::Bundle {
                settings: BundleSettings {
                    compress_images: None,
                    compress_videos: Some(true),
                    ..
                },
                ..
            }
        ));
        let video_plan: ToWorker = serde_json::from_str(r#"{"type":"videoPlan","id":8}"#).unwrap();
        assert_eq!(video_plan, ToWorker::VideoPlan { id: 8 });
        let video: ToWorker =
            serde_json::from_str(r#"{"type":"addVideo","id":9,"path":"media/a.mov","file":null}"#)
                .unwrap();
        assert!(
            matches!(video, ToWorker::AddVideo { id: 9, ref path, .. } if path == "media/a.mov")
        );
        let alternate_plan: ToWorker =
            serde_json::from_str(r#"{"type":"alternatePlan","id":10}"#).unwrap();
        assert_eq!(alternate_plan, ToWorker::AlternatePlan { id: 10 });
        let alternate: ToWorker = serde_json::from_str(
            r#"{"type":"addAlternate","id":11,"path":"media/a.mov","file":null}"#,
        )
        .unwrap();
        assert!(
            matches!(alternate, ToWorker::AddAlternate { id: 11, ref path, .. } if path == "media/a.mov")
        );
        let plan: ToWorker = serde_json::from_str(r#"{"type":"imagePlan","id":5}"#).unwrap();
        assert_eq!(plan, ToWorker::ImagePlan { id: 5 });
        let image: ToWorker =
            serde_json::from_str(r#"{"type":"addImage","id":6,"path":"media/a.png","bytes":null}"#)
                .unwrap();
        assert!(
            matches!(image, ToWorker::AddImage { id: 6, ref path, .. } if path == "media/a.png")
        );
        let progress = FromWorker::Progress {
            id: 2,
            phase: Phase::Print,
        };
        assert_eq!(
            serde_json::to_string(&progress).unwrap(),
            r#"{"type":"progress","id":2,"phase":"print"}"#
        );
        let done = FromWorker::Done {
            id: 3,
            report: None,
        };
        assert_eq!(
            serde_json::to_string(&done).unwrap(),
            r#"{"type":"done","id":3,"report":null}"#
        );
        let chunk = FromWorker::ZipChunk {
            id: 3,
            bytes: JsObject,
        };
        assert_eq!(
            serde_json::to_string(&chunk).unwrap(),
            r#"{"type":"zipChunk","id":3,"bytes":null}"#
        );
    }

    #[test]
    fn maps_bundle_settings_onto_the_bundle_options() {
        let settings = BundleSettings {
            single_file: true,
            single_file_threshold: None,
            leave_out: vec!["Calibri".to_owned(), "Aptos".to_owned()],
            include: vec!["Optima".to_owned()],
            compress_images: Some(true),
            compress_videos: Some(true),
            title: None,
            description: None,
            site_url: None,
            presenter: None,
            notes: None,
            text_view: None,
            alternate_videos: Some(true),
        };
        let options = slidock::BundleOptions::from(&settings);
        // Absent page fields keep the bundle's defaults.
        assert!(options.text_view);
        assert!(options.presenter);
        assert_eq!(options.notes, slidock::NotesMode::Presenter);
        assert_eq!(
            (&options.title, &options.description, &options.site_url),
            (&None, &None, &None)
        );
        assert!(options.single_file);
        assert_eq!(
            options.single_file_threshold,
            slidock::DEFAULT_SINGLE_FILE_THRESHOLD
        );
        assert_eq!(
            options.leave_out.into_iter().collect::<Vec<_>>(),
            ["Aptos", "Calibri"]
        );
        assert_eq!(options.include.into_iter().collect::<Vec<_>>(), ["Optima"]);
        let options = slidock::BundleOptions::from(&BundleSettings {
            single_file_threshold: Some(1024),
            ..settings
        });
        assert_eq!(options.single_file_threshold, 1024);
        assert_eq!(options.packaging, slidock::Packaging::Zip);
        let page: BundleSettings = serde_json::from_str(
            r#"{"singleFile":false,"singleFileThreshold":null,"leaveOut":[],"include":[],"title":"T","description":"","siteUrl":"https://example.com/","presenter":false,"notes":"page","textView":false}"#,
        )
        .unwrap();
        let options = slidock::BundleOptions::from(&page);
        assert_eq!(options.title.as_deref(), Some("T"));
        assert_eq!(options.description.as_deref(), Some(""));
        assert_eq!(options.site_url.as_deref(), Some("https://example.com/"));
        assert_eq!(options.notes, slidock::NotesMode::Page);
        assert!(!options.text_view);
        assert!(!options.presenter);
        // The host fills the replacements from the pictures it took.
        assert!(options.replaced.is_empty());
    }

    #[test]
    fn maps_the_added_sources_onto_the_sessions() {
        assert_eq!(
            slidock::FontSource::from(AddedFontSource::Local),
            slidock::FontSource::Local
        );
        assert_eq!(
            slidock::FontSource::from(AddedFontSource::Google),
            slidock::FontSource::Google
        );
        assert_eq!(
            slidock::FontSource::from(AddedFontSource::User),
            slidock::FontSource::User
        );
        for source in [
            slidock::FontSource::User,
            slidock::FontSource::Google,
            slidock::FontSource::Local,
            slidock::FontSource::Embedded,
        ] {
            assert_eq!(
                slidock::FontSource::from(AddedFontSource::from(source)),
                source
            );
        }
    }
}
