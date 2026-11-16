//! What [`Compiler`](crate::Compiler)'s stages return, as the JavaScript values they cross as: each is serialized
//! to JSON and parsed into a plain object, so the deck is exactly the JSON the bundler embeds. Their TypeScript
//! declarations sit in the module's `.d.ts` ([`DECLARATIONS`]), and the stages are typed with them; the
//! `typescript` test checks the declarations against these types.

use std::collections::BTreeMap;

use serde::Serialize;
use slidock::deck::{Deck, DeckDiagnostic};
use slidock::{PageDefaults, SlideTemplate};
use slidock_ir::fonts::FaceStyle;
use ts_rs::TS;

use crate::protocol::{FontIssue, FontNeed, FontReport, HeicPlan, SubstituteFace};

/// `open`'s result.
#[derive(Clone, Debug, Serialize, TS)]
#[ts(rename = "Opened")]
pub struct OpenedJson {
    pub diagnostics: Vec<DeckDiagnostic>,
    /// The faces no font provides yet.
    pub needs: Vec<FontNeed>,
    /// The media the app may decode (HEIC photos).
    pub media: Vec<MediaNeedJson>,
}

/// A media need as the compiler reports it: the protocol's `MediaNeed` with its source's place in the input in place
/// of its `Blob`.
#[derive(Clone, Debug, Serialize, TS)]
#[ts(rename = "OpenedMediaNeed")]
pub struct MediaNeedJson {
    /// The source item's SHA-256, lowercase hex.
    pub id: String,
    pub mime: String,
    /// Where the source file is in the input; `null` for bytes `mediaNeedBytes` reads.
    pub range: Option<RangeJson>,
    pub plan: Option<HeicPlan>,
    /// Why the compiler plans no decode.
    pub refusal: Option<String>,
}

/// A byte range of the input.
#[derive(Clone, Copy, Debug, Serialize, TS)]
#[ts(rename = "MediaRange")]
pub struct RangeJson {
    pub offset: u64,
    pub length: u64,
}

/// A file under `media/` of a printed deck.
#[derive(Clone, Debug, Serialize, TS)]
#[ts(rename = "PrintedMedia")]
pub struct MediaJson {
    /// `media/<sha256>.<ext>`, as the slides reference it.
    pub path: String,
    pub mime: String,
    /// Where a copied medium's bytes are in the input; `null` for bytes `mediaBytes` writes.
    pub range: Option<RangeJson>,
}

/// One `@font-face` rule of the shipped faces, as `<style id="sd-fonts">` holds it.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "PrintedFont")]
pub struct FontJson {
    pub family: String,
    /// `font-weight`: one weight, or `min max`.
    pub weight: String,
    pub style: FaceStyle,
    /// `unicode-range`; `null` for none.
    pub unicode_range: Option<String>,
    /// The `format()` hint: `woff2`, `truetype` or `opentype`.
    pub format: String,
}

/// `print`'s result: the finished deck, printed.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "Printed")]
pub struct PrintedJson<'a> {
    pub deck: &'a Deck,
    pub slides: &'a [SlideTemplate],
    pub media: Vec<MediaJson>,
    pub diagnostics: Vec<DeckDiagnostic>,
    /// The shipped faces' rules, in order; the `n`th rule's file is `fontBytes(n)`.
    pub fonts: Vec<FontJson>,
    /// The fonts panel's faces.
    pub faces: Vec<FontReport>,
    pub font_issues: Vec<FontIssue>,
    /// The bundled substitute faces the deck draws with no file yet: add each as `bundled`, then finish again.
    pub substitutes: Vec<SubstituteFace>,
    /// The page's default title and description.
    pub page: PageDefaults,
}

/// `documents`' result: the document set of a printed deck (`--emit documents`).
#[derive(Clone, Debug, Serialize, TS)]
#[ts(rename = "DocumentSet")]
pub struct DocumentsJson<'a> {
    /// The text files by path.
    pub text: &'a BTreeMap<String, String>,
    /// The paths of the media, whose bytes `mediaBytes` reads.
    pub media: Vec<&'a String>,
}

/// One entry of `zipEntries`.
#[derive(Clone, Debug, Serialize, TS)]
#[ts(rename = "ZipEntry")]
pub struct ZipEntryJson {
    pub name: String,
    pub size: u64,
    pub stored: bool,
    /// The SHA-256 of the entry's bytes, in hex.
    pub sha256: String,
}

/// The declarations of the types above in the module's `.d.ts`, as `ts-rs` prints them (64-bit integers as
/// `number`), with the protocol's types imported from `@slidock/types`.
pub const DECLARATIONS: &str = r#"
import type { AddedFontSource, BundleReport, Deck, DeckDiagnostic, FaceStyle, FontIssue, FontNeed, FontReport, GoogleFace, HeicPlan, LocalPick, PageDefaults, PlannedImage, SlideTemplate, SubstituteFace } from '@slidock/types';

/**
 * `open`'s result.
 */
export type Opened = { diagnostics: Array<DeckDiagnostic>, 
/**
 * The faces no font provides yet.
 */
needs: Array<FontNeed>, 
/**
 * The media the app may decode (HEIC photos).
 */
media: Array<OpenedMediaNeed>, };

/**
 * A media need as the compiler reports it: the protocol's `MediaNeed` with its source's place in the input in place
 * of its `Blob`.
 */
export type OpenedMediaNeed = { 
/**
 * The source item's SHA-256, lowercase hex.
 */
id: string, mime: string, 
/**
 * Where the source file is in the input; `null` for bytes `mediaNeedBytes` reads.
 */
range: MediaRange | null, plan: HeicPlan | null, 
/**
 * Why the compiler plans no decode.
 */
refusal: string | null, };

/**
 * A byte range of the input.
 */
export type MediaRange = { offset: number, length: number, };

/**
 * A file under `media/` of a printed deck.
 */
export type PrintedMedia = { 
/**
 * `media/<sha256>.<ext>`, as the slides reference it.
 */
path: string, mime: string, 
/**
 * Where a copied medium's bytes are in the input; `null` for bytes `mediaBytes` writes.
 */
range: MediaRange | null, };

/**
 * One `@font-face` rule of the shipped faces, as `<style id="sd-fonts">` holds it.
 */
export type PrintedFont = { family: string, 
/**
 * `font-weight`: one weight, or `min max`.
 */
weight: string, style: FaceStyle, 
/**
 * `unicode-range`; `null` for none.
 */
unicodeRange: string | null, 
/**
 * The `format()` hint: `woff2`, `truetype` or `opentype`.
 */
format: string, };

/**
 * `print`'s result: the finished deck, printed.
 */
export type Printed = { deck: Deck, slides: Array<SlideTemplate>, media: Array<PrintedMedia>, diagnostics: Array<DeckDiagnostic>, 
/**
 * The shipped faces' rules, in order; the `n`th rule's file is `fontBytes(n)`.
 */
fonts: Array<PrintedFont>, 
/**
 * The fonts panel's faces.
 */
faces: Array<FontReport>, fontIssues: Array<FontIssue>, 
/**
 * The bundled substitute faces the deck draws with no file yet: add each as `bundled`, then finish again.
 */
substitutes: Array<SubstituteFace>, 
/**
 * The page's default title and description.
 */
page: PageDefaults, };

/**
 * `documents`' result: the document set of a printed deck (`--emit documents`).
 */
export type DocumentSet = { 
/**
 * The text files by path.
 */
text: { [key in string]: string }, 
/**
 * The paths of the media, whose bytes `mediaBytes` reads.
 */
media: Array<string>, };

/**
 * One entry of `zipEntries`.
 */
export type ZipEntry = { name: string, size: number, stored: boolean, 
/**
 * The SHA-256 of the entry's bytes, in hex.
 */
sha256: string, };
"#;
