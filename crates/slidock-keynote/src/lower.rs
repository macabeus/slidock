//! Slides and template slides as IR containers. Slides come in show order: the slide tree depth first, as indented
//! slides play, skipped slides included and hidden. Template slides become masterless layouts: their drawables minus
//! the tagged prototypes each slide copies, their placeholders as the layout placeholders a slide's own placeholders
//! bind to. Builds, transitions and Magic Move keys come from [`crate::animation`].

use std::collections::{BTreeMap, BTreeSet};

use slidock_core_api::lowering::LowerError;
use slidock_ir::features::FeatureId;
use slidock_ir::ids::NodeId;
use slidock_ir::origin::Origin;
use slidock_ir::paint::Fill;
use slidock_ir::source::{Layout, Slide};

use crate::animation::{Placement, assign_keys, placed_transition, timeline};
use crate::context::{Layer, Lowering};
use crate::drawables::{DrawableContext, lower_drawables, placeholder_fields, table_slots};
use crate::graph::{ObjectGraph, referenced};
use crate::iwa::ObjectId;
use crate::paint::{FillBox, fill};
use crate::proto::{kn, tsp};
use crate::styles::{SlideStyle, inherited, style_chain};
use crate::text::notes_body;

/// The slide tree's depth limit; deeper slides are left out of the show.
const MAX_TREE_DEPTH: usize = 32;

/// A node of the slide tree.
#[derive(Clone, Copy, Debug)]
pub struct SlideNode<'g> {
    pub id: ObjectId,
    pub node: &'g kn::SlideNodeArchive,
}

/// The slide tree, depth first: a slide, then the slides indented under it. A node whose slide another node shows
/// earlier is left out of the show (its indented slides stay), so a small index cannot show one slide many times
/// over; [`SlideOrder::relisted`] names each such slide once.
pub fn slide_order<'g>(
    graph: &'g ObjectGraph,
    roots: &[tsp::Reference],
    limit: usize,
) -> SlideOrder<'g> {
    struct Walk<'g> {
        graph: &'g ObjectGraph,
        limit: usize,
        seen: BTreeSet<ObjectId>,
        shown: BTreeSet<ObjectId>,
        order: SlideOrder<'g>,
    }
    fn visit<'g>(walk: &mut Walk<'g>, reference: &tsp::Reference, depth: usize) {
        let Some(id) = referenced(Some(reference)) else {
            return;
        };
        if walk.seen.contains(&id) || depth > MAX_TREE_DEPTH || walk.order.slides.len() > walk.limit
        {
            return;
        }
        walk.seen.insert(id);
        let Some(node) = walk.graph.get_id::<kn::SlideNodeArchive>(id) else {
            return;
        };
        match referenced(node.slide.as_ref()) {
            Some(slide) if !walk.shown.insert(slide) => {
                if !walk.order.relisted.contains(&slide) {
                    walk.order.relisted.push(slide);
                }
            }
            _ => walk.order.slides.push(SlideNode { id, node }),
        }
        for child in &node.children {
            visit(walk, child, depth + 1);
        }
    }
    let mut walk = Walk {
        graph,
        limit,
        seen: BTreeSet::new(),
        shown: BTreeSet::new(),
        order: SlideOrder::default(),
    };
    for reference in roots {
        visit(&mut walk, reference, 0);
    }
    walk.order
}

/// The show order, and the slides a later node of the tree showed again.
#[derive(Clone, Debug, Default)]
pub struct SlideOrder<'g> {
    pub slides: Vec<SlideNode<'g>>,
    pub relisted: Vec<ObjectId>,
}

/// Back to front.
fn z_order(slide: &kn::SlideArchive) -> &[tsp::Reference] {
    if slide.drawables_z_order.is_empty() {
        &slide.owned_drawables
    } else {
        &slide.drawables_z_order
    }
}

/// A template's drawables as its layout shows them: tagged objects are prototypes each slide copies.
fn layout_drawables(template: &kn::SlideArchive) -> Vec<tsp::Reference> {
    let tagged: BTreeSet<Option<u64>> = template
        .sage_tag_to_info_map
        .iter()
        .map(|entry| entry.info.as_ref().and_then(|info| info.identifier))
        .collect();
    z_order(template)
        .iter()
        .filter(|reference| !tagged.contains(&reference.identifier))
        .cloned()
        .collect()
}

/// The grid slots of the tables a slide or template lowers, counted from their models.
pub fn container_slots(
    graph: &ObjectGraph,
    slide: Option<&kn::SlideArchive>,
    layer: Layer,
    group_depth: usize,
) -> u64 {
    let Some(slide) = slide else {
        return 0;
    };
    match layer {
        Layer::Slide => table_slots(graph, z_order(slide), group_depth),
        Layer::Layout => table_slots(graph, &layout_drawables(slide), group_depth),
    }
}

fn background(ctx: &mut Lowering<'_>, style: Option<&tsp::Reference>) -> Option<Fill> {
    let chain = style_chain::<SlideStyle>(ctx.graph, style);
    let slide = FillBox::Frame {
        w: ctx.slide_width,
        h: ctx.slide_height,
    };
    let value = fill(
        inherited(&chain, |p| (p.fill.as_ref(), None)),
        slide,
        ctx,
        None,
    );
    if value.as_ref().is_some_and(|value| *value != Fill::None) {
        ctx.see(FeatureId::SlideBackground);
    }
    value
}

/// A template slide: its object and the index file it was read from.
#[derive(Clone, Copy, Debug)]
pub struct Template<'g> {
    pub id: ObjectId,
    pub message: &'g kn::SlideArchive,
    pub file: &'g str,
}

fn source_origin(file: &str, object: ObjectId) -> Origin {
    Origin::Source {
        part: file.to_owned(),
        path: None,
        object: Some(object.to_string()),
    }
}

pub fn lower_layout(
    ctx: &mut DrawableContext<'_>,
    template: &Template<'_>,
) -> Result<Layout, LowerError> {
    let message = template.message;
    let background = background(&mut ctx.low, message.style.as_ref());
    let elements = lower_drawables(ctx, &layout_drawables(message), 0)?;
    Ok(Layout {
        id: ctx.low.container().clone(),
        origin: source_origin(template.file, template.id),
        name: message
            .name
            .as_deref()
            .filter(|name| !name.is_empty())
            .map(|name| slidock_canon::utf16::clip(name, 256).to_owned()),
        layout_type: None,
        master: None,
        show_master: true,
        color_map_override: None,
        ext: None,
        background,
        elements,
        failed: None,
    })
}

/// Each placeholder kind's layout placeholder: the template's own field for the kind, when the layout holds it.
pub fn placeholder_targets(
    template: Option<&kn::SlideArchive>,
    layout: Option<&Layout>,
) -> BTreeMap<i32, NodeId> {
    let mut targets = BTreeMap::new();
    let (Some(template), Some(layout)) = (template, layout) else {
        return targets;
    };
    let mut by_object: BTreeMap<&str, &NodeId> = BTreeMap::new();
    for element in &layout.elements {
        if element.placeholder().is_some()
            && let Origin::Source {
                object: Some(object),
                ..
            } = element.origin()
            && !object.is_empty()
        {
            by_object.insert(object.as_str(), element.id());
        }
    }
    for (kind, field) in placeholder_fields(template) {
        let target = field
            .and_then(|field| field.identifier)
            .and_then(|id| by_object.get(id.to_string().as_str()).copied());
        if let Some(target) = target {
            targets.insert(kind, target.clone());
        }
    }
    targets
}

/// A slide: its object and the index file it was read from.
#[derive(Clone, Copy, Debug)]
pub struct SlideObject<'g> {
    pub id: ObjectId,
    pub message: &'g kn::SlideArchive,
    pub file: &'g str,
}

/// A slide; with `match_keys` (it leaves by Magic Move, or follows a slide that does), its elements carry their
/// match keys. Its transition is the one `placement` names.
pub fn lower_slide(
    ctx: &mut DrawableContext<'_>,
    slide: &SlideObject<'_>,
    layout: Option<&NodeId>,
    hidden: bool,
    match_keys: bool,
    placement: Placement<'_>,
) -> Result<Slide, LowerError> {
    let message = slide.message;
    if hidden {
        ctx.low.see(FeatureId::PresentationHiddenSlide);
    }
    let background = background(&mut ctx.low, message.style.as_ref());
    let mut elements = lower_drawables(ctx, z_order(message), 0)?;
    if match_keys {
        assign_keys(&mut elements);
    }
    let container = ctx.low.container().clone();
    let notes = notes_body(&mut ctx.low, message.note.as_ref(), &container)?;
    let transition = placed_transition(&mut ctx.low, message, placement);
    let timeline = timeline(&mut ctx.low, message, &elements);
    Ok(Slide {
        id: container,
        origin: source_origin(slide.file, slide.id),
        layout: layout.cloned(),
        hidden,
        show_layout: true,
        color_map_override: None,
        notes,
        transition,
        timeline,
        ext: None,
        background,
        elements,
        title: None,
        reading_order: None,
        failed: None,
    })
}
