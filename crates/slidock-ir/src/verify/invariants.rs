//! Invariants: what the presentation guarantees after each pass that provides it. The pass manager checks every
//! invariant provided so far after each pass and attributes a violation to the pass and container.

use std::collections::BTreeSet;
use std::fmt;

use crate::color::ThemeColor;
use crate::diagnostics::{Diagnostic, DiagnosticCode, DiagnosticSite, codes};
use crate::fonts::font_key;
use crate::geometry::Geometry;
use crate::ids::NodeId;
use crate::path::{IrPath, PathSeg};
use crate::source::{Container, Element, Presentation};
use crate::table::table_cells;
use crate::text::{Autofit, Bullet, FontRef, Inline, OrText, PartialRunProps};
use crate::verify::graph::{Emit, GraphOptions, reference_diagnostics, verify_graph};
use crate::verify::visit::{ObjectVisitor, Walk};
use crate::verify::walk::{ContainerRef, container_elements, containers_of, rendered_text};

/// An invariant, in pass order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Invariant {
    SourceValid,
    FramesCanonical,
    PlaceholdersBound,
    InheritanceResolved,
    ThemeResolved,
    TextCascaded,
    ListsLabelled,
    FontsBound,
    TextMetrics,
    GeometryLowered,
    MediaPlanned,
    TimelineNormalized,
    A11yDone,
}

impl Invariant {
    pub const ALL: [Invariant; 13] = [
        Invariant::SourceValid,
        Invariant::FramesCanonical,
        Invariant::PlaceholdersBound,
        Invariant::InheritanceResolved,
        Invariant::ThemeResolved,
        Invariant::TextCascaded,
        Invariant::ListsLabelled,
        Invariant::FontsBound,
        Invariant::TextMetrics,
        Invariant::GeometryLowered,
        Invariant::MediaPlanned,
        Invariant::TimelineNormalized,
        Invariant::A11yDone,
    ];

    /// Every resolved-phase invariant, in pass order.
    pub const RESOLVED: [Invariant; 12] = [
        Invariant::FramesCanonical,
        Invariant::PlaceholdersBound,
        Invariant::InheritanceResolved,
        Invariant::ThemeResolved,
        Invariant::TextCascaded,
        Invariant::ListsLabelled,
        Invariant::FontsBound,
        Invariant::TextMetrics,
        Invariant::GeometryLowered,
        Invariant::MediaPlanned,
        Invariant::TimelineNormalized,
        Invariant::A11yDone,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Invariant::SourceValid => "source.valid",
            Invariant::FramesCanonical => "frames.canonical",
            Invariant::PlaceholdersBound => "placeholders.bound",
            Invariant::InheritanceResolved => "inheritance.resolved",
            Invariant::ThemeResolved => "theme.resolved",
            Invariant::TextCascaded => "text.cascaded",
            Invariant::ListsLabelled => "lists.labelled",
            Invariant::FontsBound => "fonts.bound",
            Invariant::TextMetrics => "text.metrics",
            Invariant::GeometryLowered => "geometry.lowered",
            Invariant::MediaPlanned => "media.planned",
            Invariant::TimelineNormalized => "timeline.normalized",
            Invariant::A11yDone => "a11y.done",
        }
    }

    pub fn parse(name: &str) -> Option<Invariant> {
        Self::ALL
            .into_iter()
            .find(|invariant| invariant.as_str() == name)
    }

    /// The rules the invariant's checker owns.
    pub const fn rules(self) -> &'static [DiagnosticCode] {
        match self {
            Invariant::SourceValid => &[
                codes::IR0001,
                codes::IR0002,
                codes::IR0003,
                codes::IR0004,
                codes::IR0005,
                codes::IR0006,
                codes::IR0007,
                codes::IR0008,
                codes::IR0011,
                codes::IR0014,
                codes::IR0015,
            ],
            Invariant::FramesCanonical => &[codes::IR0105],
            Invariant::PlaceholdersBound => &[codes::IR0003],
            Invariant::InheritanceResolved => &[codes::IR0108, codes::IR0115],
            Invariant::ThemeResolved => &[codes::IR0101, codes::IR0114],
            Invariant::TextCascaded => &[codes::IR0102, codes::IR0103],
            Invariant::ListsLabelled => &[codes::IR0111],
            Invariant::FontsBound => &[codes::IR0104],
            Invariant::TextMetrics => &[codes::IR0110, codes::IR0107],
            Invariant::GeometryLowered => &[codes::IR0105, codes::IR0113],
            Invariant::MediaPlanned => &[codes::IR0106],
            Invariant::TimelineNormalized => &[codes::IR0003],
            Invariant::A11yDone => &[codes::IR0109],
        }
    }

    /// The IR0003 kinds a graph invariant holds (`None`: all the graph rules); `None` for a checked invariant.
    fn graph_kinds(self) -> Option<Option<&'static [&'static str]>> {
        match self {
            Invariant::SourceValid => Some(None),
            Invariant::PlaceholdersBound => Some(Some(&["inherits", "placeholder-target"])),
            Invariant::TimelineNormalized => Some(Some(&["timeline-target"])),
            _ => None,
        }
    }

    fn is_graph(self) -> bool {
        self.graph_kinds().is_some()
    }
}

impl fmt::Display for Invariant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Options of the invariant checks.
#[derive(Clone, Debug, Default)]
pub struct CheckOptions {
    pub graph: GraphOptions,
    /// Only these containers; all when absent. Presentation-level diagnostics always pass.
    pub containers: Option<BTreeSet<NodeId>>,
}

impl CheckOptions {
    fn in_scope(&self, id: Option<&NodeId>) -> bool {
        match (id, &self.containers) {
            (None, _) | (_, None) => true,
            (Some(id), Some(containers)) => containers.contains(id),
        }
    }
}

fn at(path: &IrPath) -> String {
    path.to_string()
}

fn site(container: &ContainerRef<'_>) -> DiagnosticSite {
    DiagnosticSite::new().container(container.container.id().clone())
}

fn frames_canonical(refs: &[ContainerRef<'_>], emit: &mut Emit) {
    for container in refs {
        for element in container_elements(container) {
            let Element::Group(group) = element.element else {
                continue;
            };
            if group.child_space.is_some() {
                emit.emit(
                    codes::IR0105,
                    site(container)
                        .node(group.id.clone())
                        .data("path", at(&element.path.with("childSpace")))
                        .data("reason", "a group child space remains"),
                );
            }
            if group
                .frame
                .as_ref()
                .is_some_and(|frame| frame.flip_h || frame.flip_v)
            {
                emit.emit(
                    codes::IR0105,
                    site(container)
                        .node(group.id.clone())
                        .data("path", at(&element.path.with("frame")))
                        .data("reason", "a group flips its children"),
                );
            }
        }
    }
}

fn inheritance_resolved(refs: &[ContainerRef<'_>], emit: &mut Emit) {
    for container in refs {
        for element in container_elements(container) {
            if element.element.frame().is_none() {
                emit.emit(
                    codes::IR0108,
                    site(container)
                        .node(element.element.id().clone())
                        .data("path", at(&element.path)),
                );
            }
        }
        for body in rendered_text(container) {
            let missing = body.text.body.missing_keys();
            if !missing.is_empty() {
                emit.emit(
                    codes::IR0115,
                    site(container)
                        .node(body.node.clone())
                        .data("path", at(&body.path.with("body")))
                        .data("missing", missing.join(", ")),
                );
            }
        }
    }
}

struct ThemeRefs<'c, 'e> {
    container: &'c ContainerRef<'c>,
    emit: &'e mut Emit,
}

impl ObjectVisitor for ThemeRefs<'_, '_> {
    fn theme_color(&mut self, path: &IrPath, color: &ThemeColor) {
        if color.resolved.is_none() {
            self.emit
                .emit(codes::IR0101, site(self.container).data("path", at(path)));
        }
    }

    fn theme_font(&mut self, path: &IrPath) {
        self.emit
            .emit(codes::IR0114, site(self.container).data("path", at(path)));
    }
}

/// Theme references anywhere a container renders from: elements, backgrounds, list and master text styles.
fn theme_resolved(refs: &[ContainerRef<'_>], emit: &mut Emit) {
    for container in refs {
        let mut visitor = ThemeRefs { container, emit };
        Walk::container(&mut visitor, container.container, &container.path, false);
    }
}

fn text_cascaded(refs: &[ContainerRef<'_>], emit: &mut Emit) {
    for container in refs {
        for body in rendered_text(container) {
            for (p, paragraph) in body.text.paragraphs.iter().enumerate() {
                let paragraph_path = body
                    .path
                    .join([PathSeg::from("paragraphs"), PathSeg::Index(p)]);
                let missing = paragraph.props.missing_cascaded_keys();
                if !missing.is_empty() {
                    emit.emit(
                        codes::IR0102,
                        site(container)
                            .node(body.node.clone())
                            .data("path", at(&paragraph_path))
                            .data("missing", missing.join(", ")),
                    );
                }
                let mut runs: Vec<(&PartialRunProps, IrPath)> = Vec::new();
                for (i, inline) in paragraph.inlines.iter().enumerate() {
                    let props = match inline {
                        Inline::Run { props, .. } | Inline::Field { props, .. } => Some(props),
                        Inline::Break { props } => props.as_ref(),
                        Inline::Tab {} => None,
                    };
                    if let Some(props) = props {
                        runs.push((
                            props,
                            paragraph_path.join([PathSeg::from("inlines"), PathSeg::Index(i)]),
                        ));
                    }
                }
                if let Some(end) = &paragraph.end_props {
                    runs.push((end, paragraph_path.with("endProps")));
                }
                for (props, path) in runs {
                    let lacking = props.missing_keys();
                    if !lacking.is_empty() {
                        emit.emit(
                            codes::IR0103,
                            site(container)
                                .node(body.node.clone())
                                .data("path", at(&path))
                                .data("missing", lacking.join(", ")),
                        );
                    }
                }
            }
        }
    }
}

fn lists_labelled(refs: &[ContainerRef<'_>], emit: &mut Emit) {
    for container in refs {
        for body in rendered_text(container) {
            for (p, paragraph) in body.text.paragraphs.iter().enumerate() {
                let Some(bullet) = &paragraph.props.bullet else {
                    continue;
                };
                if matches!(bullet, Bullet::Char { .. } | Bullet::Auto { .. }) {
                    emit.emit(
                        codes::IR0111,
                        site(container)
                            .node(body.node.clone())
                            .data(
                                "path",
                                at(&body
                                    .path
                                    .join([PathSeg::from("paragraphs"), PathSeg::Index(p)])),
                            )
                            .data("bullet", bullet.kind()),
                    );
                }
            }
        }
    }
}

fn family_of(font: Option<&FontRef>) -> Option<&str> {
    match font {
        Some(FontRef::Family(family)) => Some(family.family.as_str()),
        _ => None,
    }
}

fn fonts_bound(presentation: &Presentation, refs: &[ContainerRef<'_>], emit: &mut Emit) {
    for (key, entry) in &presentation.fonts {
        if entry.binding.is_none() {
            emit.emit(
                codes::IR0104,
                DiagnosticSite::new()
                    .data("family", entry.request.family.as_str())
                    .data("key", key.as_str()),
            );
        }
    }
    for container in refs {
        let mut reported: BTreeSet<String> = BTreeSet::new();
        let mut check = |emit: &mut Emit, family: Option<&str>, node: &NodeId| {
            let Some(family) = family else { return };
            let key = font_key(family);
            let bound = presentation
                .fonts
                .get(&key)
                .is_some_and(|entry| entry.binding.is_some());
            if !bound && reported.insert(key.clone()) {
                emit.emit(
                    codes::IR0104,
                    site(container)
                        .node(node.clone())
                        .data("family", family)
                        .data("key", key),
                );
            }
        };
        for body in rendered_text(container) {
            for paragraph in &body.text.paragraphs {
                match &paragraph.props.bullet {
                    Some(Bullet::Char { font, .. } | Bullet::Auto { font, .. }) => {
                        let font = match font {
                            Some(OrText::Value(font)) => Some(font),
                            _ => None,
                        };
                        check(emit, family_of(font), body.node);
                    }
                    Some(Bullet::Label { font, .. }) => {
                        check(emit, Some(font.family.as_str()), body.node)
                    }
                    Some(Bullet::None {}) | None => {}
                }
                for inline in &paragraph.inlines {
                    let props = inline.props();
                    check(
                        emit,
                        family_of(props.and_then(|p| p.font.as_ref())),
                        body.node,
                    );
                }
                check(
                    emit,
                    family_of(paragraph.end_props.as_ref().and_then(|p| p.font.as_ref())),
                    body.node,
                );
            }
        }
    }
}

fn text_metrics(refs: &[ContainerRef<'_>], emit: &mut Emit) {
    for container in refs {
        for body in rendered_text(container) {
            if let Some(Autofit::Shrink {
                font_scale,
                line_spacing_reduction,
                fitted,
                ..
            }) = &body.text.body.autofit
            {
                let missing: Vec<&str> = [
                    ("fontScale", font_scale.is_some()),
                    ("lineSpacingReduction", line_spacing_reduction.is_some()),
                    ("fitted", fitted.is_some()),
                ]
                .into_iter()
                .filter(|(_, present)| !present)
                .map(|(key, _)| key)
                .collect();
                if !missing.is_empty() {
                    emit.emit(
                        codes::IR0107,
                        site(container)
                            .node(body.node.clone())
                            .data("path", at(&body.path.join(["body", "autofit"])))
                            .data("missing", missing.join(", ")),
                    );
                }
            }
            for (p, paragraph) in body.text.paragraphs.iter().enumerate() {
                let missing = paragraph.props.missing_line_metric_keys();
                if !missing.is_empty() {
                    emit.emit(
                        codes::IR0110,
                        site(container)
                            .node(body.node.clone())
                            .data(
                                "path",
                                at(&body
                                    .path
                                    .join([PathSeg::from("paragraphs"), PathSeg::Index(p)])),
                            )
                            .data("missing", missing.join(", ")),
                    );
                }
            }
        }
    }
}

fn geometry_lowered(refs: &[ContainerRef<'_>], emit: &mut Emit) {
    let fill = |emit: &mut Emit,
                container: &ContainerRef<'_>,
                lowered: bool,
                path: IrPath,
                node: Option<&NodeId>| {
        if !lowered {
            emit.emit(
                codes::IR0113,
                site(container)
                    .node(node.cloned())
                    .data("what", "fill")
                    .data("path", at(&path)),
            );
        }
    };
    for container in refs {
        if let Some(background) = container.container.background() {
            fill(
                emit,
                container,
                background.is_lowered(),
                container.path.with("background"),
                None,
            );
        }
        for element in container_elements(container) {
            let node = element.element.id();
            match element.element {
                Element::Shape(shape) => {
                    let kind = shape.geometry.as_ref().map_or("missing", Geometry::kind);
                    if kind != "paths" {
                        emit.emit(
                            codes::IR0105,
                            site(container)
                                .node(node.clone())
                                .data("path", at(&element.path.with("geometry")))
                                .data("reason", kind),
                        );
                    }
                    if let Some(shape_fill) = &shape.fill {
                        fill(
                            emit,
                            container,
                            shape_fill.is_lowered(),
                            element.path.with("fill"),
                            Some(node),
                        );
                    }
                }
                Element::Table(table) => {
                    for (cell, r, c) in table_cells(&table.rows) {
                        if let Some(cell_fill) = &cell.fill {
                            let path = element.path.join([
                                PathSeg::from("rows"),
                                PathSeg::Index(r),
                                PathSeg::from("cells"),
                                PathSeg::Index(c),
                                PathSeg::from("fill"),
                            ]);
                            fill(emit, container, cell_fill.is_lowered(), path, Some(node));
                        }
                    }
                }
                Element::Picture(picture) => {
                    if let Some(clip) = &picture.clip
                        && !matches!(clip, Geometry::Paths(_))
                    {
                        emit.emit(
                            codes::IR0105,
                            site(container)
                                .node(node.clone())
                                .data("path", at(&element.path.with("clip")))
                                .data("reason", clip.kind()),
                        );
                    }
                }
                _ => {}
            }
        }
        for body in rendered_text(container) {
            if body.text.rotation.is_none() {
                emit.emit(
                    codes::IR0113,
                    site(container)
                        .node(body.node.clone())
                        .data("what", "text rotation")
                        .data("path", at(&body.path)),
                );
            }
        }
    }
}

fn media_planned(presentation: &Presentation, emit: &mut Emit) {
    for (sha256, entry) in &presentation.media {
        if entry.plan.is_none() {
            emit.emit(
                codes::IR0106,
                DiagnosticSite::new().data("sha256", sha256.to_hex()),
            );
        }
    }
}

fn a11y_done(refs: &[ContainerRef<'_>], emit: &mut Emit) {
    for container in refs {
        let Container::Slide(slide) = container.container else {
            continue;
        };
        let missing: Vec<&str> = [
            ("title", slide.title.is_some()),
            ("readingOrder", slide.reading_order.is_some()),
        ]
        .into_iter()
        .filter(|(_, present)| !present)
        .map(|(key, _)| key)
        .collect();
        if !missing.is_empty() {
            emit.emit(
                codes::IR0109,
                site(container).data("missing", missing.join(", ")),
            );
        }
    }
}

/// Runs one invariant's checker on a schema-valid presentation. Failed containers are exempt (their elements are
/// empty by construction); the graph rules hold for them too. Diagnostics name their container, so the manager
/// can fail that container only.
pub fn check_invariant(
    invariant: Invariant,
    presentation: &Presentation,
    options: &CheckOptions,
) -> Vec<Diagnostic> {
    let emitter = options.graph.emitter.as_deref().unwrap_or("verify");
    if let Some(kinds) = invariant.graph_kinds() {
        let graph_options = GraphOptions {
            emitter: Some(emitter.to_owned()),
            linkable_slides: options.graph.linkable_slides.clone(),
        };
        let graph = verify_graph(presentation, &graph_options);
        let graph = match kinds {
            Some(kinds) => reference_diagnostics(graph, kinds),
            None => graph,
        };
        return graph
            .into_iter()
            .filter(|d| options.in_scope(d.container.as_ref()))
            .collect();
    }
    let refs: Vec<ContainerRef<'_>> = containers_of(presentation)
        .into_iter()
        .filter(|c| c.container.failed().is_none() && options.in_scope(Some(c.container.id())))
        .collect();
    let mut emit = Emit::new(emitter);
    match invariant {
        Invariant::FramesCanonical => frames_canonical(&refs, &mut emit),
        Invariant::InheritanceResolved => inheritance_resolved(&refs, &mut emit),
        Invariant::ThemeResolved => theme_resolved(&refs, &mut emit),
        Invariant::TextCascaded => text_cascaded(&refs, &mut emit),
        Invariant::ListsLabelled => lists_labelled(&refs, &mut emit),
        Invariant::FontsBound => fonts_bound(presentation, &refs, &mut emit),
        Invariant::TextMetrics => text_metrics(&refs, &mut emit),
        Invariant::GeometryLowered => geometry_lowered(&refs, &mut emit),
        Invariant::MediaPlanned => media_planned(presentation, &mut emit),
        Invariant::A11yDone => a11y_done(&refs, &mut emit),
        Invariant::SourceValid | Invariant::PlaceholdersBound | Invariant::TimelineNormalized => {}
    }
    emit.out
}

pub fn check_invariants(
    invariants: &[Invariant],
    presentation: &Presentation,
    options: &CheckOptions,
) -> Vec<Diagnostic> {
    invariants
        .iter()
        .flat_map(|invariant| check_invariant(*invariant, presentation, options))
        .collect()
}

/// The graph rules plus the checkers of the other `invariants`: the checks the pass manager makes after a pass
/// (with every invariant provided so far), and the resolved verification at the end.
pub fn verify_invariants(
    presentation: &Presentation,
    invariants: &[Invariant],
    options: &CheckOptions,
) -> Vec<Diagnostic> {
    let all: Vec<Invariant> = std::iter::once(Invariant::SourceValid)
        .chain(
            invariants
                .iter()
                .copied()
                .filter(|invariant| !invariant.is_graph()),
        )
        .collect();
    check_invariants(&all, presentation, options)
}
