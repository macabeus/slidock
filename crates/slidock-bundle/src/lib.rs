//! Packaging: composes the single-page `index.html` (slide templates, deck JSON, `@font-face` rules and the
//! embedded player), the CSP hashes, `media/` and `LICENSES.txt`, and writes the deck zip deterministically.
//!
//! The output streams through a [`BundleSink`] in chunks of at most [`CHUNK_BYTES`], so neither the zip nor a media
//! file is ever whole in memory (the single-file option holds the media it inlines, up to its threshold): in the
//! browser the worker posts each chunk to the page. The media bytes come from the host through
//! [`DeckMedia`], which reads them by range from the input.
//!
//! The player is `@slidock/player`'s IIFE build, embedded at build time ([`player`]), and so is the presenter view's,
//! which a page ships only with [`BundleOptions::presenter`]: `pnpm player:build` refreshes the crate's committed
//! copies, `player/{player,presenter}.iife.js`, and the next cargo build embeds them (`build.rs`). A deck records their
//! version and SHA-256 as its `player`.

mod check;
mod fonts;
mod licenses;
mod metadata;
mod output;
pub mod page;
pub mod player;
pub mod text_view;
mod zip;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::Write;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use slidock_fonts::{FontFaceRule, FontOrigin, LicenceEntry};
use slidock_format::{ComposedSlide, MediaUrls, compose_with_sources};
use slidock_html::Documents;
use slidock_html::deck::{Deck, DeckMedia as DeckFile, PlayerStamp, deck_id};
use slidock_ir::fonts::FaceStyle;
use thiserror::Error;
use ts_rs::TS;

pub use check::lint_templates;
pub use fonts::deck_source;
pub use metadata::{
    DESCRIPTION_CHARS, PageDefaults, SiteUrlError, page_defaults, site_url, size_notice,
};
pub use output::CHUNK_BYTES;
pub use zip::DEFLATE_LEVEL;

use output::Chunks;
use zip::{Crc32, ZipWriter};

/// The default single-file threshold: above it, media stay files under `media/`.
pub const DEFAULT_SINGLE_FILE_THRESHOLD: u64 = 50 * 1024 * 1024;

/// Where `index.html` sits in the output.
pub const INDEX_PATH: &str = "index.html";

/// Where the shipped faces' licences sit in the output.
pub const LICENSES_PATH: &str = "LICENSES.txt";

/// How the output is packed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Packaging {
    /// One zip archive, written as one byte stream.
    #[default]
    Zip,
    /// The files themselves, one after another (`index.html`, `media/…`, `LICENSES.txt`), for a host that writes a
    /// folder.
    Files,
}

/// How a deck is bundled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundleOptions {
    pub packaging: Packaging,
    /// Inline the media into `index.html` as `data:` URIs when they add up to at most `single_file_threshold`
    /// bytes, each counted once per slide template that shows it; above it they stay files under `media/`.
    pub single_file: bool,
    pub single_file_threshold: u64,
    /// Families to leave out of the download (the licence policy's per-family toggle, offered for viewer-installed
    /// and dropped faces), as the deck JSON's `fonts` or the shipped rules name them, matched by family key: their
    /// faces from this computer or the user ship no file, the deck JSON lists them as `system`, and the report lists
    /// them in `fonts_left_out`. A family of faces from any other source ships whatever this holds.
    pub leave_out: BTreeSet<String>,
    /// Families to include in the download whatever their faces' restricted `fsType` says (the same toggle, the
    /// user's call), named and matched as `leave_out`: their faces from this computer or the user ship, with the
    /// notice. A family in both is left out; a restricted face from any other source never ships.
    pub include: BTreeSet<String>,
    /// Files the host made smaller for this download, by the deck's media path: each is written at its own path (and
    /// inlined with its own type) in place of the original, and the slides and the deck JSON name it instead.
    pub replaced: BTreeMap<String, ReplacedMedia>,
    /// Write the text view of the shown slides below the stage (`<article id="sd-text">`, [`text_view`]).
    pub text_view: bool,
    /// Ship the presenter view: its script, the P key, its help row and the `#presenter` window. Without it the page
    /// has none, and `#presenter` shows the deck.
    pub presenter: bool,
    /// Which speaker notes ship, and where.
    pub notes: NotesMode,
    /// The page's title, trimmed; [`page_defaults`]'s when `None` or blank.
    pub title: Option<String>,
    /// The page's description, trimmed; [`page_defaults`]'s when `None`, none when blank.
    pub description: Option<String>,
    /// The deck's public URL, for the canonical link and `og:url`: an absolute `http(s)` URL with no user info and
    /// no fragment ([`site_url`]), or the bundle fails.
    pub site_url: Option<String>,
    /// Files the host made for this download that play where the original does not, by the deck's media path: each
    /// is written after the original, listed in the deck JSON's `media`, and offered after it by every `<video>`
    /// that plays it (a `<source>` of each, the original's first, in place of its `src`).
    pub alternates: BTreeMap<String, AlternateSource>,
}

/// A file offered after one of the deck's videos (a copy other browsers play): its path, `media/<sha256>.<ext>`, its
/// type, its `codecs` parameter and its size, and the `codecs` parameter of the original's `<source>`. The host's
/// [`DeckMedia`] serves it at that path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlternateSource {
    pub path: String,
    pub mime: String,
    pub codecs: String,
    pub bytes: u64,
    /// The original's sample format, bare (`hvc1`): WebKit takes a QuickTime type of it, Firefox and Chromium do not
    /// (they may decode HEVC, but without its alpha layer), so only WebKit plays the original.
    pub original_codecs: String,
}

impl AlternateSource {
    /// Its `<source type>`: the type and the codec.
    pub fn source_type(&self) -> String {
        format!("{}; codecs=\"{}\"", self.mime, self.codecs)
    }

    /// The original's `<source type>`: its type `mime` and its bare sample format.
    pub fn original_type(&self, mime: &str) -> String {
        format!("{mime}; codecs=\"{}\"", self.original_codecs)
    }
}

/// Which speaker notes a bundle carries, and where. Notes can be private, so they ship only where the page shows
/// them: in the deck JSON for the presenter view only when the page ships one ([`BundleOptions::presenter`]), and in
/// the text view only by choice ([`NotesMode::Page`], with [`BundleOptions::text_view`]). Anywhere else, the deck
/// JSON's notes are emptied.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum NotesMode {
    /// In the presenter view.
    #[default]
    Presenter,
    /// In the presenter view and in the text view, as an open disclosure after each section's blocks.
    Page,
    /// Nowhere: the download carries none.
    None,
}

impl BundleOptions {
    /// Whether the deck JSON keeps the notes: for the presenter view, when the page ships one.
    pub fn notes_in_deck(&self) -> bool {
        self.presenter && self.notes != NotesMode::None
    }

    /// Whether the text view prints the notes.
    pub fn notes_on_page(&self) -> bool {
        self.text_view && self.notes == NotesMode::Page
    }
}

/// A file written in place of one of the deck's media (a picture re-encoded smaller): its path, `media/<sha256>.<ext>`
/// of its own type, its type and its size. The host's [`DeckMedia`] serves it at that path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplacedMedia {
    pub path: String,
    pub mime: String,
    pub bytes: u64,
}

impl Default for BundleOptions {
    /// A zip with the media as files beside `index.html`, every shipped face included.
    fn default() -> Self {
        BundleOptions {
            packaging: Packaging::Zip,
            single_file: false,
            single_file_threshold: DEFAULT_SINGLE_FILE_THRESHOLD,
            leave_out: BTreeSet::new(),
            include: BTreeSet::new(),
            replaced: BTreeMap::new(),
            text_view: true,
            presenter: true,
            notes: NotesMode::Presenter,
            title: None,
            description: None,
            site_url: None,
            alternates: BTreeMap::new(),
        }
    }
}

/// The stamp of the player this crate embeds: its version and the SHA-256 of `player.iife.js`, and with `presenter`
/// of `presenter.iife.js`. It is the only source of a bundled deck's `player`.
pub fn embedded_player(presenter: bool) -> Result<PlayerStamp, BundleError> {
    Ok(player::stamp(presenter))
}

/// Where the bundle's bytes go.
pub trait BundleSink {
    /// Takes the next bytes of the output. Returning an error (such as [`BundleError::Cancelled`] when the host
    /// observes a cancel) stops the bundle with it. With [`Packaging::Zip`] they are the archive's, in order, and `path` names
    /// the entry they belong to (`""` for the archive's closing directory), for progress. With [`Packaging::Files`]
    /// they are the next bytes of the file at `path`; files come one after another, each in order.
    fn write(&mut self, path: &str, bytes: &[u8]) -> Result<(), BundleError>;
}

/// The bytes of the deck's media files, which the host reads from its input.
pub trait DeckMedia {
    /// Writes the file at `path` (a path of the deck's `media`) to `out`, in order, in chunks of the host's choosing.
    fn write_media(&self, path: &str, out: &mut dyn std::io::Write) -> Result<(), BundleError>;
}

/// A face of the deck a shipped file draws, as the deck JSON's `fonts` list it: the family the slides name, the drawn
/// weight and the style.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServedFace {
    pub family: String,
    pub weight: u32,
    pub style: FaceStyle,
}

/// A face shipped in `<style id="sd-fonts">`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShippedFace {
    /// Its `@font-face` rules, with the file's own descriptors: one per weight range it claims.
    pub rules: Vec<FontFaceRule>,
    /// Its `LICENSES.txt` entry; the deck JSON's `licence` of each face it draws is its text.
    pub licence: LicenceEntry,
    /// The deck faces it draws.
    pub serves: Vec<ServedFace>,
}

impl ShippedFace {
    /// It came from the computer that made the deck or from the user: the download lists it for a notice, and the
    /// host may leave its family out.
    pub fn notice(&self) -> bool {
        matches!(self.licence.origin, FontOrigin::Local | FontOrigin::User)
    }
}

/// A compiled deck, ready to bundle.
pub struct CompiledDeck<'a> {
    /// Its `player` is not read: the bundle writes [`embedded_player`].
    pub deck: Deck,
    /// The printed document set: the manifest, the slide and layout documents and the media they reference.
    pub documents: Documents,
    /// The bytes of the files under `media/`.
    pub media: &'a dyn DeckMedia,
    /// The faces the deck ships.
    pub fonts: Vec<ShippedFace>,
}

impl fmt::Debug for CompiledDeck<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CompiledDeck")
            .field("deck", &self.deck.id)
            .field("documents", &self.documents.text.len())
            .field("fonts", &self.fonts.len())
            .finish_non_exhaustive()
    }
}

/// A slide composed with its layout: what `<template data-sd-slide="<id>">` holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "bundle/")]
pub struct SlideTemplate {
    pub id: String,
    /// The composed `<main data-slide>`, referencing media as `media/<sha256>.<ext>`.
    pub html: String,
}

/// One file of the output.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "bundle/")]
pub struct BundledFile {
    pub path: String,
    /// Its size before compression.
    pub bytes: u64,
}

/// What a bundle wrote.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(export_to = "bundle/")]
pub struct BundleReport {
    /// In output order.
    pub files: Vec<BundledFile>,
    /// Every byte handed to the sink.
    pub bytes: u64,
    /// Whether the media went into `index.html`.
    pub single_file: bool,
    /// The families left out of the download, by the host or for their licence, each drawn by the viewer's installed
    /// font or its substitute.
    pub fonts_left_out: Vec<String>,
    /// The families of the shipped faces that came from the computer that made the deck or from the user, sorted: the
    /// download's notice asks whether they may be shared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub fonts_with_notice: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum BundleError {
    /// The sink refused the bytes (a closed stream, a full disk, a cancelled download).
    #[error("{0}")]
    Sink(String),
    /// The host cannot give a medium's bytes.
    #[error("media {path}: {message}")]
    Media { path: String, message: String },
    /// The document set does not compose (a document the lint refuses).
    #[error("{0}")]
    Compose(String),
    /// The deck JSON cannot be written.
    #[error("the deck JSON: {0}")]
    Deck(String),
    /// An entry the zip format cannot hold.
    #[error("{0}")]
    Zip(String),
    /// The host stopped the bundle (its sink observed a cancel).
    #[error("cancelled")]
    Cancelled,
    /// The public URL is not one a page may name as its own.
    #[error("{0}")]
    SiteUrl(#[from] SiteUrlError),
}

/// Each slide composed with its layout, in show order, as the templates of `index.html` hold them.
pub fn slide_templates(deck: &CompiledDeck<'_>) -> Result<Vec<SlideTemplate>, BundleError> {
    compose_templates(deck, &MediaUrls::Base(MEDIA_DIR), &BundleOptions::default())
}

const MEDIA_DIR: &str = slidock_format::paths::MEDIA_DIR;

/// The slides composed with `media`'s URLs, each video with an alternate of `options` offering both files.
fn compose_templates(
    deck: &CompiledDeck<'_>,
    media: &MediaUrls<'_>,
    options: &BundleOptions,
) -> Result<Vec<SlideTemplate>, BundleError> {
    Ok(templates_of(deck, &composed_slides(deck, media, options)?))
}

/// The templates of composed slides.
fn templates_of(deck: &CompiledDeck<'_>, composed: &[ComposedSlide]) -> Vec<SlideTemplate> {
    composed
        .iter()
        .zip(&deck.deck.slides)
        .map(|(composed, slide)| SlideTemplate {
            id: slide.id.clone(),
            html: composed.main_html(),
        })
        .collect()
}

/// Each slide composed with its layout, in show order, each video with an alternate of `options` offering both files.
fn composed_slides(
    deck: &CompiledDeck<'_>,
    media: &MediaUrls<'_>,
    options: &BundleOptions,
) -> Result<Vec<ComposedSlide>, BundleError> {
    let files = deck.documents.lintable();
    let sources = |path: &str| -> Vec<(String, String)> {
        let Some(alternate) = options.alternates.get(path) else {
            return Vec::new();
        };
        let mime = match options.replaced.get(path) {
            Some(replaced) => replaced.mime.clone(),
            None => deck
                .documents
                .media
                .get(path)
                .map(|plan| plan.mime.as_str().to_owned())
                .unwrap_or_default(),
        };
        vec![
            (path.to_owned(), alternate.original_type(&mime)),
            (alternate.path.clone(), alternate.source_type()),
        ]
    };
    deck.deck
        .slides
        .iter()
        .map(|slide| {
            compose_with_sources(&files, &slide.id, media, &sources)
                .map_err(|error| BundleError::Compose(error.to_string()))
        })
        .collect()
}

/// A medium's bytes, kept while they stay within a budget: each byte costs once per copy the page will hold.
struct Within {
    bytes: Vec<u8>,
    budget: u64,
    copies: u64,
    over: bool,
}

impl Write for Within {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let cost = (buf.len() as u64).saturating_mul(self.copies);
        if cost > self.budget {
            self.over = true;
            return Err(std::io::Error::other("over the single-file threshold"));
        }
        self.budget -= cost;
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The path a medium is written at: its replacement's, or its own.
fn written_path<'p>(options: &'p BundleOptions, path: &'p str) -> &'p str {
    options
        .replaced
        .get(path)
        .map_or(path, |replaced| replaced.path.as_str())
}

/// Each medium the `templates` reference as a `data:` URI, by the path it is written at, when the page would hold at
/// most `threshold` bytes of media: a medium counts once per reference, since every template that shows it holds its
/// own copy (a layout's picture repeats on each slide of the layout). `None` above it.
fn inline_media(
    deck: &CompiledDeck<'_>,
    options: &BundleOptions,
    templates: &[SlideTemplate],
    threshold: u64,
) -> Result<Option<BTreeMap<String, String>>, BundleError> {
    let copies = |path: &str| -> u64 {
        templates
            .iter()
            .map(|template| template.html.matches(path).count() as u64)
            .sum()
    };
    // Each referenced medium: the path and type it is written with, its size when known, its copies.
    let referenced: Vec<(&str, &str, Option<u64>, u64)> = deck
        .documents
        .media
        .iter()
        .map(|(path, plan)| match options.replaced.get(path) {
            Some(replaced) => (
                replaced.path.as_str(),
                replaced.mime.as_str(),
                Some(replaced.bytes),
            ),
            None => (
                path.as_str(),
                plan.mime.as_str(),
                deck.deck
                    .media
                    .iter()
                    .find(|media| &media.path == path)
                    .and_then(|media| media.bytes),
            ),
        })
        .chain(options.alternates.values().map(|alternate| {
            (
                alternate.path.as_str(),
                alternate.mime.as_str(),
                Some(alternate.bytes),
            )
        }))
        .map(|(path, mime, bytes)| (path, mime, bytes, copies(path)))
        .filter(|(_, _, _, copies)| *copies > 0)
        .collect();
    let known: u64 = referenced
        .iter()
        .filter_map(|(_, _, bytes, copies)| Some(bytes.as_ref()?.saturating_mul(*copies)))
        .fold(0, u64::saturating_add);
    if known > threshold {
        return Ok(None);
    }
    let mut budget = threshold;
    let mut uris = BTreeMap::new();
    for (path, mime, _, copies) in referenced {
        let mut within = Within {
            bytes: Vec::new(),
            budget,
            copies,
            over: false,
        };
        if let Err(error) = deck.media.write_media(path, &mut within) {
            if within.over {
                return Ok(None);
            }
            return Err(error);
        }
        budget = within.budget;
        let uri = format!("data:{mime};base64,{}", STANDARD.encode(&within.bytes));
        uris.insert(path.to_owned(), uri);
    }
    Ok(Some(uris))
}

/// A medium's bytes on their way to the output, counted and checksummed. A sink refusal is kept, so it, and not
/// the host's report of the failed write, ends the bundle.
struct MediaOut<'c, 's> {
    chunks: &'c mut Chunks<'s>,
    crc: Crc32,
    size: u64,
    refused: Option<BundleError>,
}

impl Write for MediaOut<'_, '_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if let Err(error) = self.chunks.write(buf) {
            self.refused = Some(error);
            return Err(std::io::Error::other("the bundle's sink refused the bytes"));
        }
        self.crc.update(buf);
        self.size += buf.len() as u64;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The files of the output, one after another or as zip entries.
struct Writer<'s> {
    chunks: Chunks<'s>,
    zip: Option<ZipWriter>,
    files: Vec<BundledFile>,
}

impl<'s> Writer<'s> {
    fn new(packaging: Packaging, sink: &'s mut dyn BundleSink) -> Writer<'s> {
        Writer {
            chunks: Chunks::new(sink),
            zip: match packaging {
                Packaging::Zip => Some(ZipWriter::default()),
                Packaging::Files => None,
            },
            files: Vec::new(),
        }
    }

    /// A text file, deflated in a zip.
    fn text(&mut self, path: &str, text: &str) -> Result<(), BundleError> {
        self.chunks.begin(path)?;
        match &mut self.zip {
            Some(zip) => zip.deflated(path, text.as_bytes(), &mut self.chunks)?,
            None => self.chunks.write(text.as_bytes())?,
        }
        self.files.push(BundledFile {
            path: path.to_owned(),
            bytes: text.len() as u64,
        });
        Ok(())
    }

    /// A medium, streamed from the host and stored in a zip.
    fn media(&mut self, path: &str, media: &dyn DeckMedia) -> Result<(), BundleError> {
        self.chunks.begin(path)?;
        let entry = match &mut self.zip {
            Some(zip) => Some(zip.open_stored(path, &mut self.chunks)?),
            None => None,
        };
        let mut out = MediaOut {
            chunks: &mut self.chunks,
            crc: Crc32::default(),
            size: 0,
            refused: None,
        };
        let written = media.write_media(path, &mut out);
        if let Some(refused) = out.refused.take() {
            return Err(refused);
        }
        written?;
        let (crc, size) = (out.crc.finish(), out.size);
        if let (Some(zip), Some(entry)) = (&mut self.zip, entry) {
            zip.close_stored(entry, crc, &mut self.chunks)?;
        }
        self.files.push(BundledFile {
            path: path.to_owned(),
            bytes: size,
        });
        Ok(())
    }

    /// Ends the output (the zip's directory); returns its files and every byte handed to the sink.
    fn finish(mut self) -> Result<(Vec<BundledFile>, u64), BundleError> {
        if let Some(zip) = self.zip.take() {
            self.chunks.begin("")?;
            zip.finish(&mut self.chunks)?;
        }
        let bytes = self.chunks.finish()?;
        Ok((self.files, bytes))
    }
}

/// The deck JSON as `index.html` holds it, and the faces that ship.
fn page_deck<'a>(
    deck: &'a CompiledDeck<'_>,
    options: &BundleOptions,
) -> Result<(Deck, fonts::FontPlan<'a>), BundleError> {
    let mut fonts = fonts::plan(
        &deck.deck.fonts,
        &deck.fonts,
        &options.leave_out,
        &options.include,
    );
    let mut data = deck.deck.clone();
    data.fonts = std::mem::take(&mut fonts.faces);
    let mut listed = Vec::with_capacity(data.media.len() + options.alternates.len());
    for mut media in std::mem::take(&mut data.media) {
        let alternate = options
            .alternates
            .get(&media.path)
            .map(|alternate| DeckFile {
                path: alternate.path.clone(),
                mime: alternate.mime.clone(),
                sha256: media.sha256.clone(),
                bytes: Some(alternate.bytes),
            });
        if let Some(replaced) = options.replaced.get(&media.path) {
            media.path.clone_from(&replaced.path);
            media.mime.clone_from(&replaced.mime);
            media.bytes = Some(replaced.bytes);
        }
        listed.push(media);
        listed.extend(alternate);
    }
    data.title =
        page_text(options.title.as_deref()).unwrap_or_else(|| page_defaults(&deck.deck).title);
    if !options.notes_in_deck() {
        for slide in &mut data.slides {
            slide.notes.clear();
        }
    }
    data.media = listed;
    data.id =
        deck_id(&data, &deck.documents).map_err(|error| BundleError::Deck(error.to_string()))?;
    data.player = Some(embedded_player(options.presenter)?);
    Ok((data, fonts))
}

/// A title or description the user gave, trimmed; `None` when it is blank, which leaves the title to
/// [`page_defaults`] and the page without a description.
fn page_text(text: Option<&str>) -> Option<String> {
    let text = text?.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// The deck JSON a [`bundle`] of `deck` with `options` writes into `index.html`: [`embedded_player`] as its
/// `player`, whatever `deck.deck.player` holds; the faces that ship listed as they ship and the rest as `system`;
/// the page's title as its `title`; notes only for the presenter view ([`BundleOptions::notes_in_deck`]); its `id`
/// the hash of that content.
pub fn bundled_deck(deck: &CompiledDeck<'_>, options: &BundleOptions) -> Result<Deck, BundleError> {
    page_deck(deck, options).map(|(data, _)| data)
}

/// Writes `deck` to `sink`: `index.html` (with the deck JSON of [`bundled_deck`]), then `media/` (unless inlined),
/// then `LICENSES.txt`. The same deck and options write the same bytes. It runs to its end in one call; a host
/// stops it only through its sink.
pub fn bundle(
    deck: &CompiledDeck<'_>,
    options: &BundleOptions,
    sink: &mut dyn BundleSink,
) -> Result<BundleReport, BundleError> {
    let site_url = options.site_url.as_deref().map(site_url).transpose()?;
    let (data, fonts) = page_deck(deck, options)?;
    let description = match &options.description {
        Some(description) => page_text(Some(description)),
        None => page_defaults(&deck.deck).description,
    };

    let renamed = |path: &str| written_path(options, path).to_owned();
    let composed = if options.replaced.is_empty() {
        composed_slides(deck, &MediaUrls::Base(MEDIA_DIR), options)?
    } else {
        composed_slides(deck, &MediaUrls::Url(&renamed), options)?
    };
    // The text view prints the notes of the deck as compiled: the deck JSON may hold none (no presenter view).
    let view = options.text_view.then(|| {
        let notes = options
            .notes_on_page()
            .then_some(deck.deck.slides.as_slice());
        text_view::text_view(&data, &composed, &data.title, notes)
    });
    let mut templates = templates_of(deck, &composed);
    let inline = if options.single_file {
        inline_media(deck, options, &templates, options.single_file_threshold)?
    } else {
        None
    };
    if let Some(uris) = &inline {
        let url = |path: &str| {
            let written = written_path(options, path);
            uris.get(written)
                .cloned()
                .unwrap_or_else(|| written.to_owned())
        };
        templates = compose_templates(deck, &MediaUrls::Url(&url), options)?;
    }
    let index = page::index_html(&page::Page {
        deck: &data,
        font_rules: &fonts.rules,
        templates: &templates,
        description: description.as_deref(),
        site_url: site_url.as_deref(),
        text_view: view.as_deref(),
        presenter: options.presenter,
    })?;

    let mut writer = Writer::new(options.packaging, sink);
    writer.text(INDEX_PATH, &index)?;
    if inline.is_none() {
        for path in deck.documents.media.keys() {
            writer.media(written_path(options, path), deck.media)?;
            if let Some(alternate) = options.alternates.get(path) {
                writer.media(&alternate.path, deck.media)?;
            }
        }
    }
    writer.text(
        LICENSES_PATH,
        &licenses::licenses_text(&fonts.shipped, options.presenter),
    )?;
    let (files, bytes) = writer.finish()?;
    let notice: BTreeSet<String> = fonts
        .shipped
        .iter()
        .filter(|face| face.notice())
        .map(|face| face.licence.family.clone())
        .collect();
    Ok(BundleReport {
        files,
        bytes,
        single_file: inline.is_some(),
        fonts_left_out: fonts.left_out,
        fonts_with_notice: (!notice.is_empty()).then(|| notice.into_iter().collect()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_default_to_a_zip_with_media_files_up_to_fifty_mebibytes_inline() {
        let options = BundleOptions::default();
        assert_eq!(options.packaging, Packaging::Zip);
        assert!(!options.single_file);
        assert_eq!(options.single_file_threshold, 52_428_800);
        assert!(options.leave_out.is_empty());
        assert!(options.replaced.is_empty());
        assert!(options.alternates.is_empty());
    }
}
