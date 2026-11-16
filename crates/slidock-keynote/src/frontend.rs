//! Keynote (`.key`) as an input. The frontend reads the package's object graph itself: the show's slide tree for
//! order, the template slides as layouts, every slide and template lowered in isolation so an unreadable object fails
//! only its container. Keynote's style chains resolve here, so the presentation carries explicit values; a slide's
//! placeholders bind to their template's by explicit target. The slides around a Magic Move key their elements.
//!
//! On the deck path ([`LowerOptions::native_motion`]) a shown slide carries the transition into it, the one Keynote
//! stores on the shown slide before it ([`Placement::Into`]), its builds Keynote's own text delivery, and the
//! frontend reports how the native motion data plays (KN1404–KN1409). The parity path keeps the stored placement and
//! the delivery read from the effect's name, and reports neither.

use std::collections::{BTreeMap, BTreeSet};

use indexmap::IndexMap;
use slidock_canon::utf16;
use slidock_core_api::frontend::{App, SlideFacts};
use slidock_core_api::limits::{CellBudget, charge_container, presentation_budget};
use slidock_core_api::lowering::{
    FailureCodes, IdScope, LowerError, isolate_container, sorted_features,
};
use slidock_core_api::{
    Analysis, Frontend, FrontendCtx, FrontendError, Input, LowerOptions, LowerResult, Provenance,
    SniffHints,
};
use slidock_io::{RangeReader, sha256_range};
use slidock_ir::diagnostics::{DataValue, DiagnosticSite, codes};
use slidock_ir::features::FeatureId;
use slidock_ir::fonts::{FaceRequest, FaceStyle, FontEntry, FontRequest, font_key};
use slidock_ir::formats::{FileFormat, SourceFormat};
use slidock_ir::ids::{NodeId, Sha256, SlideId};
use slidock_ir::json_value::{Ext, JsonValue};
use slidock_ir::origin::Origin;
use slidock_ir::presentation::{Defaults, Phase, SlideSize};
use slidock_ir::source::{Layout, Presentation, Slide};
use slidock_ir::text::{DefaultTextStyles, LevelDefaults};
use slidock_ir::units::Size;

use crate::animation::{Placement, is_magic_move, shared_keys, transition_effect};
use crate::context::{Layer, Lowering, Setup};
use crate::document::{KeynoteDocument, open_document, report_index};
use crate::drawables::DrawableContext;
use crate::fonts::font_face;
use crate::geometry::SlideMap;
use crate::graph::ObjectGraph;
use crate::ids::{id_scope, kn_id};
use crate::iwa::ObjectId;
use crate::lower::{
    SlideNode, SlideObject, SlideOrder, Template, container_slots, lower_layout, lower_slide,
    placeholder_targets, slide_order,
};
use crate::media::{MediaRegistry, media_bytes};
use crate::num::{finite_or, js_min, pt, wide};
use crate::package::{DOCUMENT_PATH, KeynotePackage, package_root};
use crate::proto::kn;
use crate::text::{keynote_body, keynote_para, keynote_run};

const FORMAT: SourceFormat = SourceFormat::File(FileFormat::Keynote);
const BUILD_HISTORY: &str = "Metadata/BuildVersionHistory.plist";
const MAX_PLIST: u64 = 64 * 1024;
/// Keynote's default slide size, when a show states none.
const DEFAULT_SIZE: (f64, f64) = (1920.0, 1080.0);
/// The writer versions kept, the latest last.
const MAX_VERSIONS: usize = 32;
const MAX_VERSION_UNITS: usize = 128;

/// A failed container's codes: a limit, or anything else (a coded error names its own).
fn failure_codes() -> FailureCodes {
    FailureCodes::new(slidock_ir::diagnostics::codes::KN1011)
        .limit(slidock_ir::diagnostics::codes::KN1901)
}

fn presentation_size(show: &kn::ShowArchive) -> SlideSize {
    let side = |value: Option<f32>, fallback: f64| {
        let value = finite_or(wide(value), 0.0);
        let side = if value > 0.0 {
            js_min(100_000.0, value)
        } else {
            fallback
        };
        Size::from_steps(pt(side).steps())
    };
    let size = show.size.as_ref();
    SlideSize {
        w: side(size.and_then(|s| s.width), DEFAULT_SIZE.0),
        h: side(size.and_then(|s| s.height), DEFAULT_SIZE.1),
    }
}

fn default_text() -> DefaultTextStyles {
    let level = || LevelDefaults {
        para: keynote_para(),
        run: keynote_run(),
    };
    DefaultTextStyles {
        l0: level(),
        l1: level(),
        l2: level(),
        l3: level(),
        l4: level(),
        l5: level(),
        l6: level(),
        l7: level(),
        l8: level(),
    }
}

/// The template slides, in the theme's order, then any a slide uses that the theme does not list.
fn templates_of<'g>(
    graph: &'g ObjectGraph,
    show: &kn::ShowArchive,
    slides: &[SlideNode<'g>],
) -> Vec<Template<'g>> {
    let mut refs = Vec::new();
    if let Some(theme) = graph.get::<kn::ThemeArchive>(show.theme.as_ref()) {
        for reference in &theme.templates {
            if let Some(slide) = graph
                .get::<kn::SlideNodeArchive>(Some(reference))
                .and_then(|node| node.slide.as_ref())
            {
                refs.push(slide);
            }
        }
    }
    for SlideNode { node, .. } in slides {
        if let Some(template) = graph
            .get::<kn::SlideArchive>(node.slide.as_ref())
            .and_then(|slide| slide.template_slide.as_ref())
        {
            refs.push(template);
        }
    }
    let mut out: IndexMap<ObjectId, Template<'g>> = IndexMap::new();
    for reference in refs {
        let Some(object) = graph.resolve(Some(reference)) else {
            continue;
        };
        if let Some(message) = object.get::<kn::SlideArchive>()
            && !out.contains_key(&object.id)
        {
            out.insert(
                object.id,
                Template {
                    id: object.id,
                    message,
                    file: object.file,
                },
            );
        }
    }
    out.into_values().collect()
}

/// `<string>…</string>` contents of 1 to 128 UTF-16 code units, in order.
fn plist_strings(text: &str) -> Vec<String> {
    const OPEN: &str = "<string>";
    const CLOSE: &str = "</string>";
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(found) = text.get(at..).and_then(|rest| rest.find(OPEN)) {
        let start = at + found + OPEN.len();
        let rest = text.get(start..).unwrap_or_default();
        let run = rest.find('<').unwrap_or(rest.len());
        let content = rest.get(..run).unwrap_or_default();
        let units = utf16::len(content);
        if (1..=MAX_VERSION_UNITS).contains(&units)
            && rest.get(run..).is_some_and(|r| r.starts_with(CLOSE))
        {
            out.push(content.to_owned());
            at = start + run + CLOSE.len();
        } else {
            at = at + found + 1;
        }
    }
    out
}

/// The writer of the package, from the build history Keynote appends to on every save (`M14.5-7045.0.17-4`).
fn writer_of<R: RangeReader>(pkg: &KeynotePackage<R>) -> (Vec<String>, Option<App>) {
    let Some(entry) = pkg.entry(BUILD_HISTORY) else {
        return (Vec::new(), None);
    };
    if entry.size > MAX_PLIST {
        return (Vec::new(), None);
    }
    let Some(Ok(bytes)) = pkg.read(BUILD_HISTORY) else {
        return (Vec::new(), None);
    };
    // As TextDecoder reads it: a byte order mark dropped, invalid sequences replaced.
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    let text = String::from_utf8_lossy(bytes);
    let mut versions = plist_strings(&text);
    if versions.len() > MAX_VERSIONS {
        versions.drain(..versions.len() - MAX_VERSIONS);
    }
    let version = versions.last().and_then(|last| app_version(last));
    (
        versions,
        Some(App {
            name: "Keynote".to_owned(),
            version,
        }),
    )
}

/// `/^M(\d+(?:\.\d+)*)-/`
fn app_version(build: &str) -> Option<String> {
    let rest = build.strip_prefix('M')?;
    let end = rest.find('-')?;
    let version = rest.get(..end)?;
    let valid = !version.is_empty()
        && version
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()));
    valid.then(|| version.to_owned())
}

/// Font requests for the PostScript names the text uses, keyed like bind-fonts keys families.
fn font_entries(names: &BTreeSet<String>) -> IndexMap<String, FontEntry> {
    let mut sorted: Vec<&String> = names.iter().collect();
    sorted.sort_by(|a, b| utf16::cmp(a, b));
    let mut fonts: IndexMap<String, FontRequest> = IndexMap::new();
    for name in sorted {
        let face = font_face(name);
        let request = fonts
            .entry(font_key(&face.family))
            .or_insert_with(|| FontRequest {
                family: face.family.clone(),
                faces: Vec::new(),
                postscript_names: Vec::new(),
                embedded: false,
            });
        let style = if face.italic {
            FaceStyle::Italic
        } else {
            FaceStyle::Normal
        };
        if !request
            .faces
            .iter()
            .any(|f| f.weight == face.weight && f.style == style)
        {
            request.faces.push(FaceRequest {
                weight: face.weight,
                style,
            });
        }
        if !request.postscript_names.contains(name) {
            request.postscript_names.push(name.clone());
        }
    }
    fonts
        .into_iter()
        .map(|(key, mut request)| {
            request.faces.sort_by(|a, b| {
                a.weight
                    .cmp(&b.weight)
                    .then_with(|| style_name(a.style).cmp(style_name(b.style)))
            });
            (
                key,
                FontEntry {
                    request,
                    binding: None,
                },
            )
        })
        .collect()
}

fn style_name(style: FaceStyle) -> &'static str {
    match style {
        FaceStyle::Normal => "normal",
        FaceStyle::Italic => "italic",
    }
}

/// The PostScript names a container's printed text draws, in `ext.keynote.fonts`: what an export of the slides that
/// show it embeds, which the bench's oracle check verifies.
fn drawn_fonts(drawn: &BTreeSet<String>) -> Option<Ext> {
    if drawn.is_empty() {
        return None;
    }
    let mut names: Vec<&String> = drawn.iter().collect();
    names.sort_by(|a, b| utf16::cmp(a, b));
    let mut keynote = IndexMap::new();
    keynote.insert(
        "fonts".to_owned(),
        JsonValue::Array(
            names
                .into_iter()
                .map(|name| JsonValue::string(name.clone()))
                .collect(),
        ),
    );
    Some(Ext {
        keynote: Some(keynote),
        ..Ext::default()
    })
}

fn crashed(error: impl std::fmt::Display) -> FrontendError {
    FrontendError::Crashed(error.to_string())
}

fn node_id(object: ObjectId) -> Result<NodeId, FrontendError> {
    kn_id(object).map_err(crashed)
}

fn source_origin(part: &str, object: ObjectId) -> Origin {
    Origin::Source {
        part: part.to_owned(),
        path: None,
        object: Some(object.to_string()),
    }
}

/// What every container's lowering shares.
struct Shared<'a> {
    graph: &'a ObjectGraph,
    media: MediaRegistry<'a>,
    ids: IdScope,
    /// Every PostScript name the lowered text asks for.
    fonts: BTreeSet<String>,
    size: SlideSize,
    /// The slots the presentation's containers have left, charged in show order.
    cells: CellBudget,
    native_motion: bool,
}

/// What a container's lowering kept besides the container: its features and the fonts its text draws.
#[derive(Default)]
struct Kept {
    seen: Vec<FeatureId>,
    drawn: BTreeSet<String>,
}

/// Which container lowers, and what it binds and numbers.
struct Place<'p> {
    container: &'p NodeId,
    layer: Layer,
    slide_number: usize,
    targets: &'p BTreeMap<i32, NodeId>,
}

/// Lowers one container in isolation over the shared state.
fn isolated<C: slidock_core_api::Failable>(
    shared: &mut Shared<'_>,
    ctx: &mut FrontendCtx<'_>,
    skeleton: &C,
    place: Place<'_>,
    lower: impl FnOnce(&mut DrawableContext<'_>) -> Result<C, LowerError>,
) -> Result<(C, Option<Kept>), FrontendError> {
    let Place {
        container,
        layer,
        slide_number,
        targets,
    } = place;
    let mut kept = None;
    let cells = shared.cells;
    let native_motion = shared.native_motion;
    let Shared {
        graph,
        media,
        ids,
        fonts,
        size,
        ..
    } = shared;
    let result = isolate_container(skeleton, ctx, &failure_codes(), |ctx| {
        let setup = Setup {
            graph,
            container: container.clone(),
            layer,
            limits: ctx.limits,
            media,
            slide_width: size.w.to_f64(),
            slide_height: size.h.to_f64(),
            slide_number,
            native_motion,
        };
        let mut dctx = DrawableContext {
            low: Lowering::new(setup, &cells, &mut *ctx.diag),
            ids,
            placed: BTreeSet::new(),
            requested: fonts,
            drawn: BTreeSet::new(),
            targets,
            to_slide: SlideMap::SLIDE,
        };
        let container = lower(&mut dctx)?;
        kept = Some(Kept {
            seen: sorted_features(dctx.low.reporter.seen.iter().copied()),
            drawn: std::mem::take(&mut dctx.drawn),
        });
        Ok(container)
    })?;
    Ok((result, kept))
}

/// The Keynote frontend.
#[derive(Clone, Copy, Debug, Default)]
pub struct KeynoteFrontend;

fn slide_tree<'g>(doc: &'g KeynoteDocument<'_>, ctx: &FrontendCtx<'_>) -> SlideOrder<'g> {
    let roots = doc
        .show
        .slide_tree
        .as_ref()
        .map(|tree| tree.slides.as_slice())
        .unwrap_or_default();
    slide_order(&doc.graph, roots, ctx.limits.slides)
}

/// Each shown slide's shown predecessor, and the last shown slide, by position in show order.
struct ShownNeighbours {
    previous: Vec<Option<usize>>,
    last: Option<usize>,
}

impl ShownNeighbours {
    fn of(order: &[SlideNode<'_>]) -> ShownNeighbours {
        let mut previous = Vec::with_capacity(order.len());
        let mut last = None;
        for (position, node) in order.iter().enumerate() {
            previous.push(last);
            if node.node.is_skipped != Some(true) {
                last = Some(position);
            }
        }
        ShownNeighbours { previous, last }
    }

    /// The shown slide before `position`.
    fn previous(&self, position: usize) -> Option<usize> {
        self.previous.get(position).copied().flatten()
    }
}

/// The Magic Move pairs in show order, by position: a shown slide that leaves by Magic Move and the next shown
/// slide.
fn magic_move_pairs(graph: &ObjectGraph, order: &[SlideNode<'_>]) -> Vec<(usize, usize)> {
    let shown = |node: &SlideNode<'_>| node.node.is_skipped != Some(true);
    let mut pairs = Vec::new();
    for (index, node) in order.iter().enumerate() {
        let leaves_by_magic_move = graph
            .get::<kn::SlideArchive>(node.node.slide.as_ref())
            .and_then(transition_effect)
            .is_some_and(is_magic_move);
        if !shown(node) || !leaves_by_magic_move {
            continue;
        }
        let next = order
            .iter()
            .enumerate()
            .skip(index + 1)
            .find(|(_, next)| shown(next));
        if let Some((next, _)) = next {
            pairs.push((index, next));
        }
    }
    pairs
}

impl Frontend for KeynoteFrontend {
    fn format(&self) -> SourceFormat {
        FORMAT
    }

    fn version(&self) -> &str {
        "1"
    }

    fn sniff(&self, _head: &[u8], hints: &SniffHints<'_>) -> bool {
        sniff(hints)
    }

    fn analyze(
        &self,
        input: &Input<'_>,
        ctx: &mut FrontendCtx<'_>,
    ) -> Result<Analysis, FrontendError> {
        analyze(input, ctx)
    }

    fn lower(
        &self,
        input: &Input<'_>,
        ctx: &mut FrontendCtx<'_>,
        options: &LowerOptions<'_>,
    ) -> Result<LowerResult, FrontendError> {
        lower(input, ctx, options)
    }
}

fn sniff(hints: &SniffHints<'_>) -> bool {
    hints.zip_names.is_some_and(|names| {
        package_root(names.iter().map(String::as_str), DOCUMENT_PATH).is_some()
    })
}

fn analyze(input: &Input<'_>, ctx: &mut FrontendCtx<'_>) -> Result<Analysis, FrontendError> {
    let doc = open_document(input, ctx.limits)?;
    let order = slide_tree(&doc, ctx).slides;
    let slides = order
        .iter()
        .map(|slide| {
            Ok(SlideFacts {
                id: node_id(slide.id)?,
                hidden: slide.node.is_skipped == Some(true),
                title: None,
            })
        })
        .collect::<Result<Vec<_>, FrontendError>>()?;
    let features = if slides.iter().any(|slide| slide.hidden) {
        vec![FeatureId::PresentationHiddenSlide]
    } else {
        Vec::new()
    };
    Ok(Analysis {
        format: FORMAT,
        size: presentation_size(&doc.show),
        slides,
        features,
        media_bytes: media_bytes(&doc.pkg, &doc.graph),
        refusal: None,
    })
}

fn lower(
    input: &Input<'_>,
    ctx: &mut FrontendCtx<'_>,
    options: &LowerOptions<'_>,
) -> Result<LowerResult, FrontendError> {
    let native_motion = options.native_motion;
    let doc = open_document(input, ctx.limits)?;
    report_index(&doc.graph, ctx.diag);
    let graph = &doc.graph;
    let SlideOrder {
        slides: order,
        relisted,
    } = slide_tree(&doc, ctx);
    for slide in relisted {
        ctx.diag.report(
            codes::KN1012,
            DiagnosticSite::new().data("id", slide.to_string()),
        );
    }
    let wanted: Option<BTreeSet<&SlideId>> = options.only.map(|only| only.iter().collect());
    let lowers = |id: &NodeId| wanted.as_ref().is_none_or(|wanted| wanted.contains(id));
    let ids: Vec<NodeId> = order
        .iter()
        .map(|slide| node_id(slide.id))
        .collect::<Result<_, _>>()?;
    let show_index: BTreeMap<&SlideId, usize> = options
        .facts
        .slides
        .iter()
        .enumerate()
        .map(|(index, slide)| (&slide.id, index + 1))
        .collect();
    let media_pkg = KeynotePackage::open(input.name, input.reader, ctx.limits.zip)?;
    let mut shared = Shared {
        graph,
        media: MediaRegistry::new(media_pkg, graph),
        ids: id_scope(),
        fonts: BTreeSet::new(),
        size: presentation_size(&doc.show),
        cells: presentation_budget(ctx.limits),
        native_motion,
    };
    let mut features: IndexMap<String, Vec<FeatureId>> = IndexMap::new();
    let group_depth = ctx.limits.group_depth;

    // Templates, then slides: each charges the presentation's cells its tables count, lowered or skipped.
    let templates = templates_of(graph, &doc.show, &order);
    let used_templates: BTreeSet<u64> = order
        .iter()
        .zip(&ids)
        .filter(|(_, id)| lowers(id))
        .filter_map(|(slide, _)| {
            graph
                .get::<kn::SlideArchive>(slide.node.slide.as_ref())
                .and_then(|slide| slide.template_slide.as_ref())
                .and_then(|template| template.identifier)
        })
        .collect();
    let no_targets = BTreeMap::new();
    let mut layouts: IndexMap<NodeId, Layout> = IndexMap::new();
    for template in &templates {
        let slots = container_slots(graph, Some(template.message), Layer::Layout, group_depth);
        if wanted.is_some() && !used_templates.contains(&template.id.0) {
            charge_container(&mut shared.cells, slots);
            continue;
        }
        let id = node_id(template.id)?;
        let skeleton = Layout {
            id: id.clone(),
            origin: source_origin(template.file, template.id),
            name: None,
            layout_type: None,
            master: None,
            show_master: true,
            color_map_override: None,
            ext: None,
            background: None,
            elements: Vec::new(),
            failed: None,
        };
        // A layout's slide-number fields show the number of the slide that shows them, as composition sets it.
        let (mut layout, kept) = isolated(
            &mut shared,
            ctx,
            &skeleton,
            Place {
                container: &id,
                layer: Layer::Layout,
                slide_number: 1,
                targets: &no_targets,
            },
            |dctx| lower_layout(dctx, template),
        )?;
        charge_container(&mut shared.cells, slots);
        if let Some(kept) = kept.filter(|_| layout.failed.is_none()) {
            features.insert(id.as_str().to_owned(), kept.seen);
            if let Some(ext) = drawn_fonts(&kept.drawn) {
                layout.ext = Some(ext);
            }
        }
        layouts.insert(id, layout);
    }

    let pairs = magic_move_pairs(graph, &order);
    let neighbours = ShownNeighbours::of(&order);
    let keyed: BTreeSet<usize> = pairs.iter().flat_map(|&(from, to)| [from, to]).collect();
    let mut slides: Vec<Slide> = Vec::new();
    // Each lowered slide's index in `slides`, by its position in show order.
    let mut lowered_at: BTreeMap<usize, usize> = BTreeMap::new();
    for (position, (slide_node, id)) in order.iter().zip(&ids).enumerate() {
        let object = graph.resolve(slide_node.node.slide.as_ref());
        let slide = object.and_then(|object| {
            object.get::<kn::SlideArchive>().map(|message| SlideObject {
                id: object.id,
                message,
                file: object.file,
            })
        });
        let slots = container_slots(graph, slide.map(|s| s.message), Layer::Slide, group_depth);
        if !lowers(id) {
            charge_container(&mut shared.cells, slots);
            continue;
        }
        let template = slide
            .and_then(|slide| slide.message.template_slide.as_ref())
            .and_then(|template| template.identifier)
            .map(ObjectId);
        let layout_id = match template {
            Some(template) => {
                let layout_id = node_id(template)?;
                layouts.contains_key(&layout_id).then_some(layout_id)
            }
            None => None,
        };
        let hidden = slide_node.node.is_skipped == Some(true);
        let part = slide
            .map(|slide| slide.file)
            .or_else(|| graph.file_of(slide_node.id))
            .unwrap_or(DOCUMENT_PATH);
        let skeleton = Slide {
            id: id.clone(),
            origin: source_origin(part, slide.map_or(slide_node.id, |slide| slide.id)),
            layout: layout_id.clone(),
            hidden,
            show_layout: true,
            color_map_override: None,
            notes: None,
            transition: None,
            timeline: Vec::new(),
            ext: None,
            background: None,
            elements: Vec::new(),
            title: None,
            reading_order: None,
            failed: None,
        };
        let targets = placeholder_targets(
            slide.and_then(|slide| {
                graph.get::<kn::SlideArchive>(slide.message.template_slide.as_ref())
            }),
            layout_id.as_ref().and_then(|layout| layouts.get(layout)),
        );
        let number = show_index.get(id).copied().unwrap_or(1);
        let (mut lowered, kept) = isolated(
            &mut shared,
            ctx,
            &skeleton,
            Place {
                container: id,
                layer: Layer::Slide,
                slide_number: number,
                targets: &targets,
            },
            |dctx| {
                let Some(slide) = slide else {
                    let mut data = slidock_ir::diagnostics::DiagnosticData::new();
                    data.insert("reason".to_owned(), DataValue::from("the slide is missing"));
                    return Err(LowerError::Coded {
                        code: slidock_ir::diagnostics::codes::KN1009,
                        data,
                    });
                };
                let placement = if native_motion {
                    Placement::Into {
                        previous: neighbours
                            .previous(position)
                            .and_then(|previous| order.get(previous))
                            .and_then(|previous| {
                                graph.get::<kn::SlideArchive>(previous.node.slide.as_ref())
                            }),
                        skipped: hidden,
                        last: neighbours.last == Some(position),
                    }
                } else {
                    Placement::Stored
                };
                lower_slide(
                    dctx,
                    &slide,
                    layout_id.as_ref(),
                    hidden,
                    keyed.contains(&position),
                    placement,
                )
            },
        )?;
        charge_container(&mut shared.cells, slots);
        if let Some(kept) = kept.filter(|_| lowered.failed.is_none()) {
            features.insert(id.as_str().to_owned(), kept.seen);
            if let Some(ext) = drawn_fonts(&kept.drawn) {
                lowered.ext = Some(ext);
            }
        }
        lowered_at.insert(position, slides.len());
        slides.push(lowered);
    }
    if native_motion {
        report_unmatched(ctx, &pairs, &lowered_at, &slides);
    }
    features.insert("presentation".to_owned(), Vec::new());

    let (versions, app) = writer_of(&doc.pkg);
    let stats = &graph.stats;
    // A payload with no type is type -1, which sorts first as `None` does.
    let unknown: Vec<JsonValue> = stats
        .unknown_type_ids
        .keys()
        .map(|id| JsonValue::number(id.map_or(-1.0, f64::from)))
        .collect();
    let count = |n: u64| JsonValue::number(n as f64);
    let objects: IndexMap<String, JsonValue> = [
        ("decoded", count(stats.decoded)),
        ("skipped", count(stats.skipped)),
        ("unknownTypeIds", JsonValue::Array(unknown)),
        ("decodeErrors", count(stats.decode_errors)),
        ("mergeRecords", count(stats.merge_records)),
        ("extraPayloads", count(stats.extra_payloads)),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value))
    .collect();
    let mut keynote = IndexMap::new();
    keynote.insert(
        "writer".to_owned(),
        JsonValue::Array(versions.into_iter().map(JsonValue::string).collect()),
    );
    keynote.insert("objects".to_owned(), JsonValue::Object(objects));

    let media_store = shared.media.store();
    let presentation = Presentation {
        schema: None,
        size: shared.size.clone(),
        defaults: Defaults {
            text: Box::new(default_text()),
            body: keynote_body(),
        },
        themes: IndexMap::new(),
        ext: Some(Ext {
            keynote: Some(keynote),
            ..Ext::default()
        }),
        phase: Phase::Source,
        masters: IndexMap::new(),
        layouts,
        slides,
        fonts: font_entries(&shared.fonts),
        media: std::mem::take(&mut shared.media.entries),
    };
    let sha256 = sha256_range(input.reader, 0, input.size()).map_err(crashed)?;
    let provenance = Provenance {
        format: FORMAT,
        name: input.name.to_owned(),
        sha256: Sha256::parse(&sha256).map_err(crashed)?,
        bytes: input.size(),
        app,
    };
    Ok(LowerResult {
        provenance,
        presentation: presentation.into(),
        media: Box::new(media_store),
        fonts: Vec::new(),
        features,
    })
}

/// KN1407 on each Magic Move whose two slides lowered and share no key, on the slide it plays into: every object
/// fades.
fn report_unmatched(
    ctx: &mut FrontendCtx<'_>,
    pairs: &[(usize, usize)],
    lowered_at: &BTreeMap<usize, usize>,
    slides: &[Slide],
) {
    for (from, to) in pairs {
        let lowered = |position: &usize| {
            lowered_at
                .get(position)
                .and_then(|index| slides.get(*index))
                .filter(|slide| slide.failed.is_none())
        };
        let (Some(from), Some(to)) = (lowered(from), lowered(to)) else {
            continue;
        };
        if shared_keys(&from.elements, &to.elements) == 0 {
            ctx.diag.report(
                codes::KN1407,
                DiagnosticSite::new()
                    .container(to.id.clone())
                    .data("slide", to.id.as_str().to_owned()),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_build_strings_as_the_writer_history_regex_does() {
        let plist = "<array><string>M14.5-7045.0.17-4</string><string></string>\
                     <string>a<b</string><string>M15.4-1-2</string></array>";
        assert_eq!(plist_strings(plist), vec!["M14.5-7045.0.17-4", "M15.4-1-2"]);
        let long = format!("<string>{}</string><string>ok</string>", "x".repeat(129));
        assert_eq!(plist_strings(&long), vec!["ok"]);
        assert_eq!(app_version("M14.5-7045.0.17-4").as_deref(), Some("14.5"));
        assert_eq!(app_version("M14.-1"), None);
        assert_eq!(app_version("14.5-1"), None);
    }

    /// A slide's failure under the frontend's codes.
    fn failed_with(error: LowerError) -> (Option<String>, Vec<String>) {
        let limits = slidock_core_api::Limits::default();
        let mut diagnostics = Vec::new();
        let mut sink = slidock_core_api::Sink::new(
            &mut diagnostics,
            slidock_ir::diagnostics::Stage::Frontend,
            "keynote",
        );
        let mut ctx = FrontendCtx {
            limits: &limits,
            diag: &mut sink,
            deadline: slidock_core_api::Deadline::never(),
            spent: slidock_core_api::Output::default(),
        };
        let skeleton = Slide {
            id: NodeId::new("kn-1").unwrap_or_else(|_| unreachable!()),
            origin: Origin::source(DOCUMENT_PATH),
            layout: None,
            hidden: false,
            show_layout: true,
            color_map_override: None,
            notes: None,
            transition: None,
            timeline: Vec::new(),
            ext: None,
            background: None,
            elements: Vec::new(),
            title: None,
            reading_order: None,
            failed: None,
        };
        let slide = isolate_container(&skeleton, &mut ctx, &failure_codes(), |_| Err(error));
        let code = slide
            .ok()
            .and_then(|slide| slide.failed)
            .map(|failed| failed.code.as_str().to_owned());
        (
            code,
            diagnostics
                .iter()
                .map(|d| d.code.code().to_owned())
                .collect(),
        )
    }

    #[test]
    fn fails_a_container_that_crashed_with_kn1011_and_one_past_a_limit_with_kn1901() {
        let crashed = failed_with(LowerError::Other("an unexpected crash".to_owned()));
        assert_eq!(
            crashed,
            (Some("KN1011".to_owned()), vec!["KN1011".to_owned()])
        );
        let limited = failed_with(LowerError::Limit("Groups nest deeper than 16".to_owned()));
        assert_eq!(
            limited,
            (Some("KN1901".to_owned()), vec!["KN1901".to_owned()])
        );
    }
}
