//! PowerPoint (.pptx, Office Open XML) as an input. The frontend reads the package itself: the presentation part for
//! size, order and defaults, then each master with its theme, each layout and each slide, every one in isolation so an
//! unreadable part fails only its container. Inheritance stays open: placeholders keep their roles and indexes, and the
//! passes bind and resolve them. On the parity path transitions are effect names and animations are reported and
//! dropped; with native motion both map to the common effect vocabulary and the timeline (`timing`, `transition`). The
//! static render is always the slide fully built. Embedded fonts are recorded as font requests.

mod color;
mod containers;
mod context;
mod element;
mod embedded;
mod fill;
mod geometry;
mod line;
mod links;
mod media;
mod not_numbers;
mod package;
mod picture;
mod presentation;
mod report;
mod shadow;
mod shapes;
mod style_refs;
mod table;
mod table_style;
mod text;
mod theme;
mod timing;
mod transition;
mod units;
mod xml;

use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use indexmap::{IndexMap, IndexSet};
use slidock_canon::{js_trim, utf16};
use slidock_core_api::frontend::{App, SlideFacts};
use slidock_core_api::limits::{charge_container, presentation_budget};
use slidock_core_api::lowering::{FailureCodes, LowerError, isolate_container};
use slidock_core_api::{
    Analysis, Frontend, FrontendCtx, FrontendError, Input, LowerOptions, LowerResult, Provenance,
    SniffHints,
};
use slidock_ir::diagnostics::{DiagnosticSite, codes};
use slidock_ir::features::FeatureId;
use slidock_ir::formats::{FileFormat, IdPrefix, SourceFormat};
use slidock_ir::ids::{IdPart, NodeId, Sha256, ThemeId, node_id};
use slidock_ir::origin::Origin;
use slidock_ir::presentation::{Defaults, Phase};
use slidock_ir::source::{Layout, Master, Presentation, Slide};

pub use media::PptxMedia;

use containers::{is_hidden, listed_layouts, lower_layout, lower_master, lower_slide};
use context::{Lowering, Shared};
use package::{Package, PartError, refusal, unless_unreadable};
use presentation::{
    PartRef, PptxPresentation, ecma_body, embedded_font_entries, presentation_part,
    read_pptx_presentation, text_defaults,
};
use report::{OnContainer, OnPresentation, Report};
use table::table_slots;
use table_style::read_table_styles;
use theme::{FormatScheme, ThemeData, read_theme};
use xml::{Element, Ns};

const FORMAT: SourceFormat = SourceFormat::File(FileFormat::Pptx);
pub(crate) const EMITTER: &str = "pptx";

/// The first `max` UTF-16 code units of `text` (`text.slice(0, max)`), a pair the cut splits dropped.
pub(crate) fn clip(text: &str, max: usize) -> String {
    utf16::clip(text, max).to_owned()
}

impl text::TextCx for OnPresentation<'_> {}

fn crashed(error: impl std::fmt::Display) -> FrontendError {
    FrontendError::Crashed(error.to_string())
}

/// An error past the containers: a refusal stays one, anything else is a crash.
fn frontend_error(error: LowerError) -> FrontendError {
    match error {
        LowerError::Refused(diagnostic) => FrontendError::Refused(diagnostic),
        LowerError::Stopped => FrontendError::Stopped,
        other => crashed(other),
    }
}

/// The package and its presentation part; a package without a readable presentation is refused.
fn open<'r>(
    input: &Input<'r>,
    ctx: &FrontendCtx<'_>,
) -> Result<(Package<'r>, PptxPresentation), FrontendError> {
    let pkg = Package::open(input.name, input.reader, ctx.limits.zip, ctx.limits.xml)?;
    let read = (|| -> Result<Result<PptxPresentation, String>, LowerError> {
        let Some(part) = presentation_part(&pkg)? else {
            return Ok(Err("the package has no presentation part".to_owned()));
        };
        let root = pkg.xml(&part)?;
        if !root.is(Ns::P, "presentation") {
            return Ok(Err(format!("{part} is not a PresentationML presentation")));
        }
        read_pptx_presentation(&pkg, &part, &root).map(Ok)
    })();
    match read {
        Ok(Ok(pptx)) => Ok((pkg, pptx)),
        Ok(Err(reason)) => Err(refusal(input.name, &reason)),
        // An unreadable presentation part refuses the whole package.
        Err(LowerError::Part { part, reason }) => {
            Err(refusal(input.name, &format!("{part}: {reason}")))
        }
        Err(other) => Err(frontend_error(other)),
    }
}

/// A slide's root, or the part error that fails the slide when it is lowered.
fn slide_root(pkg: &Package<'_>, part: Option<&str>) -> Result<Rc<Element>, PartError> {
    match part {
        Some(part) => pkg.xml(part),
        None => Err(PartError::new("(none)", "the slide has no part")),
    }
}

/// The grid slots a skipped container's part counts: none when it cannot be read, as its lowering then fails.
fn skipped_slots(pkg: &Package<'_>, part: Option<&str>) -> u64 {
    part.and_then(|part| pkg.xml(part).ok())
        .map_or(0, |root| table_slots(&root))
}

const MEDIA_RELATIONSHIPS: [&str; 4] = ["image", "media", "video", "audio"];

/// The total size of the media parts the masters, layouts and slides reference, each part counted once.
fn media_bytes(pkg: &Package<'_>, pptx: &PptxPresentation) -> u64 {
    let rels_of = |part: &str| pkg.rels(part).ok();
    let mut containers: IndexSet<String> = IndexSet::new();
    for master in &pptx.masters {
        containers.insert(master.part.clone());
        for rel in rels_of(&master.part).iter().flat_map(|rels| rels.values()) {
            if rel.kind == "slideLayout" {
                containers.insert(rel.target.clone());
            }
        }
    }
    for slide in &pptx.slides {
        if let Some(part) = &slide.part {
            containers.insert(part.clone());
        }
    }
    let mut media: IndexSet<String> = IndexSet::new();
    for part in &containers {
        for rel in rels_of(part).iter().flat_map(|rels| rels.values()) {
            if !rel.external && MEDIA_RELATIONSHIPS.contains(&rel.kind.as_str()) {
                media.insert(rel.target.clone());
            }
        }
    }
    media
        .iter()
        .map(|part| pkg.entry(part).map_or(0, |entry| entry.size))
        .fold(0, u64::saturating_add)
}

/// The app that wrote the package, from its extended properties.
fn app_of(pkg: &Package<'_>) -> Option<App> {
    let rel = pkg.first("", "extended-properties").ok().flatten()?;
    let root = pkg.xml(&rel.target).ok()?;
    let text_of = |local: &str| {
        root.kids()
            .find(|element| &*element.local == local)
            .map(|element| js_trim(&element.text_content()).to_owned())
    };
    let name = text_of("Application").filter(|name| !name.is_empty())?;
    let version = text_of("AppVersion").filter(|version| !version.is_empty());
    Some(App {
        name: clip(&name, 256),
        version: version.map(|version| clip(&version, 64)),
    })
}

fn theme_id(master: &NodeId) -> Result<ThemeId, LowerError> {
    node_id(
        IdPrefix::Pp,
        &[IdPart::Str("t"), IdPart::Str(master.body())],
    )
    .map_err(|error| LowerError::Other(error.to_string()))
}

/// The theme of a master; a part error fails the master.
fn read_theme_of(
    pkg: &Package<'_>,
    master: &PartRef,
    id: ThemeId,
    ctx: &mut FrontendCtx<'_>,
) -> Result<Option<ThemeData>, LowerError> {
    let Some(rel) = pkg.first(&master.part, "theme")? else {
        return Ok(None);
    };
    let root = pkg.xml(&rel.target)?;
    if !root.is(Ns::A, "theme") {
        return Err(PartError::new(rel.target, "not a DrawingML theme").into());
    }
    let mut report = OnContainer {
        sink: &mut *ctx.diag,
        container: master.id.clone(),
    };
    let (data, missing) = read_theme(&root, id, &rel.target, &mut report);
    if !missing.is_empty() {
        report.report(
            codes::PX2008,
            DiagnosticSite::new().data("slots", missing.join(", ")),
        );
    }
    Ok(Some(data))
}

fn master_skeleton(reference: &PartRef) -> Master {
    Master {
        id: reference.id.clone(),
        origin: Origin::source(reference.part.clone()),
        name: None,
        theme: None,
        color_map: None,
        text_styles: None,
        ext: None,
        background: None,
        elements: Vec::new(),
        failed: None,
    }
}

fn layout_skeleton(reference: &PartRef, master: Option<NodeId>) -> Layout {
    Layout {
        id: reference.id.clone(),
        origin: Origin::source(reference.part.clone()),
        name: None,
        layout_type: None,
        master,
        show_master: true,
        color_map_override: None,
        ext: None,
        background: None,
        elements: Vec::new(),
        failed: None,
    }
}

fn slide_skeleton(id: &NodeId, part: &str, layout: Option<NodeId>, hidden: bool) -> Slide {
    Slide {
        id: id.clone(),
        origin: Origin::source(part),
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

/// How a container fails: a part that is missing or malformed, a limit, anything else.
fn failure(part: &str) -> FailureCodes {
    FailureCodes::new(codes::PX2007)
        .part(codes::PX2002)
        .limit(codes::PX2901)
        .data("part", part)
}

/// The layout a slide shows, keyed by its part; a layout no master lists takes its id from its file name.
fn layout_of(
    pkg: &Package<'_>,
    part: &str,
    layouts: &mut IndexMap<String, PartRef>,
    pptx: &mut PptxPresentation,
) -> Result<Option<PartRef>, LowerError> {
    let Some(rel) = unless_unreadable(pkg.first(part, "slideLayout")) else {
        return Ok(None);
    };
    if let Some(layout) = layouts.get(&rel.target) {
        return Ok(Some(layout.clone()));
    }
    if !pkg.has(&rel.target) {
        return Ok(None);
    }
    let file = rel.target.rsplit('/').next().unwrap_or(&rel.target);
    let taken = pptx
        .ids
        .take(&[IdPart::Str("l"), IdPart::Str(file)])
        .map_err(|error| LowerError::Other(error.to_string()))?;
    let layout = PartRef {
        id: taken.id,
        part: rel.target.clone(),
        duplicate: taken.duplicate,
    };
    layouts.insert(rel.target, layout.clone());
    Ok(Some(layout))
}

/// Reports a container whose native id another container took first.
fn report_duplicate(id: &NodeId, duplicate: bool, lowering: &mut Lowering<'_>) {
    if duplicate {
        lowering.report(codes::PX2004, DiagnosticSite::new().data("id", id.as_str()));
    }
}

/// The PPTX frontend.
#[derive(Clone, Copy, Debug, Default)]
pub struct PptxFrontend;

impl PptxFrontend {
    pub fn new() -> Self {
        PptxFrontend
    }
}

/// A master's root as the planning read found it.
struct MasterRead {
    reference: PartRef,
    root: Result<Rc<Element>, LowerError>,
}

impl Frontend for PptxFrontend {
    fn format(&self) -> SourceFormat {
        FORMAT
    }

    fn version(&self) -> &str {
        "1"
    }

    fn sniff(&self, _head: &[u8], hints: &SniffHints<'_>) -> bool {
        hints
            .zip_names
            .is_some_and(|names| names.iter().any(|name| name == "ppt/presentation.xml"))
    }

    fn analyze(
        &self,
        input: &Input<'_>,
        ctx: &mut FrontendCtx<'_>,
    ) -> Result<Analysis, FrontendError> {
        let (pkg, pptx) = open(input, ctx)?;
        let mut slides = Vec::with_capacity(pptx.slides.len());
        for slide in &pptx.slides {
            ctx.deadline.check()?;
            let hidden = slide_root(&pkg, slide.part.as_deref()).is_ok_and(|root| is_hidden(&root));
            slides.push(SlideFacts {
                id: slide.id.clone(),
                hidden,
                title: None,
            });
        }
        let mut features = Vec::new();
        if !pptx.embedded_fonts.is_empty() {
            features.push(FeatureId::FontEmbedded);
        }
        if slides.iter().any(|slide| slide.hidden) {
            features.push(FeatureId::PresentationHiddenSlide);
        }
        features.sort();
        Ok(Analysis {
            format: FORMAT,
            size: pptx.size.clone(),
            slides,
            features,
            media_bytes: media_bytes(&pkg, &pptx),
            refusal: None,
        })
    }

    fn lower(
        &self,
        input: &Input<'_>,
        ctx: &mut FrontendCtx<'_>,
        options: &LowerOptions<'_>,
    ) -> Result<LowerResult, FrontendError> {
        let (pkg, mut pptx) = open(input, ctx)?;
        let wanted: Option<BTreeSet<&NodeId>> = options.only.map(|only| only.iter().collect());
        let media = media::MediaRegistry::default();
        let mut slide_parts: IndexMap<String, NodeId> = IndexMap::new();
        for slide in &pptx.slides {
            if let Some(part) = &slide.part {
                slide_parts.insert(part.clone(), slide.id.clone());
            }
        }
        let table_styles = read_table_styles(&pkg, &pptx.part);
        let limits = *ctx.limits;
        let size = pptx.size.clone();
        let shared = Shared {
            pkg: &pkg,
            slides: &slide_parts,
            table_styles: &table_styles,
            media: &media,
            limits: &limits,
            motion: options.native_motion.then_some(&size),
        };
        let mut cells = presentation_budget(&limits);
        let mut features: IndexMap<String, Vec<FeatureId>> = IndexMap::new();
        let mut nan_by_container = Vec::new();
        let empty_formats = FormatScheme::default();

        // Masters and the layouts they list.
        let mut masters: Vec<MasterRead> = Vec::new();
        let mut layout_by_part: IndexMap<String, PartRef> = IndexMap::new();
        for reference in pptx.masters.clone() {
            let read = pkg
                .xml(&reference.part)
                .map_err(LowerError::from)
                .and_then(|root| {
                    listed_layouts(&pkg, &reference, &root, &mut pptx.ids)
                        .map(|layouts| (root, layouts))
                });
            match read {
                Ok((root, layouts)) => {
                    for layout in layouts {
                        layout_by_part.entry(layout.part.clone()).or_insert(layout);
                    }
                    masters.push(MasterRead {
                        reference,
                        root: Ok(root),
                    });
                }
                Err(error @ LowerError::Part { .. }) => masters.push(MasterRead {
                    reference,
                    root: Err(error),
                }),
                Err(other) => return Err(frontend_error(other)),
            }
        }
        let mut master_by_part: IndexMap<String, PartRef> = IndexMap::new();
        for read in &masters {
            master_by_part.insert(read.reference.part.clone(), read.reference.clone());
        }

        // Slide relationships next: they add the layouts no master lists, in show order, and the lowered slides
        // decide which layouts and masters a partial lowering needs. A slide's own part is parsed when it is
        // lowered or counted, so one slide tree at a time is held.
        let hidden_slides: BTreeSet<&NodeId> = options
            .facts
            .slides
            .iter()
            .filter(|slide| slide.hidden)
            .map(|slide| &slide.id)
            .collect();
        let mut slide_reads = Vec::with_capacity(pptx.slides.len());
        for slide in pptx.slides.clone() {
            let layout = match &slide.part {
                Some(part) => {
                    layout_of(&pkg, part, &mut layout_by_part, &mut pptx).map_err(frontend_error)?
                }
                None => None,
            };
            let lowered = wanted
                .as_ref()
                .is_none_or(|wanted| wanted.contains(&slide.id));
            slide_reads.push((slide, layout, lowered));
        }
        let used_layouts: BTreeSet<String> = slide_reads
            .iter()
            .filter(|(_, _, lowered)| *lowered)
            .filter_map(|(_, layout, _)| layout.as_ref().map(|layout| layout.part.clone()))
            .collect();
        let mut layout_master: BTreeMap<String, Option<PartRef>> = BTreeMap::new();
        for reference in layout_by_part.values() {
            if wanted.is_some() && !used_layouts.contains(&reference.part) {
                continue;
            }
            let rel = unless_unreadable(pkg.first(&reference.part, "slideMaster"));
            let master = rel.and_then(|rel| master_by_part.get(&rel.target).cloned());
            layout_master.insert(reference.part.clone(), master);
        }
        let used_masters: BTreeSet<String> = layout_master
            .values()
            .flatten()
            .map(|master| master.part.clone())
            .collect();

        // Masters, then layouts, then slides: each charges the presentation's cells its tables count, lowered or
        // skipped.
        let mut themes = IndexMap::new();
        let mut presentation_masters = IndexMap::new();
        let mut theme_by_master: BTreeMap<String, ThemeData> = BTreeMap::new();
        for read in &masters {
            let reference = &read.reference;
            let slots = read.root.as_ref().map_or(0, |root| table_slots(root));
            if wanted.is_some() && !used_masters.contains(&reference.part) {
                charge_container(&mut cells, slots);
                continue;
            }
            let theme = theme_id(&reference.id).map_err(frontend_error)?;
            let skeleton = master_skeleton(reference);
            let mut seen = None;
            let master = isolate_container(&skeleton, ctx, &failure(&reference.part), |ctx| {
                let root = read.root.clone()?;
                let data = read_theme_of(&pkg, reference, theme.clone(), ctx)?;
                let rels = pkg.rels(&reference.part)?;
                let formats = data.as_ref().map_or(&empty_formats, |data| &data.formats);
                let mut lowering = Lowering::new(
                    shared,
                    &reference.part,
                    reference.id.clone(),
                    rels,
                    formats,
                    &cells,
                    &mut *ctx.diag,
                );
                report_duplicate(&reference.id, reference.duplicate, &mut lowering);
                if let Some(data) = &data {
                    themes.insert(theme.clone(), data.theme.clone());
                    theme_by_master.insert(reference.part.clone(), data.clone());
                }
                let shown_theme = data.as_ref().map(|_| theme.clone());
                let master = lower_master(&root, shown_theme, &mut lowering)?;
                seen = Some((lowering.reporter.features(), lowering.not_numbers));
                Ok(master)
            })?;
            if let Some((seen, nan)) = seen {
                features.insert(master.id.as_str().to_owned(), seen);
                nan_by_container.push((master.id.clone(), nan));
            }
            charge_container(&mut cells, slots);
            presentation_masters.insert(master.id.clone(), master);
        }

        let mut presentation_layouts = IndexMap::new();
        for reference in layout_by_part.values() {
            if wanted.is_some() && !used_layouts.contains(&reference.part) {
                charge_container(&mut cells, skipped_slots(&pkg, Some(&reference.part)));
                continue;
            }
            let master = layout_master.get(&reference.part).cloned().flatten();
            let master_id = master.as_ref().map(|master| master.id.clone());
            let skeleton = layout_skeleton(reference, master_id.clone());
            let formats = master
                .as_ref()
                .and_then(|master| theme_by_master.get(&master.part))
                .map_or(&empty_formats, |data| &data.formats);
            let mut seen = None;
            let mut slots = 0;
            let layout = isolate_container(&skeleton, ctx, &failure(&reference.part), |ctx| {
                let root = pkg.xml(&reference.part)?;
                slots = table_slots(&root);
                let rels = pkg.rels(&reference.part)?;
                let mut lowering = Lowering::new(
                    shared,
                    &reference.part,
                    reference.id.clone(),
                    rels,
                    formats,
                    &cells,
                    &mut *ctx.diag,
                );
                report_duplicate(&reference.id, reference.duplicate, &mut lowering);
                let layout = lower_layout(&root, master_id.clone(), &mut lowering)?;
                seen = Some((lowering.reporter.features(), lowering.not_numbers));
                Ok(layout)
            })?;
            if let Some((seen, nan)) = seen {
                features.insert(layout.id.as_str().to_owned(), seen);
                nan_by_container.push((layout.id.clone(), nan));
            }
            charge_container(&mut cells, slots);
            presentation_layouts.insert(layout.id.clone(), layout);
        }

        let mut slides = Vec::new();
        for (reference, layout, lowered) in &slide_reads {
            if !lowered {
                charge_container(&mut cells, skipped_slots(&pkg, reference.part.as_deref()));
                continue;
            }
            let part = reference.part.clone().unwrap_or_else(|| pptx.part.clone());
            let master = layout
                .as_ref()
                .and_then(|layout| layout_master.get(&layout.part).cloned().flatten());
            let layout_id = layout.as_ref().map(|layout| layout.id.clone());
            let skeleton = slide_skeleton(
                &reference.id,
                &part,
                layout_id.clone(),
                hidden_slides.contains(&reference.id),
            );
            let formats = master
                .as_ref()
                .and_then(|master| theme_by_master.get(&master.part))
                .map_or(&empty_formats, |data| &data.formats);
            let mut seen = None;
            let mut slots = 0;
            let slide = isolate_container(&skeleton, ctx, &failure(&part), |ctx| {
                let root = slide_root(&pkg, reference.part.as_deref())?;
                slots = table_slots(&root);
                let rels = pkg.rels(&part)?;
                let mut lowering = Lowering::new(
                    shared,
                    &part,
                    reference.id.clone(),
                    rels,
                    formats,
                    &cells,
                    &mut *ctx.diag,
                );
                report_duplicate(&reference.id, reference.duplicate, &mut lowering);
                let slide = lower_slide(&root, layout_id.clone(), &mut lowering)?;
                seen = Some((lowering.reporter.features(), lowering.not_numbers));
                Ok(slide)
            })?;
            if let Some((seen, nan)) = seen {
                features.insert(slide.id.as_str().to_owned(), seen);
                nan_by_container.push((slide.id.clone(), nan));
            }
            charge_container(&mut cells, slots);
            slides.push(slide);
        }

        for relisted in &pptx.relisted {
            ctx.diag.report(
                codes::PX2013,
                DiagnosticSite::new()
                    .data("list", relisted.list)
                    .data("part", relisted.part.as_str()),
            );
        }

        // Presentation-level: the format's defaults and the embedded fonts, recorded as requests.
        let fonts = embedded_font_entries(&pptx);
        for entry in fonts.values() {
            ctx.diag.report(
                codes::PX2501,
                DiagnosticSite::new().data("family", entry.request.family.as_str()),
            );
        }
        let presentation_features = if fonts.is_empty() {
            Vec::new()
        } else {
            vec![FeatureId::FontEmbedded]
        };
        features.insert("presentation".to_owned(), presentation_features);
        let defaults = text_defaults(&pptx, &mut OnPresentation(&mut *ctx.diag));
        let presentation = Presentation {
            schema: None,
            size: pptx.size.clone(),
            defaults: Defaults {
                text: Box::new(defaults),
                body: ecma_body(),
            },
            themes,
            ext: None,
            phase: Phase::Source,
            masters: presentation_masters,
            layouts: presentation_layouts,
            slides,
            fonts,
            media: media.entries(),
        };

        let app = app_of(&pkg);
        let hex = slidock_io::sha256_range(input.reader, 0, input.size()).map_err(crashed)?;
        let provenance = Provenance {
            format: FORMAT,
            name: input.name.to_owned(),
            sha256: Sha256::parse(&hex).map_err(crashed)?,
            bytes: input.size(),
            app,
        };
        Ok(LowerResult {
            provenance,
            presentation: not_numbers::unverified(presentation, &nan_by_container)
                .map_err(crashed)?,
            media: Box::new(media.store()),
            fonts: embedded::embedded_fonts(&pkg, &pptx.embedded_fonts, ctx.limits.zip.entry_bytes),
            features,
        })
    }
}
