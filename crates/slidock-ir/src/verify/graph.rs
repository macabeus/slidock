//! The graph rules of both phases, on a schema-valid presentation: the maps' keys, unique ids, references that resolve,
//! acyclic inheritance, nesting depths, size limits and table grids. They run in that order, which fixes the order of
//! their diagnostics.

use std::collections::BTreeSet;

use indexmap::IndexMap;

use crate::diagnostics::{
    Diagnostic, DiagnosticCode, DiagnosticSite, Stage, codes, make_diagnostic,
};
use crate::fonts::font_key;
use crate::ids::{NodeId, Sha256};
use crate::path::{IrPath, PathSeg};
use crate::presentation::limits;
use crate::source::{Container, Element, Presentation, Table};
use crate::table::{grid_size, table_cells};
use crate::text::Link;
use crate::verify::visit::{ObjectVisitor, Walk, slide_target};
use crate::verify::walk::{ContainerRef, container_elements, containers_of, rendered_text};

/// Options of the graph rules.
#[derive(Clone, Debug, Default)]
pub struct GraphOptions {
    /// Who reports (`verify` when absent).
    pub emitter: Option<String>,
    /// Slides a slide link may target besides the presentation's own: the whole show order when only some slides
    /// were lowered, so a link to a slide left out still resolves.
    pub linkable_slides: Vec<NodeId>,
}

pub(crate) struct Emit {
    emitter: String,
    pub(crate) out: Vec<Diagnostic>,
}

impl Emit {
    pub(crate) fn new(emitter: &str) -> Self {
        Emit {
            emitter: emitter.to_owned(),
            out: Vec::new(),
        }
    }

    pub(crate) fn emit(&mut self, code: DiagnosticCode, site: DiagnosticSite) {
        self.out
            .push(make_diagnostic(code, Stage::Verify, &self.emitter, site));
    }
}

/// The UTF-8 length of a value's compact canonical JSON.
fn canonical_bytes<T: serde::Serialize + ?Sized>(value: &T) -> usize {
    crate::canonical::to_json(value).map_or(usize::MAX, |json| json.len())
}

struct Indexed<'a> {
    element: &'a Element,
    container: &'a NodeId,
}

/// Every element by id (its first occurrence), with the container holding it.
fn index_elements<'a>(refs: &[ContainerRef<'a>]) -> IndexMap<&'a NodeId, Indexed<'a>> {
    let mut elements = IndexMap::new();
    for container in refs {
        for element in container_elements(container) {
            elements.entry(element.element.id()).or_insert(Indexed {
                element: element.element,
                container: container.container.id(),
            });
        }
    }
    elements
}

fn site_path(path: &IrPath) -> String {
    path.to_string()
}

fn check_maps(presentation: &Presentation, emit: &mut Emit) {
    let keyed = [
        (
            "themes",
            presentation
                .themes
                .iter()
                .map(|(k, t)| (k, &t.id))
                .collect::<Vec<_>>(),
        ),
        (
            "masters",
            presentation
                .masters
                .iter()
                .map(|(k, m)| (k, &m.id))
                .collect(),
        ),
        (
            "layouts",
            presentation
                .layouts
                .iter()
                .map(|(k, l)| (k, &l.id))
                .collect(),
        ),
    ];
    for (field, entries) in keyed {
        for (key, id) in entries {
            if id != key {
                let container = (field != "themes").then(|| key.clone());
                let path = IrPath::root().join([
                    PathSeg::from(field),
                    PathSeg::from(key.to_string()),
                    "id".into(),
                ]);
                emit.emit(
                    codes::IR0001,
                    DiagnosticSite::new()
                        .container(container)
                        .data("path", site_path(&path))
                        .data("reason", "The key differs from the id"),
                );
            }
        }
    }
    for (key, entry) in &presentation.media {
        if &entry.sha256 != key {
            let path = IrPath::root().join([
                PathSeg::from("media"),
                PathSeg::from(key.to_hex()),
                "sha256".into(),
            ]);
            emit.emit(
                codes::IR0001,
                DiagnosticSite::new()
                    .data("path", site_path(&path))
                    .data("reason", "The key differs from the hash"),
            );
        }
    }
    for (key, entry) in &presentation.fonts {
        if &font_key(&entry.request.family) != key {
            let path = IrPath::root().join([PathSeg::from("fonts"), PathSeg::from(key.clone())]);
            emit.emit(
                codes::IR0001,
                DiagnosticSite::new()
                    .data("path", site_path(&path))
                    .data("reason", "The key is not the family\u{2019}s font key"),
            );
        }
    }
}

fn note<'a>(seen: &mut IndexMap<&'a str, (usize, Vec<&'a str>)>, id: &'a str, owner: &'a str) {
    let entry = seen.entry(id).or_insert((0, Vec::new()));
    entry.0 += 1;
    if !entry.1.contains(&owner) {
        entry.1.push(owner);
    }
}

fn check_ids(presentation: &Presentation, refs: &[ContainerRef<'_>], emit: &mut Emit) {
    const PRESENTATION: &str = "presentation";
    let mut seen: IndexMap<&str, (usize, Vec<&str>)> = IndexMap::new();
    for theme in presentation.themes.values() {
        note(&mut seen, theme.id.as_str(), PRESENTATION);
    }
    for container in refs {
        let id = container.container.id().as_str();
        note(&mut seen, id, PRESENTATION);
        for element in container_elements(container) {
            note(&mut seen, element.element.id().as_str(), id);
        }
    }
    for (id, (count, owners)) in &seen {
        if *count < 2 {
            continue;
        }
        // Within one container the container fails; across containers the presentation does.
        let container = match owners.as_slice() {
            [only] if *only != PRESENTATION => NodeId::new(*only).ok(),
            _ => None,
        };
        emit.emit(
            codes::IR0002,
            DiagnosticSite::new().container(container).data("id", *id),
        );
    }
}

struct References<'a, 'e> {
    presentation: &'a Presentation,
    slide_ids: &'a BTreeSet<&'a str>,
    container: &'a NodeId,
    emit: &'e mut Emit,
}

fn dangling(
    emit: &mut Emit,
    container: &NodeId,
    kind: &str,
    target: &str,
    path: &IrPath,
    node: Option<&NodeId>,
) {
    emit.emit(
        codes::IR0003,
        DiagnosticSite::new()
            .container(container.clone())
            .node(node.cloned())
            .data("kind", kind)
            .data("target", target)
            .data("path", site_path(path)),
    );
}

impl ObjectVisitor for References<'_, '_> {
    fn media(&mut self, path: &IrPath, field: &'static str, sha: &Sha256) {
        if !self.presentation.media.contains_key(sha) {
            dangling(
                self.emit,
                self.container,
                "media",
                &sha.to_hex(),
                &path.with(field),
                None,
            );
        }
    }

    fn link(&mut self, path: &IrPath, link: &Link) {
        if let Some(slide) = slide_target(link)
            && !self.slide_ids.contains(slide.as_str())
        {
            dangling(
                self.emit,
                self.container,
                "slide-link",
                slide.as_str(),
                path,
                None,
            );
        }
    }
}

fn check_references(
    presentation: &Presentation,
    refs: &[ContainerRef<'_>],
    elements: &IndexMap<&NodeId, Indexed<'_>>,
    emit: &mut Emit,
    linkable_slides: &[NodeId],
) {
    let slide_ids: BTreeSet<&str> = presentation
        .slides
        .iter()
        .map(|slide| slide.id.as_str())
        .chain(linkable_slides.iter().map(NodeId::as_str))
        .collect();
    for container in refs {
        let id = container.container.id();
        let path = &container.path;
        match container.container {
            Container::Slide(slide) => {
                if let Some(layout) = &slide.layout
                    && !presentation.layouts.contains_key(layout)
                {
                    dangling(
                        emit,
                        id,
                        "layout",
                        layout.as_str(),
                        &path.with("layout"),
                        None,
                    );
                }
            }
            Container::Layout(layout) => {
                if let Some(master) = &layout.master
                    && !presentation.masters.contains_key(master)
                {
                    dangling(
                        emit,
                        id,
                        "master",
                        master.as_str(),
                        &path.with("master"),
                        None,
                    );
                }
            }
            Container::Master(master) => {
                if let Some(theme) = &master.theme
                    && !presentation.themes.contains_key(theme)
                {
                    dangling(emit, id, "theme", theme.as_str(), &path.with("theme"), None);
                }
            }
        }

        let mut local: BTreeSet<&str> = BTreeSet::new();
        for element in container_elements(container) {
            let node = element.element.id();
            local.insert(node.as_str());
            if let Some(inherits) = element.element.inherits()
                && !elements.contains_key(inherits)
            {
                dangling(
                    emit,
                    id,
                    "inherits",
                    inherits.as_str(),
                    &element.path.with("inherits"),
                    Some(node),
                );
            }
            if let Some(target) = element
                .element
                .placeholder()
                .and_then(|p| p.target.as_ref())
                && !elements.contains_key(target)
            {
                let at = element.path.join(["placeholder", "target"]);
                dangling(
                    emit,
                    id,
                    "placeholder-target",
                    target.as_str(),
                    &at,
                    Some(node),
                );
            }
        }
        if let Container::Slide(slide) = container.container {
            for (index, step) in slide.timeline.iter().enumerate() {
                if !local.contains(step.target.element.as_str()) {
                    let at = path.join([
                        PathSeg::from("timeline"),
                        PathSeg::Index(index),
                        "target".into(),
                    ]);
                    dangling(
                        emit,
                        id,
                        "timeline-target",
                        step.target.element.as_str(),
                        &at,
                        None,
                    );
                }
            }
        }

        let mut visitor = References {
            presentation,
            slide_ids: &slide_ids,
            container: id,
            emit,
        };
        Walk::container(&mut visitor, container.container, path, true);
    }
}

fn check_inheritance(elements: &IndexMap<&NodeId, Indexed<'_>>, emit: &mut Emit) {
    let mut reported: BTreeSet<Vec<&str>> = BTreeSet::new();
    for (id, indexed) in elements {
        let mut chain: Vec<&NodeId> = vec![id];
        let mut next = indexed.element.inherits();
        while let Some(target) = next {
            let Some(found) = elements.get(target) else {
                break;
            };
            if let Some(start) = chain.iter().position(|member| *member == target) {
                let mut cycle: Vec<&str> = chain
                    .iter()
                    .skip(start)
                    .map(|member| member.as_str())
                    .collect();
                cycle.sort_unstable();
                if reported.insert(cycle) {
                    emit.emit(
                        codes::IR0004,
                        DiagnosticSite::new()
                            .container(indexed.container.clone())
                            .node((*id).clone())
                            .data("id", target.as_str()),
                    );
                }
                break;
            }
            chain.push(target);
            if chain.len() - 1 > limits::INHERITS_CHAIN {
                emit.emit(
                    codes::IR0005,
                    DiagnosticSite::new()
                        .container(indexed.container.clone())
                        .node((*id).clone())
                        .data("kind", "inherits")
                        .data("limit", limits::INHERITS_CHAIN),
                );
                break;
            }
            next = found.element.inherits();
        }
    }
}

fn check_bounds(presentation: &Presentation, refs: &[ContainerRef<'_>], emit: &mut Emit) {
    if presentation.slides.len() > limits::SLIDES {
        emit.emit(
            codes::IR0006,
            DiagnosticSite::new()
                .data("count", presentation.slides.len())
                .data("limit", limits::SLIDES),
        );
    }

    let mut presentation_ext = 0usize;
    let mut ext = |emit: &mut Emit,
                   value: Option<&crate::json_value::Ext>,
                   path: IrPath,
                   container: Option<&NodeId>,
                   node: Option<&NodeId>| {
        let Some(value) = value else { return };
        let bytes = canonical_bytes(value);
        presentation_ext = presentation_ext.saturating_add(bytes);
        if bytes > limits::EXT_PER_NODE {
            emit.emit(
                codes::IR0008,
                DiagnosticSite::new()
                    .container(container.cloned())
                    .node(node.cloned())
                    .data("what", "ext")
                    .data("path", site_path(&path))
                    .data("limit", limits::EXT_PER_NODE),
            );
        }
    };
    ext(
        emit,
        presentation.ext.as_ref(),
        IrPath::root().with("ext"),
        None,
        None,
    );
    for (key, theme) in &presentation.themes {
        let path = IrPath::root().join([
            PathSeg::from("themes"),
            PathSeg::from(key.to_string()),
            "ext".into(),
        ]);
        ext(emit, theme.ext.as_ref(), path, None, None);
    }

    for container in refs {
        let id = container.container.id();
        ext(
            emit,
            container.container.ext(),
            container.path.with("ext"),
            Some(id),
            None,
        );
        let mut count = 0usize;
        for element in container_elements(container) {
            count += 1;
            let node = element.element.id();
            ext(
                emit,
                element.element.ext(),
                element.path.with("ext"),
                Some(id),
                Some(node),
            );
            if element.depth == limits::GROUP_DEPTH + 1 {
                emit.emit(
                    codes::IR0005,
                    DiagnosticSite::new()
                        .container(id.clone())
                        .node(node.clone())
                        .data("kind", "group")
                        .data("limit", limits::GROUP_DEPTH),
                );
            }
            if let Element::Foreign(foreign) = element.element
                && let Some(payload) = &foreign.payload
                && canonical_bytes(payload) > limits::PAYLOAD
            {
                emit.emit(
                    codes::IR0008,
                    DiagnosticSite::new()
                        .container(id.clone())
                        .node(node.clone())
                        .data("what", "payload")
                        .data("path", site_path(&element.path.with("payload")))
                        .data("limit", limits::PAYLOAD),
                );
            }
        }
        if count > limits::ELEMENTS_PER_CONTAINER {
            emit.emit(
                codes::IR0008,
                DiagnosticSite::new()
                    .container(id.clone())
                    .data("what", "elements")
                    .data("path", site_path(&container.path))
                    .data("limit", limits::ELEMENTS_PER_CONTAINER),
            );
        }
        let mut bodies: Vec<(&crate::text::TextBody, IrPath, Option<&NodeId>)> =
            rendered_text(container)
                .into_iter()
                .map(|body| (body.text, body.path, Some(body.node)))
                .collect();
        if let Container::Slide(slide) = container.container
            && let Some(notes) = &slide.notes
        {
            bodies.push((notes, container.path.with("notes"), None));
        }
        for (text, path, node) in bodies {
            let chars: usize = text
                .paragraphs
                .iter()
                .flat_map(|paragraph| &paragraph.inlines)
                .map(|inline| match inline {
                    crate::text::Inline::Run { text, .. } => slidock_canon::utf16::len(text),
                    crate::text::Inline::Field { cached, .. } => slidock_canon::utf16::len(cached),
                    _ => 0,
                })
                .sum();
            if chars > limits::TEXT_PER_BODY {
                emit.emit(
                    codes::IR0008,
                    DiagnosticSite::new()
                        .container(id.clone())
                        .node(node.filter(|node| *node != id).cloned())
                        .data("what", "text")
                        .data("path", site_path(&path))
                        .data("limit", limits::TEXT_PER_BODY),
                );
            }
        }
    }
    if presentation_ext > limits::EXT_PER_PRESENTATION {
        emit.emit(
            codes::IR0008,
            DiagnosticSite::new()
                .data("what", "ext")
                .data("path", "(presentation)")
                .data("limit", limits::EXT_PER_PRESENTATION),
        );
    }
}

/// The first way a table's grid breaks density or covers a slot other than once, if any: where, and why.
fn grid_problem(table: &Table) -> Option<(Vec<PathSeg>, String)> {
    let columns = table.columns.len();
    for (row, cells) in table.rows.iter().map(|row| &row.cells).enumerate() {
        if cells.len() != columns {
            return Some((
                vec!["rows".into(), PathSeg::Index(row), "cells".into()],
                format!(
                    "row {row} holds {} slots for {columns} columns",
                    cells.len()
                ),
            ));
        }
    }
    let mut covers: Vec<Vec<u32>> = vec![vec![0; columns]; table.rows.len()];
    for (cell, row, column) in table_cells(&table.rows) {
        let (span_columns, span_rows) = cell.spans();
        if row + span_rows > table.rows.len() || column + span_columns > columns {
            return Some((
                vec![
                    "rows".into(),
                    PathSeg::Index(row),
                    "cells".into(),
                    PathSeg::Index(column),
                ],
                format!("the cell at row {row}, column {column} spans past the grid"),
            ));
        }
        for counts in covers.iter_mut().skip(row).take(span_rows) {
            for count in counts.iter_mut().skip(column).take(span_columns) {
                *count += 1;
            }
        }
    }
    for (row, counts) in covers.iter().enumerate() {
        if let Some((column, count)) = counts.iter().enumerate().find(|(_, count)| **count != 1) {
            let times = if *count == 0 {
                "by no cell".to_owned()
            } else {
                format!("{count} times")
            };
            return Some((
                vec![
                    "rows".into(),
                    PathSeg::Index(row),
                    "cells".into(),
                    PathSeg::Index(column),
                ],
                format!("the slot at row {row}, column {column} is covered {times}"),
            ));
        }
    }
    None
}

fn check_table(table: &Table, path: &IrPath, container: &NodeId, emit: &mut Emit) {
    if let Some((at, reason)) = grid_problem(table) {
        emit.emit(
            codes::IR0014,
            DiagnosticSite::new()
                .container(container.clone())
                .node(table.id.clone())
                .data("path", site_path(&path.join(at)))
                .data("reason", reason),
        );
    }
    let Some(frame) = &table.frame else { return };
    let (w, h) = grid_size(&table.columns, &table.rows);
    for (side, tracks, frame_side, sum) in [
        ("w", "columns", frame.w.to_f64(), w),
        ("h", "rows", frame.h.to_f64(), h),
    ] {
        if sum.to_f64() != frame_side {
            emit.emit(
                codes::IR0015,
                DiagnosticSite::new()
                    .container(container.clone())
                    .node(table.id.clone())
                    .data("path", site_path(&path.join(["frame", side])))
                    .data("frame", frame_side)
                    .data("tracks", tracks)
                    .data("sum", sum.to_f64()),
            );
        }
    }
}

/// A container, its tables with their paths, and their grid slots.
type ContainerTables<'r, 'a> = (&'r ContainerRef<'a>, Vec<(&'a Table, IrPath)>, usize);

fn check_tables(refs: &[ContainerRef<'_>], emit: &mut Emit) {
    let containers: Vec<ContainerTables<'_, '_>> = refs
        .iter()
        .map(|container| {
            let mut tables = Vec::new();
            let mut cells = 0usize;
            for element in container_elements(container) {
                if let Element::Table(table) = element.element {
                    cells =
                        cells.saturating_add(table.rows.len().saturating_mul(table.columns.len()));
                    tables.push((&**table, element.path));
                }
            }
            (container, tables, cells)
        })
        .collect();
    let total = containers
        .iter()
        .fold(0usize, |total, (_, _, cells)| total.saturating_add(*cells));
    if total > limits::CELLS_PER_PRESENTATION {
        emit.emit(
            codes::IR0008,
            DiagnosticSite::new()
                .data("what", "cells")
                .data("path", "(presentation)")
                .data("limit", limits::CELLS_PER_PRESENTATION),
        );
        return;
    }
    for (container, tables, cells) in containers {
        let id = container.container.id();
        if cells > limits::CELLS_PER_CONTAINER {
            emit.emit(
                codes::IR0008,
                DiagnosticSite::new()
                    .container(id.clone())
                    .data("what", "cells")
                    .data("path", site_path(&container.path))
                    .data("limit", limits::CELLS_PER_CONTAINER),
            );
            continue;
        }
        for (table, path) in tables {
            check_table(table, &path, id, emit);
        }
    }
}

/// The graph rules of both phases on a schema-valid presentation.
pub fn verify_graph(presentation: &Presentation, options: &GraphOptions) -> Vec<Diagnostic> {
    let mut emit = Emit::new(options.emitter.as_deref().unwrap_or("verify"));
    let refs = containers_of(presentation);
    let elements = index_elements(&refs);
    check_maps(presentation, &mut emit);
    check_ids(presentation, &refs, &mut emit);
    check_references(
        presentation,
        &refs,
        &elements,
        &mut emit,
        &options.linkable_slides,
    );
    check_inheritance(&elements, &mut emit);
    check_bounds(presentation, &refs, &mut emit);
    check_tables(&refs, &mut emit);
    emit.out
}

/// The IR0003 diagnostics of the given reference kinds.
pub(crate) fn reference_diagnostics(
    diagnostics: Vec<Diagnostic>,
    kinds: &[&str],
) -> Vec<Diagnostic> {
    diagnostics
        .into_iter()
        .filter(|d| {
            d.code == codes::IR0003
                && d.data
                    .as_ref()
                    .and_then(|data| data.get("kind"))
                    .is_some_and(|kind| kinds.contains(&kind.to_js_string().as_str()))
        })
        .collect()
}
