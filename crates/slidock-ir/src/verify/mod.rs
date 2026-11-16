//! The verifier: the source schema (structure, grids and bounds), the phase, the graph rules and the invariants, then
//! the stricter resolved schema.
//!
//! Structure is checked while reading: the first structural error, with zod's path, is IR0001, and a number off
//! its grid is IR0007 among its unit's failed checks, in zod's order ([`crate::units::Check`]). Bounds are checked
//! on the parsed value, every issue reported. zod reports every structural issue at once, and words its reasons
//! its own way; only deliberately broken IR input sees those differences ([`crate::parse`] lists the others).

mod graph;
mod invariants;
mod issues;
mod narrow;
mod visit;
mod walk;

use indexmap::IndexMap;
use serde_json::Value;

use crate::diagnostics::{Diagnostic, DiagnosticSite, Stage, codes, make_diagnostic};
use crate::ids::{LayoutId, MasterId, NodeId, ThemeId};
use crate::presentation::Phase;
use crate::{parse, resolved, source, validate};

pub use graph::{GraphOptions, verify_graph};
pub use invariants::{
    CheckOptions, Invariant, check_invariant, check_invariants, verify_invariants,
};
pub use narrow::{
    CascadedPara, CascadedRun, InvariantError, LabelledBullet, LineMetrics, binding, cascaded_para,
    cascaded_run, complete_body, family_font, fitted_autofit, framed, labelled_bullet,
    line_metrics, lowered_fill, lowered_geometry, media_plan, resolved_color, text_rotation,
};
pub use walk::{
    ContainerRef, ElementRef, RenderedText, container_elements, containers_of, element_texts,
    rendered_text, walk_elements,
};

/// Options of the verifier.
#[derive(Clone, Debug, Default)]
pub struct VerifyOptions {
    /// Slides a slide link may target besides the presentation's own.
    pub linkable_slides: Vec<NodeId>,
}

const PARSE: &str = "parse";
const VERIFY_SOURCE: &str = "verify-source";
const VERIFY_RESOLVED: &str = "verify-resolved";

/// Bound issues of a typed presentation, as diagnostics.
pub fn validate_presentation(
    presentation: &source::Presentation,
    emitter: &str,
) -> Vec<Diagnostic> {
    issues::issue_diagnostics(&validate::validate(presentation), presentation, emitter)
}

/// Reads parsed JSON as a source presentation through the source schema: structure, grids and bounds.
pub fn parse_presentation(
    input: &Value,
    emitter: &str,
) -> Result<source::Presentation, Vec<Diagnostic>> {
    let presentation: source::Presentation = parse::from_value(input.clone())
        .map_err(|error| issues::structure_diagnostics(&error, input, emitter))?;
    let issues = validate::validate(&presentation);
    if issues.is_empty() {
        Ok(presentation)
    } else {
        Err(issues::issue_diagnostics(&issues, &presentation, emitter))
    }
}

/// [`parse_presentation`] with the default emitter.
pub fn parse_presentation_json(input: &Value) -> Result<source::Presentation, Vec<Diagnostic>> {
    parse_presentation(input, PARSE)
}

fn phase_and_graph(
    presentation: &source::Presentation,
    options: &VerifyOptions,
    emitter: &str,
) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    if presentation.phase != Phase::Source {
        out.push(make_diagnostic(
            codes::IR0009,
            Stage::Verify,
            emitter,
            DiagnosticSite::new()
                .data("expected", "source")
                .data("found", presentation.phase.as_str()),
        ));
    }
    out.extend(verify_graph(
        presentation,
        &GraphOptions {
            emitter: Some(emitter.to_owned()),
            linkable_slides: options.linkable_slides.clone(),
        },
    ));
    out
}

/// Verifies a presentation a frontend or an agent wrote, as JSON: schema (with quantisation and bounds), phase, and
/// the graph rules, which run only on a schema-valid presentation.
pub fn verify_source_json(input: &Value, options: &VerifyOptions) -> Vec<Diagnostic> {
    match parse_presentation(input, VERIFY_SOURCE) {
        Ok(presentation) => phase_and_graph(&presentation, options, VERIFY_SOURCE),
        Err(diagnostics) => diagnostics,
    }
}

/// Verifies a typed source presentation: bounds, phase and the graph rules.
pub fn verify_source(
    presentation: &source::Presentation,
    options: &VerifyOptions,
) -> Vec<Diagnostic> {
    let bounds = validate_presentation(presentation, VERIFY_SOURCE);
    if !bounds.is_empty() {
        return bounds;
    }
    phase_and_graph(presentation, options, VERIFY_SOURCE)
}

fn check_options(options: &VerifyOptions, emitter: &str) -> CheckOptions {
    CheckOptions {
        graph: GraphOptions {
            emitter: Some(emitter.to_owned()),
            linkable_slides: options.linkable_slides.clone(),
        },
        containers: None,
    }
}

/// The last step of the pass manager: bounds, the graph rules, every invariant checker, then the stricter resolved
/// schema, with the phase flipped to resolved. A presentation already marked resolved is accepted, so a resolved
/// presentation can be re-verified. Failed containers are exempt from the resolved-only rules.
pub fn verify_resolved(
    presentation: &source::Presentation,
    options: &VerifyOptions,
) -> Result<resolved::Presentation, Vec<Diagnostic>> {
    let bounds = validate_presentation(presentation, VERIFY_RESOLVED);
    if !bounds.is_empty() {
        return Err(bounds);
    }
    let violations = verify_invariants(
        presentation,
        &Invariant::RESOLVED,
        &check_options(options, VERIFY_RESOLVED),
    );
    if !violations.is_empty() {
        return Err(violations);
    }
    convert(presentation, Some(Phase::Resolved)).map_err(|failed| {
        let diagnostic = |path: &str, reason: String| {
            make_diagnostic(
                codes::IR0001,
                Stage::Verify,
                VERIFY_RESOLVED,
                DiagnosticSite::new()
                    .data("path", path)
                    .data("reason", reason),
            )
        };
        match failed {
            Converted::Unwritable(reason) => vec![diagnostic("(presentation)", reason)],
            Converted::Structure(error, value) => {
                issues::structure_diagnostics(&error, &value, VERIFY_RESOLVED)
            }
        }
    })
}

/// [`verify_resolved`] on JSON input: the source schema first.
pub fn verify_resolved_json(
    input: &Value,
    options: &VerifyOptions,
) -> Result<resolved::Presentation, Vec<Diagnostic>> {
    let presentation = parse_presentation(input, VERIFY_RESOLVED)?;
    verify_resolved(&presentation, options)
}

impl TryFrom<source::Presentation> for resolved::Presentation {
    type Error = Vec<Diagnostic>;

    /// Narrows a source presentation to the resolved phase: [`verify_resolved`] with default options, whose
    /// errors are the IR01xx violations (or the schema's IR00xx).
    fn try_from(presentation: source::Presentation) -> Result<Self, Self::Error> {
        verify_resolved(&presentation, &VerifyOptions::default())
    }
}

impl resolved::Presentation {
    /// The presentation in the source phase's types (the source phase is a superset of the resolved one).
    pub fn to_source(&self) -> Result<source::Presentation, parse::StructureError> {
        convert(self, None).map_err(Converted::into_structure)
    }

    /// A source presentation read in the resolved schema as it is, unverified: what a backend made of the
    /// presentation it printed, with containers failed in the source form.
    pub fn from_source(
        presentation: &source::Presentation,
    ) -> Result<resolved::Presentation, parse::StructureError> {
        convert(presentation, None).map_err(Converted::into_structure)
    }
}

/// Why a presentation did not read in the other phase's types: it did not write as JSON, or the JSON does not read
/// (the error, with the whole presentation's JSON its path points into).
enum Converted {
    Unwritable(String),
    Structure(parse::StructureError, Value),
}

impl Converted {
    fn into_structure(self) -> parse::StructureError {
        match self {
            Converted::Unwritable(reason) => parse::StructureError {
                path: crate::path::IrPath::root(),
                issues: vec![parse::StructureIssue {
                    code: codes::IR0001,
                    detail: crate::validate::IssueDetail::Reason(reason),
                }],
            },
            Converted::Structure(error, _) => error,
        }
    }
}

/// A presentation of one phase's types read as the other's, through JSON, as if whole (its phase set to `phase`
/// when given): the presentation without its containers, then each container without its elements, then each
/// element, so no more than one element's JSON is held at a time. When a piece does not read, the whole presentation
/// is read again, for the error and its path from the root.
fn convert<From, To>(presentation: &From, phase: Option<Phase>) -> Result<To, Converted>
where
    From: Phased,
    To: Phased,
{
    /// A container's element sits four levels deep: in the root object's `masters`, `layouts` or `slides`, in the
    /// container's `elements`.
    fn container<T: Split, U: Split>(container: &T) -> Option<U> {
        let shell = serde_json::to_value(container.without_elements()).ok()?;
        let mut out: U = parse::from_value_at(shell, 2).ok()?;
        let elements = container
            .elements()
            .iter()
            .map(|element| parse::from_value_at(serde_json::to_value(element).ok()?, 4).ok())
            .collect::<Option<Vec<_>>>()?;
        out.set_elements(elements).then_some(out)
    }
    let by_container = || -> Option<To> {
        let mut shell = serde_json::to_value(presentation.shell()).ok()?;
        set_phase(&mut shell, phase);
        let mut out: To = parse::from_value_at(shell, 0).ok()?;
        let (masters, layouts, slides) = presentation.containers();
        let (into_masters, into_layouts, into_slides) = out.containers_mut();
        for (id, master) in masters {
            into_masters.insert(id.clone(), container(master)?);
        }
        for (id, layout) in layouts {
            into_layouts.insert(id.clone(), container(layout)?);
        }
        for slide in slides {
            into_slides.push(container(slide)?);
        }
        Some(out)
    };
    if let Some(out) = by_container() {
        return Ok(out);
    }
    let mut value = serde_json::to_value(presentation)
        .map_err(|error| Converted::Unwritable(error.to_string()))?;
    set_phase(&mut value, phase);
    parse::from_value(value.clone()).map_err(|error| Converted::Structure(error, value))
}

/// A container of either phase, as [`convert`] reads it: itself without its elements, its elements, and elements
/// put in a container read without them.
trait Split: serde::Serialize + serde::de::DeserializeOwned {
    type Element: serde::Serialize + serde::de::DeserializeOwned;

    fn without_elements(&self) -> Self;

    fn elements(&self) -> &[Self::Element];

    /// Sets the elements of a container read without them; `false` when it cannot hold them (a failed resolved
    /// container holds none).
    fn set_elements(&mut self, elements: Vec<Self::Element>) -> bool;
}

macro_rules! split_source {
    ($ty:ident { $($field:ident),* }) => {
        impl Split for source::$ty {
            type Element = source::Element;

            fn without_elements(&self) -> Self {
                let source::$ty { $($field,)* elements: _ } = self;
                source::$ty {
                    $($field: $field.clone(),)*
                    elements: Vec::new(),
                }
            }

            fn elements(&self) -> &[source::Element] {
                &self.elements
            }

            fn set_elements(&mut self, elements: Vec<source::Element>) -> bool {
                self.elements = elements;
                true
            }
        }
    };
}

split_source!(Master {
    id,
    origin,
    name,
    theme,
    color_map,
    text_styles,
    ext,
    background,
    failed
});
split_source!(Layout {
    id,
    origin,
    name,
    layout_type,
    master,
    show_master,
    color_map_override,
    ext,
    background,
    failed
});
split_source!(Slide {
    id,
    origin,
    layout,
    hidden,
    show_layout,
    color_map_override,
    notes,
    transition,
    timeline,
    ext,
    background,
    title,
    reading_order,
    failed
});

/// A resolved body without its elements.
fn body_without_elements<B: Clone>(
    body: &resolved::Body<B>,
    empty: impl Fn(&B) -> B,
) -> resolved::Body<B> {
    match body {
        resolved::Body::Built(built) => resolved::Body::Built(empty(built)),
        failed @ resolved::Body::Failed { .. } => failed.clone(),
    }
}

fn built_without_elements(built: &resolved::Built) -> resolved::Built {
    let resolved::Built {
        background,
        elements: _,
    } = built;
    resolved::Built {
        background: background.clone(),
        elements: Vec::new(),
    }
}

macro_rules! split_resolved {
    ($ty:ident { $($field:ident),* }, $empty:expr) => {
        impl Split for resolved::$ty {
            type Element = resolved::Element;

            fn without_elements(&self) -> Self {
                let resolved::$ty { $($field,)* body } = self;
                resolved::$ty {
                    $($field: $field.clone(),)*
                    body: body_without_elements(body, $empty),
                }
            }

            fn elements(&self) -> &[resolved::Element] {
                self.body.elements()
            }

            fn set_elements(&mut self, elements: Vec<resolved::Element>) -> bool {
                match &mut self.body {
                    resolved::Body::Built(built) => {
                        built.elements = elements;
                        true
                    }
                    resolved::Body::Failed { .. } => elements.is_empty(),
                }
            }
        }
    };
}

split_resolved!(
    Master {
        id,
        origin,
        name,
        theme,
        color_map,
        text_styles,
        ext
    },
    built_without_elements
);
split_resolved!(
    Layout {
        id,
        origin,
        name,
        layout_type,
        master,
        show_master,
        color_map_override,
        ext
    },
    built_without_elements
);
split_resolved!(
    Slide {
        id,
        origin,
        layout,
        hidden,
        show_layout,
        color_map_override,
        notes,
        transition,
        timeline,
        ext,
        failed_a11y
    },
    |built: &resolved::BuiltSlide| {
        let resolved::BuiltSlide {
            background,
            elements: _,
            title,
            reading_order,
        } = built;
        resolved::BuiltSlide {
            background: background.clone(),
            elements: Vec::new(),
            title: title.clone(),
            reading_order: reading_order.clone(),
        }
    }
);

fn set_phase(value: &mut Value, phase: Option<Phase>) {
    if let (Some(phase), Some(object)) = (phase, value.as_object_mut()) {
        object.insert("phase".to_owned(), Value::from(phase.as_str()));
    }
}

/// A presentation type of either phase, as [`convert`] reads it: its fields without the containers, and the
/// containers.
trait Phased: serde::Serialize + serde::de::DeserializeOwned {
    type Master: Split;
    type Layout: Split;
    type Slide: Split;
    type Shell<'a>: serde::Serialize
    where
        Self: 'a;

    fn shell(&self) -> Self::Shell<'_>;

    #[allow(clippy::type_complexity, reason = "the three container collections")]
    fn containers(
        &self,
    ) -> (
        &IndexMap<MasterId, Self::Master>,
        &IndexMap<LayoutId, Self::Layout>,
        &[Self::Slide],
    );

    #[allow(clippy::type_complexity, reason = "the three container collections")]
    fn containers_mut(
        &mut self,
    ) -> (
        &mut IndexMap<MasterId, Self::Master>,
        &mut IndexMap<LayoutId, Self::Layout>,
        &mut Vec<Self::Slide>,
    );
}

/// A presentation's fields with empty containers, serialised as the presentation is.
#[derive(serde::Serialize)]
struct Shell<'a, Schema, Phase, Fonts, Media> {
    #[serde(rename = "$schema", skip_serializing_if = "Option::is_none")]
    schema: &'a Option<Schema>,
    size: &'a crate::presentation::SlideSize,
    defaults: &'a crate::presentation::Defaults,
    themes: &'a IndexMap<ThemeId, crate::presentation::Theme>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ext: &'a Option<crate::json_value::Ext>,
    phase: &'a Phase,
    masters: Empty,
    layouts: Empty,
    slides: [(); 0],
    fonts: &'a Fonts,
    media: &'a Media,
}

/// An empty JSON object.
struct Empty;

impl serde::Serialize for Empty {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        serializer.serialize_map(Some(0))?.end()
    }
}

macro_rules! phased {
    ($phase:ident, $schema:ty, $phase_ty:ty, $fonts:ty, $media:ty) => {
        impl Phased for $phase::Presentation {
            type Master = $phase::Master;
            type Layout = $phase::Layout;
            type Slide = $phase::Slide;
            type Shell<'a> = Shell<'a, $schema, $phase_ty, $fonts, $media>;

            fn shell(&self) -> Self::Shell<'_> {
                Shell {
                    schema: &self.schema,
                    size: &self.size,
                    defaults: &self.defaults,
                    themes: &self.themes,
                    ext: &self.ext,
                    phase: &self.phase,
                    masters: Empty,
                    layouts: Empty,
                    slides: [],
                    fonts: &self.fonts,
                    media: &self.media,
                }
            }

            fn containers(
                &self,
            ) -> (
                &IndexMap<MasterId, Self::Master>,
                &IndexMap<LayoutId, Self::Layout>,
                &[Self::Slide],
            ) {
                (&self.masters, &self.layouts, &self.slides)
            }

            fn containers_mut(
                &mut self,
            ) -> (
                &mut IndexMap<MasterId, Self::Master>,
                &mut IndexMap<LayoutId, Self::Layout>,
                &mut Vec<Self::Slide>,
            ) {
                (&mut self.masters, &mut self.layouts, &mut self.slides)
            }
        }
    };
}

phased!(
    source,
    crate::presentation::SchemaUrl,
    Phase,
    IndexMap<String, crate::fonts::FontEntry>,
    IndexMap<crate::ids::Sha256, crate::media::MediaEntry>
);
phased!(
    resolved,
    crate::presentation::SchemaUrl,
    crate::presentation::ResolvedPhase,
    IndexMap<String, crate::fonts::ResolvedFontEntry>,
    IndexMap<crate::ids::Sha256, crate::media::ResolvedMediaEntry>
);
