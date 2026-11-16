//! OpenDocument Presentation (`.odp`) as an input. The frontend reads the package itself: content.xml for the pages,
//! styles.xml for the common styles and master pages. ODF style inheritance resolves here into explicit values, since
//! it is internal to the format. Each master page and page lowers in isolation, so a part or object that cannot be read
//! fails only its container. Fonts the package embeds are handed to the host as they are. Transitions and build
//! animations are detected and reported on the parity path, or read into the effect vocabulary when
//! [`LowerOptions::native_motion`] is set.

mod context;
mod custom_shape_map;
mod defaults;
mod document;
mod element;
mod enhanced;
mod fonts;
mod geometry;
mod gradient;
mod graphic;
mod ids;
mod links;
mod lists;
mod master_pages;
mod media;
mod motion;
mod names;
mod odf;
mod package;
mod page;
mod picture;
mod shapes;
mod styles;
mod svg_path;
mod table;
mod text;
mod timing;
mod transform;
mod transition;
mod units;

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use indexmap::{IndexMap, IndexSet};
use slidock_core_api::frontend::SlideFacts;
use slidock_core_api::limits::{charge_container, container_budget, presentation_budget};
use slidock_core_api::lowering::{FailureCodes, isolate_container, sorted_features};
use slidock_core_api::{
    Analysis, Failable, Frontend, FrontendCtx, FrontendError, Input, LowerOptions, LowerResult,
    Provenance, SniffHints, Unverified,
};
use slidock_ir::diagnostics::{DiagnosticSite, codes};
use slidock_ir::features::FeatureId;
use slidock_ir::formats::{FileFormat, SourceFormat};
use slidock_ir::ids::{NodeId, Sha256};
use slidock_ir::origin::Origin;
use slidock_ir::presentation::{Defaults, Failed, Phase};
use slidock_ir::source::{Layout, Presentation, Slide};

use crate::context::{ContainerState, Lowering};
use crate::defaults::{default_body, default_text};
use crate::document::{CONTENT, OdpDocument, PageRef, Parts, STYLES, is_hidden, references};
use crate::ids::master_id;
use crate::master_pages::lower_master_page;
use crate::media::MediaRegistry;
use crate::motion::MotionMode;
use crate::names::qn;
use crate::package::{OdpPackage, Reference, resolve_reference};
use crate::page::{SlideSource, lower_slide, slide_slots};
use crate::styles::{CommonStyles, StyleScope};
use crate::table::table_slots;

const FORMAT: SourceFormat = SourceFormat::File(FileFormat::Odp);

/// The ODP frontend. With [`LowerOptions::native_motion`] a page's transition and build animations lower into the
/// IR's effect vocabulary; without it (the parity path) they are detected and reported (OD9402,
/// OD9401), and the IR holds neither.
#[derive(Clone, Copy, Debug, Default)]
pub struct OdpFrontend;

impl OdpFrontend {
    pub fn new() -> Self {
        OdpFrontend
    }
}

/// The bytes of every package file the document references, each file once.
fn media_bytes(pkg: &OdpPackage<'_>, parts: &Parts) -> u64 {
    let mut paths: BTreeSet<String> = BTreeSet::new();
    for href in references(Some(&parts.content))
        .into_iter()
        .chain(references(parts.styles()))
    {
        if let Reference::Internal(path) = resolve_reference(href) {
            paths.insert(path);
        }
    }
    paths
        .iter()
        .map(|path| pkg.entry(path).map_or(0, |entry| entry.size))
        .fold(0, u64::saturating_add)
}

/// How a container fails: a part that cannot be read, a limit, anything else.
fn failure(part: &'static str) -> FailureCodes {
    FailureCodes::new(codes::OD9005)
        .part(codes::OD9003)
        .limit(codes::OD9901)
        .data("part", part)
}

/// The master page a page names.
fn master_of<'a>(page: &PageRef<'a>) -> Option<&'a str> {
    page.element.attr(qn!("draw:master-page-name"))
}

fn layout_skeleton(id: NodeId, path: Option<&str>) -> Layout {
    Layout {
        id,
        origin: Origin::Source {
            part: STYLES.to_owned(),
            path: path.map(|path| slidock_canon::utf16::clip(path, 512).to_owned()),
            object: None,
        },
        name: None,
        layout_type: None,
        master: None,
        show_master: true,
        color_map_override: None,
        ext: None,
        background: None,
        elements: Vec::new(),
        failed: None,
    }
}

fn slide_skeleton(page: &PageRef<'_>, layout: Option<NodeId>, hidden: bool) -> Slide {
    Slide {
        id: page.id.clone(),
        origin: Origin::Source {
            part: CONTENT.to_owned(),
            path: Some(slidock_canon::utf16::clip(&page.path, 512).to_owned()),
            object: None,
        },
        layout,
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
    }
}

/// Reports the style chains a scope cut, under the part that holds each style.
fn report_cuts(scope: &StyleScope<'_>, part: &'static str, ctx: &mut FrontendCtx<'_>) {
    for cut in scope.take_cuts() {
        let part = if cut.common { STYLES } else { part };
        ctx.diag.report(
            codes::OD9006,
            DiagnosticSite::new()
                .data("part", part)
                .data("style", cut.style)
                .data("reason", cut.reason.as_str()),
        );
    }
}

/// The package opened and its document read, as both `analyze` and `lower` start.
fn open<'r>(
    input: &Input<'r>,
    ctx: &FrontendCtx<'_>,
) -> Result<(OdpPackage<'r>, Parts), FrontendError> {
    let pkg = OdpPackage::open(input.name, input.reader, ctx.limits.zip, ctx.limits.xml)?;
    let parts = Parts::read(&pkg, input.name)?;
    ctx.deadline.check()?;
    Ok((pkg, parts))
}

fn analyze(input: &Input<'_>, ctx: &mut FrontendCtx<'_>) -> Result<Analysis, FrontendError> {
    let (pkg, parts) = open(input, ctx)?;
    let common = CommonStyles::new(parts.styles(), Some(&parts.content));
    let doc = OdpDocument::new(&parts, &common)?;
    let slides: Vec<SlideFacts> = doc
        .pages
        .iter()
        .map(|page| SlideFacts {
            id: page.id.clone(),
            hidden: is_hidden(page.element, &doc.content_styles),
            title: None,
        })
        .collect();
    let features = if slides.iter().any(|slide| slide.hidden) {
        vec![FeatureId::PresentationHiddenSlide]
    } else {
        Vec::new()
    };
    Ok(Analysis {
        format: FORMAT,
        size: doc.size.clone(),
        slides,
        features,
        media_bytes: media_bytes(&pkg, &parts),
        refusal: None,
    })
}

fn lower(
    input: &Input<'_>,
    ctx: &mut FrontendCtx<'_>,
    options: &LowerOptions<'_>,
) -> Result<LowerResult, FrontendError> {
    let motion = if options.native_motion {
        MotionMode::Native
    } else {
        MotionMode::Detect
    };
    let (pkg, parts) = open(input, ctx)?;
    let mut common = CommonStyles::new(parts.styles(), Some(&parts.content));
    common.unitless_angle_tenths = gradient::unitless_angle_in_tenths(
        parts
            .styles()
            .and_then(|root| root.attr(qn!("office:version"))),
        pkg.generator_text().as_deref(),
    );
    let doc = OdpDocument::new(&parts, &common)?;
    let limits = *ctx.limits;
    let wanted: Option<BTreeSet<&NodeId>> = options.only.map(|only| only.iter().collect());
    let is_wanted = |id: &NodeId| wanted.as_ref().is_none_or(|wanted| wanted.contains(id));
    let media = RefCell::new(MediaRegistry::default());
    let mut cells = presentation_budget(&limits);
    let mut pages: BTreeMap<&str, NodeId> = BTreeMap::new();
    for page in &doc.pages {
        if let Some(name) = page
            .element
            .attr(qn!("draw:name"))
            .filter(|s| !s.is_empty())
        {
            pages.entry(name).or_insert_with(|| page.id.clone());
        }
    }
    let mut features: IndexMap<String, Vec<FeatureId>> = IndexMap::new();

    // The pages decide which master pages a partial lowering needs.
    let used: IndexSet<&str> = doc
        .pages
        .iter()
        .filter(|page| is_wanted(&page.id))
        .filter_map(master_of)
        .collect();

    // Master pages, then pages: each charges the presentation's cells its tables count, lowered or skipped.
    let mut layouts: IndexMap<NodeId, Layout> = IndexMap::new();
    let mut layout_of: BTreeMap<&str, NodeId> = BTreeMap::new();
    for master in doc.masters.values() {
        let slots = table_slots(master.element);
        if wanted.is_some() && !used.contains(master.name) {
            charge_container(&mut cells, slots);
            continue;
        }
        let skeleton = layout_skeleton(master.id.clone(), Some(&master.path));
        let mut seen = None;
        let budget = container_budget(&limits, &cells);
        let layout = isolate_container(&skeleton, ctx, &failure(STYLES), |ctx| {
            let state = ContainerState::new(master.id.clone(), budget);
            let lowering = Lowering {
                pkg: &pkg,
                part: STYLES,
                key: master.id.body(),
                styles: &doc.master_styles,
                master_name: Some(master.name),
                declarations: None,
                pages: &pages,
                media: &media,
                limits: &limits,
                state: &state,
            };
            if master.duplicate {
                lowering.report(codes::OD9004, DiagnosticSite::new().data("id", master.name));
            }
            let result = lower_master_page(master, &lowering);
            state.flush(ctx.diag);
            seen = Some(state.seen.borrow().clone());
            result
        })?;
        report_cuts(&doc.content_styles, CONTENT, ctx);
        report_cuts(&doc.master_styles, STYLES, ctx);
        if let Some(seen) = seen.filter(|_| layout.failed.is_none()) {
            features.insert(layout.id.as_str().to_owned(), sorted_features(seen));
        }
        charge_container(&mut cells, slots);
        layout_of.insert(master.name, layout.id.clone());
        layouts.insert(layout.id.clone(), layout);
    }
    // A page naming a master page the document lacks: failed when styles.xml cannot be read, else no layout.
    for name in &used {
        let Some(error) = doc.styles_error.filter(|_| !layout_of.contains_key(name)) else {
            continue;
        };
        let id = master_id(name).map_err(|error| FrontendError::Crashed(error.to_string()))?;
        let skeleton = layout_skeleton(id.clone(), None);
        ctx.diag.report(
            codes::OD9003,
            DiagnosticSite::new()
                .container(id.clone())
                .data("part", STYLES)
                .data("reason", error.reason.as_str()),
        );
        layouts.insert(
            id.clone(),
            skeleton.failed_skeleton(Failed::new(codes::OD9003, None), false),
        );
        layout_of.insert(name, id);
    }

    let mut slides: Vec<Slide> = Vec::new();
    for page in &doc.pages {
        let name = master_of(page).filter(|s| !s.is_empty());
        let master = name.and_then(|name| doc.masters.get(name));
        let slots = slide_slots(page, master);
        if !is_wanted(&page.id) {
            charge_container(&mut cells, slots);
            continue;
        }
        let layout = name.and_then(|name| layout_of.get(name)).cloned();
        let hidden = is_hidden(page.element, &doc.content_styles);
        let skeleton = slide_skeleton(page, layout.clone(), hidden);
        let declarations = doc.declarations_of(page.element);
        let mut seen = None;
        let budget = container_budget(&limits, &cells);
        let slide = isolate_container(&skeleton, ctx, &failure(CONTENT), |ctx| {
            let state = ContainerState::new(page.id.clone(), budget);
            let lowering = Lowering {
                pkg: &pkg,
                part: CONTENT,
                key: page.id.body(),
                styles: &doc.content_styles,
                master_name: name,
                declarations: Some(&declarations),
                pages: &pages,
                media: &media,
                limits: &limits,
                state: &state,
            };
            if page.duplicate {
                lowering.report(
                    codes::OD9004,
                    DiagnosticSite::new().data("id", page.id.as_str()),
                );
            }
            if let Some(name) = name.filter(|_| layout.is_none()) {
                lowering.report(codes::OD9007, DiagnosticSite::new().data("name", name));
            }
            let source = SlideSource {
                page,
                layout: layout.clone(),
                master,
                master_styles: &doc.master_styles,
                size: &doc.size,
                motion,
            };
            let result = lower_slide(&source, &lowering);
            state.flush(ctx.diag);
            seen = Some(state.seen.borrow().clone());
            result
        })?;
        report_cuts(&doc.content_styles, CONTENT, ctx);
        report_cuts(&doc.master_styles, STYLES, ctx);
        if let Some(seen) = seen.filter(|_| slide.failed.is_none()) {
            features.insert(slide.id.as_str().to_owned(), sorted_features(seen));
        }
        charge_container(&mut cells, slots);
        slides.push(slide);
    }
    features.insert("presentation".to_owned(), Vec::new());

    let store = media.borrow().store();
    let presentation = Presentation {
        schema: None,
        size: doc.size.clone(),
        defaults: Defaults {
            text: Box::new(default_text()),
            body: default_body(),
        },
        themes: IndexMap::new(),
        ext: None,
        phase: Phase::Source,
        masters: IndexMap::new(),
        layouts,
        slides,
        fonts: IndexMap::new(),
        media: media.into_inner().entries,
    };

    let app = pkg.generator();
    let crashed = |error: &dyn std::fmt::Display| FrontendError::Crashed(error.to_string());
    let sha256 =
        slidock_io::sha256_range(input.reader, 0, input.size()).map_err(|e| crashed(&e))?;
    let provenance = Provenance {
        format: FORMAT,
        name: input.name.to_owned(),
        sha256: Sha256::parse(&sha256).map_err(|e| crashed(&e))?,
        bytes: input.size(),
        app,
    };
    let fonts = fonts::embedded_fonts(
        &pkg,
        &[Some(&parts.content), parts.styles()],
        ctx.limits.zip.entry_bytes,
    );
    Ok(LowerResult {
        provenance,
        presentation: Unverified::from(presentation),
        media: Box::new(store),
        fonts,
        features,
    })
}

impl Frontend for OdpFrontend {
    fn format(&self) -> SourceFormat {
        FORMAT
    }

    fn version(&self) -> &str {
        "1"
    }

    fn sniff(&self, head: &[u8], _hints: &SniffHints<'_>) -> bool {
        package::is_odp_head(head)
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
