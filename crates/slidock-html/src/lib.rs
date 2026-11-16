//! The HTML backend: a typed tree for slide and layout documents with a byte-exact serializer that follows the
//! HTML-quality rules (one element per visible object, values printed once, text in text nodes), and the document
//! set `--emit documents` writes: a document per slide and per layout, `presentation.json` and the media they
//! reference.
//!
//! One element per IR element, with its whole box and style inline and `data-el` carrying the IR id. Every number goes
//! through the IR's grid, so the same presentation prints the same bytes on every host. The build steps and transitions
//! are deck data; off the parity path a slide document also lists its steps in an inert timeline template.

mod attributes;
mod color;
mod context;
pub mod css;
pub mod deck;
mod document;
mod error;
mod foreign;
mod frame;
mod gradient;
mod group;
mod manifest;
mod media;
mod paint;
pub mod paint_motion;
mod picture;
pub mod serialize;
mod shadow;
mod shape;
mod supports;
mod svg;
mod table;
mod text;
pub mod timeline;
pub mod tree;
pub mod typescript;

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use slidock_format::manifest::Generator;
use slidock_format::paths::{MANIFEST_PATH, layout_path, slide_path};
use slidock_format::{PresentationFiles, Problem, lint_presentation};
use slidock_ir::diagnostics::Diagnostic;
use slidock_ir::resolved::Presentation;

pub use color::{css_color, literal_color};
pub use context::{DrawnFace, EMITTER, FontStacks, MediaAction, MediaPlanRef, Motion};
pub use error::{HtmlError, Result};
pub use frame::{Autosize, box_decls, text_turn_decl};
pub use manifest::{Provenance, notes_text, presentation_title};
pub use shadow::{ShadowForm, shadow_decl};
pub use supports::supported_features;

use context::{MediaFiles, PrintCtx};

/// What wrote the files, when the caller names no other generator.
pub fn slidock_generator() -> Generator {
    Generator {
        name: "slidock".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
    }
}

/// What a document set records about its making.
#[derive(Clone, Debug, PartialEq)]
pub struct EmitOptions {
    /// Where the presentation came from.
    pub provenance: Provenance,
    pub generator: Generator,
}

impl EmitOptions {
    pub fn new(provenance: Provenance) -> EmitOptions {
        EmitOptions {
            provenance,
            generator: slidock_generator(),
        }
    }
}

/// A document set: the documents and the manifest as text, media as the plans a writer materialises.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Documents {
    /// `presentation.json`, `slides/<fileId>.html` and `layouts/<fileId>.html`.
    pub text: BTreeMap<String, String>,
    /// `media/<sha256>.<ext>` → the source item and how to write it; only media a document references.
    pub media: BTreeMap<String, MediaPlanRef>,
    /// The faces each slide and layout document's text draws, by document path.
    pub faces: BTreeMap<String, BTreeSet<DrawnFace>>,
}

impl Documents {
    /// The set as the lint reads it: text files, and media present by path.
    pub fn lintable(&self) -> PresentationFiles {
        let mut files: PresentationFiles = self
            .text
            .iter()
            .map(|(path, text)| (path.clone(), Some(text.clone())))
            .collect();
        for path in self.media.keys() {
            files.insert(path.clone(), None);
        }
        files
    }

    /// The format's lint over the whole set: every document, the manifest and the media it references.
    pub fn lint(&self) -> Vec<Problem> {
        lint_presentation(&self.lintable())
    }
}

/// The documents and what the backend reported printing them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Emitted {
    pub files: Documents,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Serialize)]
struct Report<'a> {
    media: &'a BTreeMap<String, MediaPlanRef>,
    diagnostics: &'a [Diagnostic],
}

impl Emitted {
    /// The backend's report: the media the documents reference and its diagnostics, as indented canonical JSON.
    pub fn report(&self) -> Result<String> {
        let report = Report {
            media: &self.files.media,
            diagnostics: &self.diagnostics,
        };
        Ok(slidock_canon::to_canonical_json(
            &report,
            slidock_canon::Layout::Indented,
        )?)
    }
}

/// Prints a resolved presentation as a document set: the manifest, then each layout's document (its shown master's
/// non-placeholder elements and its own), then each slide's, then the diagnostics for what the documents leave out.
pub fn emit_documents(presentation: &Presentation, options: &EmitOptions) -> Result<Emitted> {
    emit_documents_shipping(presentation, options, &FontStacks::new(), Motion::Off)
}

/// [`emit_documents`] for a deck: the text of the families `stacks` names (the fonts the deck ships) prints their
/// stacks, and with [`Motion::On`] the documents print the native motion data (the slides' timeline templates, the
/// elements' match keys, the colour and font hooks of the elements such steps animate) and note each step and
/// transition that prints only its fallback (HTML6010, and HTML6011 for a step whose target has no hook).
pub fn emit_documents_shipping(
    presentation: &Presentation,
    options: &EmitOptions,
    stacks: &FontStacks,
    motion: Motion,
) -> Result<Emitted> {
    let mut diagnostics = Vec::new();
    let mut media = MediaFiles::default();
    let mut text = BTreeMap::new();
    let mut faces = BTreeMap::new();
    text.insert(
        MANIFEST_PATH.to_owned(),
        manifest::build_manifest(presentation, &options.provenance, &options.generator)?,
    );
    for layout in presentation.layouts.values() {
        let master = layout
            .master
            .as_ref()
            .and_then(|master| presentation.masters.get(master));
        let mut ctx = PrintCtx::new(presentation, &layout.id, &mut media, &mut diagnostics)
            .with_stacks(stacks)
            .with_motion(motion);
        let html = document::layout_document(layout, master, &mut ctx)?;
        let path = layout_path(layout.id.as_str());
        faces.insert(path.clone(), ctx.into_faces());
        text.insert(path, html);
    }
    for slide in &presentation.slides {
        let mut ctx = PrintCtx::new(presentation, &slide.id, &mut media, &mut diagnostics)
            .with_stacks(stacks)
            .with_motion(motion);
        let html = document::slide_document(slide, &mut ctx)?;
        let path = slide_path(slide.id.as_str());
        faces.insert(path.clone(), ctx.into_faces());
        text.insert(path, html);
    }
    if motion == Motion::On {
        diagnostics.extend(timeline::fallback_notes(presentation)?);
    }
    diagnostics.extend(supports::unsupported_diagnostics(presentation)?);
    Ok(Emitted {
        files: Documents {
            text,
            media: media.files,
            faces,
        },
        diagnostics,
    })
}
