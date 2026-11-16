//! What every drawable shares: its id (its native id, else its document-order index), origin, name, presentation role,
//! visibility, alt text and click link; the style chain its graphic properties come from; and its frame.

use std::rc::Rc;

use slidock_canon::utf16;
use slidock_core_api::lowering::LowerError;
use slidock_ir::diagnostics::{DiagnosticSite, codes};
use slidock_ir::elements::{Placeholder, PlaceholderRole};
use slidock_ir::features::FeatureId;
use slidock_ir::frame::Frame;
use slidock_ir::ids::{IdPart, NodeId};
use slidock_ir::origin::Origin;
use slidock_ir::text::Link;
use slidock_ir::units::{Angle, Pt, Size};

use crate::context::Lowering;
use crate::ids::native_id;
use crate::links::click_action;
use crate::names::qn;
use crate::odf::{Element, Ns};
use crate::styles::{Family, Props};
use crate::transform::placement;

/// `presentation:class` as the IR's placeholder roles, page thumbnails and handouts aside.
pub(crate) fn role_of(element: &Element) -> Option<PlaceholderRole> {
    Some(match element.attr(qn!("presentation:class"))? {
        "title" => PlaceholderRole::Title,
        "outline" | "text" => PlaceholderRole::Body,
        "subtitle" => PlaceholderRole::SubTitle,
        "graphic" => PlaceholderRole::Pic,
        "object" | "orgchart" => PlaceholderRole::Obj,
        "chart" => PlaceholderRole::Chart,
        "table" => PlaceholderRole::Tbl,
        "page-number" => PlaceholderRole::SldNum,
        "date-time" => PlaceholderRole::Dt,
        "footer" => PlaceholderRole::Ftr,
        "header" => PlaceholderRole::Hdr,
        "notes" => PlaceholderRole::Notes,
        _ => return None,
    })
}

/// The frame of an unsized drawable outside any placeholder: an empty box at the origin.
pub(crate) fn no_frame() -> Frame {
    Frame {
        x: Pt::ZERO,
        y: Pt::ZERO,
        w: Size::ZERO,
        h: Size::ZERO,
        rotation: Angle::ZERO,
        flip_h: false,
        flip_v: false,
    }
}

/// The fields every element has.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Base {
    pub id: NodeId,
    pub origin: Origin,
    pub name: Option<String>,
    pub placeholder: Option<Placeholder>,
    pub hidden: Option<bool>,
    pub alt: Option<String>,
    pub link: Option<Link>,
}

/// The id, origin, name, placeholder role, visibility, alt text and link of a drawable. `id` is given for content
/// materialised from a master page; anything else takes its native id, else the next index of its container.
pub(crate) fn element_base(
    element: &Element,
    path: &str,
    ctx: &Lowering<'_, '_>,
    link: Option<&Link>,
    id: Option<NodeId>,
) -> Result<Base, LowerError> {
    let native = native_id(element);
    let id = match id {
        Some(id) => id,
        None => {
            let key = IdPart::Str(ctx.key);
            let part = match native {
                Some(native) => IdPart::Str(native),
                None => IdPart::Int(ctx.next_index()),
            };
            let taken = ctx.take_id(&[key, part])?;
            if taken.duplicate {
                let shown = native.map_or_else(|| taken.id.as_str().to_owned(), str::to_owned);
                ctx.report(
                    codes::OD9004,
                    DiagnosticSite::new()
                        .node(taken.id.clone())
                        .data("id", shown),
                );
            }
            taken.id
        }
    };
    let name = element.attr(qn!("draw:name")).filter(|s| !s.is_empty());
    let trimmed_text = |local: &str| {
        element
            .child(Ns::Svg, local)
            .map(|e| slidock_canon::js_trim(&e.text_of()).to_owned())
            .filter(|s| !s.is_empty())
    };
    let alt = trimmed_text("desc").or_else(|| trimmed_text("title"));
    let display = element.attr(qn!("draw:display"));
    let hidden = matches!(display, Some("none" | "printer"));
    let role = role_of(element);
    let link = click_action(element, ctx.pages).or_else(|| link.cloned());
    if role.is_some() {
        ctx.see(FeatureId::Placeholder);
    }
    if hidden {
        ctx.see(FeatureId::ElementHidden);
    }
    if alt.is_some() {
        ctx.see(FeatureId::ElementAlt);
    }
    if link.is_some() {
        ctx.see(FeatureId::ElementLink);
    }
    Ok(Base {
        id,
        origin: Origin::Source {
            part: ctx.part.to_owned(),
            path: Some(utf16::clip(path, 512).to_owned()),
            object: native.map(|native| utf16::clip(native, 256).to_owned()),
        },
        name: name.map(|name| utf16::clip(name, 256).to_owned()),
        placeholder: role.map(|role| Placeholder {
            role,
            index: None,
            target: None,
        }),
        hidden: hidden.then_some(true),
        alt: alt.map(|alt| utf16::clip(&alt, 65_536).to_owned()),
        link,
    })
}

/// The properties a drawable's style chain gives: a presentation style for presentation objects, else a graphic one.
pub(crate) fn style_of<'a>(element: &Element, ctx: &Lowering<'a, '_>) -> Rc<Props<'a>> {
    match element
        .attr(qn!("presentation:style-name"))
        .filter(|s| !s.is_empty())
    {
        Some(style) => ctx.styles.resolve(Family::Presentation, Some(style)),
        None => ctx
            .styles
            .resolve(Family::Graphic, element.attr(qn!("draw:style-name"))),
    }
}

/// The flips a drawable's geometry or picture adds to its frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Flips {
    pub flip_h: bool,
    pub flip_v: bool,
}

/// The frame of a drawable, with the flips its geometry or picture adds; an unsized placeholder inherits its
/// parent's, any other unsized drawable is an empty box. A shear is dropped (OD9106).
pub(crate) fn frame_of(
    element: &Element,
    base: &Base,
    ctx: &Lowering<'_, '_>,
    flips: Flips,
) -> Option<Frame> {
    let Some(placed) = placement(element) else {
        return if base.placeholder.is_some() {
            None
        } else {
            Some(no_frame())
        };
    };
    if placed.unreadable {
        let written = element.attr(qn!("draw:transform")).unwrap_or("");
        ctx.report(
            codes::OD9111,
            DiagnosticSite::new()
                .node(base.id.clone())
                .data("transform", utf16::clip(written, 128).to_owned()),
        );
    }
    if placed.sheared {
        ctx.see(FeatureId::FrameShear);
        ctx.report(
            codes::OD9106,
            DiagnosticSite::new()
                .node(base.id.clone())
                .feature(FeatureId::FrameShear),
        );
    }
    let frame = Frame {
        flip_h: placed.frame.flip_h != flips.flip_h,
        flip_v: placed.frame.flip_v != flips.flip_v,
        ..placed.frame
    };
    if frame.rotation.steps() != 0 {
        ctx.see(FeatureId::FrameRotation);
    }
    if frame.flip_h || frame.flip_v {
        ctx.see(FeatureId::FrameFlip);
    }
    Some(frame)
}
