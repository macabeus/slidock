//! A group: a box its children are placed in, relative to its top-left corner (normalize-frames made them so). Its
//! shadow is that of everything it draws, as one silhouette.

use slidock_ir::resolved::Group;

use crate::attributes::{Kind, Root, common, element_attributes};
use crate::context::PrintCtx;
use crate::css::style;
use crate::error::Result;
use crate::frame::{box_decls, opacity_decl};
use crate::shadow::{ShadowForm, shadow_decl};
use crate::tree::{Element, lines};

pub fn print(group: &Group, ctx: &mut PrintCtx<'_>) -> Result<Element> {
    let axes = ctx.axes.then_frame(&group.frame);
    let mut decls = box_decls(&group.frame, None)?;
    decls.push(opacity_decl(group.opacity)?);
    // The shadow of the group's composite, not one per child.
    decls.push(shadow_decl(group.shadow.as_ref(), &axes, ShadowForm::Drop)?);
    let attrs =
        element_attributes(common!(group, Kind::Group), ctx, Root::Div)?.val("style", style(decls));
    let children = ctx.print_in(axes, &group.children)?;
    Ok(Element::new("div", attrs, lines(children)))
}
