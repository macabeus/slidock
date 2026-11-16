//! Master pages (`style:master-page`) as layouts. Google's ODP export flattens a presentation's layouts into master
//! pages, one per layout used, so each master page is a standalone IR layout: its background (its drawing-page style),
//! its drawables as elements and its presentation objects as placeholders.

use slidock_canon::utf16;
use slidock_core_api::lowering::LowerError;
use slidock_ir::features::FeatureId;
use slidock_ir::origin::Origin;
use slidock_ir::paint::Fill;
use slidock_ir::source::Layout;

use crate::context::Lowering;
use crate::document::{MasterRef, page_props};
use crate::graphic::{Paint, read_fill};
use crate::names::qn;
use crate::odf::decode_style_name;
use crate::shapes::lower_drawables;

pub(crate) fn lower_master_page<'a>(
    master: &MasterRef<'a>,
    ctx: &Lowering<'a, '_>,
) -> Result<Layout, LowerError> {
    let paint = Paint::of(*ctx, None);
    let fill = read_fill(&page_props(master.element, ctx.styles), &paint)?.fill;
    if fill
        .as_ref()
        .is_some_and(|fill| !matches!(fill, Fill::None))
    {
        ctx.see(FeatureId::SlideBackground);
    }
    let name = decode_style_name(
        master
            .element
            .attr(qn!("style:display-name"))
            .unwrap_or(master.name),
    );
    Ok(Layout {
        id: ctx.container().clone(),
        origin: Origin::Source {
            part: ctx.part.to_owned(),
            path: Some(utf16::clip(&master.path, 512).to_owned()),
            object: None,
        },
        name: (!name.is_empty()).then(|| utf16::clip(&name, 256).to_owned()),
        layout_type: None,
        master: None,
        show_master: true,
        color_map_override: None,
        ext: None,
        background: fill,
        elements: lower_drawables(master.element, &master.path, ctx)?,
        failed: None,
    })
}
