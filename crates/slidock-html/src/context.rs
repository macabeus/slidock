//! What every printer of one container shares: the presentation, the container's diagnostics, the media its document
//! references and the faces its text draws.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use slidock_format::paths::media_reference;
use slidock_ir::diagnostics::{Diagnostic, DiagnosticCode, DiagnosticSite, Stage, make_diagnostic};
use slidock_ir::ids::{NodeId, Sha256};
use slidock_ir::media::{Mime, PlanAction};
use slidock_ir::resolved::{Element, Presentation};
use slidock_ir::shadow::Axes;

use crate::error::Result;
use crate::paint_motion::{Hook, Hooks};
use crate::tree::Element as Html;

/// Who the backend's diagnostics name as their emitter.
pub const EMITTER: &str = "html";

/// How the writer makes a media file: a copy of the source bytes, or a transcode of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MediaAction {
    Copy,
    Transcode,
}

/// A media file the output holds: the source item it comes from and how the writer makes it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MediaPlanRef {
    pub sha256: Sha256,
    pub action: MediaAction,
    #[serde(rename = "type")]
    pub mime: Mime,
    /// The source's pixel count, when its frontend sniffed it: what a transcode decodes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pixels: Option<f64>,
}

/// A face a document's text draws: the family as the text names it, the weight it asks for (before its binding
/// maps it) and the style.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DrawnFace {
    pub family: String,
    pub weight: u32,
    pub italic: bool,
}

/// The media files the printed documents reference, by path.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MediaFiles {
    pub files: BTreeMap<String, MediaPlanRef>,
}

impl MediaFiles {
    /// The document reference of a media item, or `None` when the plan writes no file for it.
    pub fn reference(
        &mut self,
        presentation: &Presentation,
        sha256: &Sha256,
    ) -> Result<Option<String>> {
        let Some(entry) = presentation.media.get(sha256) else {
            return Ok(None);
        };
        let plan = &entry.plan;
        let (Some(output), action) = (&plan.output, plan.action) else {
            return Ok(None);
        };
        let action = match action {
            PlanAction::Omit => return Ok(None),
            PlanAction::Copy => MediaAction::Copy,
            PlanAction::Transcode => MediaAction::Transcode,
        };
        let pixels = match (entry.width, entry.height) {
            (Some(w), Some(h)) if w != 0 && h != 0 => Some(w as f64 * h as f64),
            _ => None,
        };
        self.files.insert(
            output.file.as_str().to_owned(),
            MediaPlanRef {
                sha256: *sha256,
                action,
                mime: output.mime.clone(),
                pixels,
            },
        );
        Ok(Some(media_reference(output.file.as_str())?))
    }
}

/// The families a font stack names before the generic, by the font key of the family the slides name, for each family
/// the deck ships faces of: the families its `@font-face` rules define. A family without an entry
/// prints the stack it prints when the deck ships no font.
pub type FontStacks = BTreeMap<String, Vec<String>>;

/// No shipped family.
static NO_STACKS: FontStacks = FontStacks::new();

/// Whether documents print the native motion data: a slide's timeline template, the elements' match keys, their
/// colour and font hooks, and the fallback notes (HTML6010, HTML6011).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Motion {
    /// The parity path: the documents are those of the IR without the effect vocabulary.
    #[default]
    Off,
    On,
}

/// A printing context for one container.
pub struct PrintCtx<'a> {
    pub presentation: &'a Presentation,
    /// The container whose document is being printed.
    pub container: &'a NodeId,
    /// The stacks of the families the deck ships.
    pub stacks: &'a FontStacks,
    pub motion: Motion,
    media: &'a mut MediaFiles,
    diagnostics: &'a mut Vec<Diagnostic>,
    faces: BTreeSet<DrawnFace>,
    /// The colour and font hooks the container's elements declare, by element id (the deck path only).
    hooks: BTreeMap<String, Hooks>,
    /// The axes of the box the element being printed sits in: the rotations and flips printed above it.
    pub axes: Axes,
    /// The axes of the text box whose runs are being printed.
    pub text_axes: Axes,
}

impl<'a> PrintCtx<'a> {
    pub fn new(
        presentation: &'a Presentation,
        container: &'a NodeId,
        media: &'a mut MediaFiles,
        diagnostics: &'a mut Vec<Diagnostic>,
    ) -> PrintCtx<'a> {
        PrintCtx {
            presentation,
            container,
            stacks: &NO_STACKS,
            motion: Motion::Off,
            media,
            diagnostics,
            faces: BTreeSet::new(),
            hooks: BTreeMap::new(),
            axes: Axes::PAGE,
            text_axes: Axes::PAGE,
        }
    }

    /// Prints `elements` placed in a box whose own axes are `axes`, the axes then restored.
    pub fn print_in(&mut self, axes: Axes, elements: &[Element]) -> Result<Vec<Html>> {
        let outer = std::mem::replace(&mut self.axes, axes);
        let printed = elements
            .iter()
            .map(|element| self.print(element))
            .collect::<Result<Vec<_>>>();
        self.axes = outer;
        printed
    }

    /// Sets the colour and font hooks of the container's elements.
    pub fn set_hooks(&mut self, hooks: BTreeMap<String, Hooks>) {
        self.hooks = hooks;
    }

    /// The static value of an element's hook, when it declares one.
    pub fn hook(&self, element: &str, hook: Hook) -> Option<&str> {
        self.hooks
            .get(element)
            .and_then(|hooks| hooks.get(&hook))
            .map(String::as_str)
    }

    /// The hooks an element declares.
    pub fn hooks(&self, element: &str) -> Option<&Hooks> {
        self.hooks.get(element)
    }

    /// The context with the stacks of the families the deck ships.
    pub fn with_stacks(self, stacks: &'a FontStacks) -> PrintCtx<'a> {
        PrintCtx { stacks, ..self }
    }

    /// The context printing the native motion data or not.
    pub fn with_motion(self, motion: Motion) -> PrintCtx<'a> {
        PrintCtx { motion, ..self }
    }

    /// Records a face the document's text draws: a glyph-bearing run or field, a paragraph's strut, a list label.
    pub fn drew(&mut self, family: &str, weight: u32, italic: bool) {
        self.faces.insert(DrawnFace {
            family: family.to_owned(),
            weight,
            italic,
        });
    }

    /// The faces the document drew.
    pub fn into_faces(self) -> BTreeSet<DrawnFace> {
        self.faces
    }

    /// Reports a backend diagnostic.
    pub fn diag(&mut self, code: DiagnosticCode, site: DiagnosticSite) {
        self.diagnostics
            .push(make_diagnostic(code, Stage::Backend, EMITTER, site));
    }

    /// A site in this container.
    pub fn site(&self) -> DiagnosticSite {
        DiagnosticSite::new().container(self.container.clone())
    }

    /// The document reference of a media item, recording the file the output then holds.
    pub fn media_ref(&mut self, sha256: &Sha256) -> Result<Option<String>> {
        self.media.reference(self.presentation, sha256)
    }

    /// An element as the HTML element that prints it.
    pub fn print(&mut self, element: &Element) -> Result<Html> {
        match element {
            Element::Shape(shape) => crate::shape::print(shape, self),
            Element::Picture(picture) => crate::picture::print(picture, self),
            Element::Media(media) => crate::media::print(media, self),
            Element::Table(table) => crate::table::print(table, self),
            Element::Group(group) => crate::group::print(group, self),
            Element::Foreign(foreign) => crate::foreign::print(foreign, self),
        }
    }
}
