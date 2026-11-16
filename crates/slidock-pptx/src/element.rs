//! The parts every drawing element shares: its non-visual properties (id, name, placeholder, visibility, alt text,
//! link) and origin, its frame, the picture of a `blipFill` and how its text body reports.

use std::rc::Rc;

use slidock_core_api::lowering::LowerError;
use slidock_ir::diagnostics::{DiagnosticCode, DiagnosticSite, codes};
use slidock_ir::elements::{Placeholder, PlaceholderRole};
use slidock_ir::features::FeatureId;
use slidock_ir::frame::Frame;
use slidock_ir::ids::{IdPart, NodeId, Sha256};
use slidock_ir::origin::Origin;
use slidock_ir::text::Link;
use slidock_ir::units::{Angle, Pt, Size};

use crate::clip;
use crate::context::Lowering;
use crate::geometry::read_frame;
use crate::report::Report;
use crate::text::TextCx;
use crate::xml::{El, Element, Ns};

fn role(name: &str) -> PlaceholderRole {
    match name {
        "title" => PlaceholderRole::Title,
        "ctrTitle" => PlaceholderRole::CtrTitle,
        "subTitle" => PlaceholderRole::SubTitle,
        "body" => PlaceholderRole::Body,
        "obj" | "clipArt" | "dgm" => PlaceholderRole::Obj,
        "pic" => PlaceholderRole::Pic,
        "chart" => PlaceholderRole::Chart,
        "tbl" => PlaceholderRole::Tbl,
        "media" => PlaceholderRole::Media,
        "dt" => PlaceholderRole::Dt,
        "ftr" => PlaceholderRole::Ftr,
        "sldNum" => PlaceholderRole::SldNum,
        "hdr" => PlaceholderRole::Hdr,
        _ => PlaceholderRole::Other,
    }
}

/// The frame of an element inheritance cannot place: an empty box at the origin.
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

/// The non-visual properties every drawing element starts with (`p:nvSpPr`, `p:nvPicPr`, …): `cNvPr`, `nvPr`.
pub(crate) fn non_visual(
    element: &Element,
) -> (Option<&Element>, Option<&Element>, Option<&Element>) {
    let nv = element
        .kids()
        .find(|kid| kid.ns == Ns::P && kid.local.starts_with("nv"))
        .map(|rc| &**rc);
    (nv.child(Ns::P, "cNvPr"), nv.child(Ns::P, "nvPr"), nv)
}

/// What every element kind shares.
#[derive(Clone, Debug)]
pub(crate) struct Base {
    pub id: NodeId,
    pub origin: Origin,
    pub name: Option<String>,
    pub placeholder: Option<Placeholder>,
    pub hidden: Option<bool>,
    pub alt: Option<String>,
    pub link: Option<Link>,
}

fn id_failed(error: impl std::fmt::Display) -> LowerError {
    LowerError::Other(error.to_string())
}

pub(crate) fn element_base(
    element: &Element,
    path: &str,
    ctx: &mut Lowering<'_>,
) -> Result<Base, LowerError> {
    let (c_nv_pr, nv_pr, _) = non_visual(element);
    let native = c_nv_pr.attr("id").unwrap_or("_");
    let body = ctx.container.body().to_owned();
    let taken = ctx
        .ids
        .take(&[IdPart::Str(&body), IdPart::Str(native)])
        .map_err(id_failed)?;
    let id = taken.id;
    if taken.duplicate {
        ctx.report_on(&id, codes::PX2004, DiagnosticSite::new().data("id", native));
    }
    let name = c_nv_pr.attr("name").filter(|name| !name.is_empty());
    // PowerPoint's "Mark as decorative" (`a:extLst/a:ext/adec:decorative val="1"`) means say nothing, whatever
    // `descr` and `title` still hold.
    let decorative = c_nv_pr
        .child(Ns::A, "extLst")
        .children(Ns::A, "ext")
        .into_iter()
        .any(|ext| ext.child(Ns::Adec, "decorative").bool_attr("val") == Some(true));
    let alt = c_nv_pr
        .attr("descr")
        .filter(|alt| !alt.is_empty())
        .or_else(|| c_nv_pr.attr("title").filter(|alt| !alt.is_empty()))
        .filter(|_| !decorative);
    let hidden = c_nv_pr.bool_attr("hidden") == Some(true);
    let ph = nv_pr.child(Ns::P, "ph");
    let index = ph.int_attr("idx");
    let link = c_nv_pr
        .child(Ns::A, "hlinkClick")
        .and_then(|hlink| ctx.link(hlink));
    if ph.is_some() {
        ctx.seen(FeatureId::Placeholder);
    }
    if hidden {
        ctx.seen(FeatureId::ElementHidden);
    }
    if alt.is_some() {
        ctx.seen(FeatureId::ElementAlt);
    }
    if link.is_some() {
        ctx.seen(FeatureId::ElementLink);
    }
    Ok(Base {
        id,
        origin: Origin::Source {
            part: ctx.part.clone(),
            path: Some(clip(path, 512)),
            object: Some(clip(native, 256)),
        },
        name: name.map(|name| clip(name, 256)),
        placeholder: ph.map(|ph| Placeholder {
            role: role(ph.attr("type").unwrap_or("obj")),
            index: index
                .filter(|index| *index >= 0)
                .and_then(|index| u64::try_from(index).ok()),
            target: None,
        }),
        hidden: hidden.then_some(true),
        alt: alt.map(|alt| clip(alt, 65_536)),
        link,
    })
}

/// The frame of an element: its own, inherited when it is a placeholder, else an empty box.
pub(crate) fn frame_of(
    xfrm: Option<&Element>,
    base: &Base,
    ctx: &mut Lowering<'_>,
) -> Option<Frame> {
    let frame = read_frame(xfrm);
    if let Some(frame) = &frame {
        if frame.rotation != Angle::ZERO {
            ctx.seen(FeatureId::FrameRotation);
        }
        if frame.flip_h || frame.flip_v {
            ctx.seen(FeatureId::FrameFlip);
        }
    }
    match frame {
        Some(frame) => Some(frame),
        None if base.placeholder.is_some() => None,
        None => Some(no_frame()),
    }
}

/// The SVG part an Office 2016 picture keeps beside its raster (`a:blip/a:extLst/a:ext/asvg:svgBlip@r:embed`), when the
/// output shows it: the raster is PowerPoint's preview for older readers, so the SVG is drawn instead when it passes
/// the allow-list, and the raster otherwise, which is reported (PX2310).
fn svg_blip(
    blip: Option<&Rc<Element>>,
    ctx: &mut Lowering<'_>,
    node: Option<&NodeId>,
) -> Option<String> {
    let embed = blip?
        .child(Ns::A, "extLst")?
        .children(Ns::A, "ext")
        .find_map(|ext| ext.child(Ns::Asvg, "svgBlip"))?
        .rel_attr("embed")
        .unwrap_or("");
    let (part, refusal) = match ctx.rels.get(embed) {
        None => (embed.to_owned(), Some("no part is referenced".to_owned())),
        Some(rel) if rel.external => (
            rel.target.clone(),
            Some("it is linked, never fetched".to_owned()),
        ),
        Some(rel) => (
            rel.target.clone(),
            ctx.media.svg_refusal(ctx.pkg, &rel.target),
        ),
    };
    let Some(reason) = refusal else {
        return Some(part);
    };
    let site = DiagnosticSite::new()
        .data(
            "part",
            clip(if part.is_empty() { "(none)" } else { &part }, 256),
        )
        .data("reason", reason);
    ctx.report(
        codes::PX2310,
        match node {
            Some(node) => site.node(node.clone()),
            None => site,
        },
    );
    None
}

/// The picture a `blipFill` shows, recorded as media; a linked or unreadable picture is declined. An SVG it keeps
/// beside its raster is shown in its place (`svg_blip`).
pub(crate) fn blip_media(
    blip_fill: &Element,
    ctx: &mut Lowering<'_>,
    node: Option<&NodeId>,
    feature: FeatureId,
) -> Result<Option<Sha256>, LowerError> {
    let blip = blip_fill.child(Ns::A, "blip");
    let embed = blip.rel_attr("embed");
    let rel = embed
        .filter(|embed| !embed.is_empty())
        .and_then(|embed| ctx.rels.get(embed))
        .cloned();
    let site = || {
        let site = DiagnosticSite::new().feature(feature);
        match node {
            Some(node) => site.node(node.clone()),
            None => site,
        }
    };
    let rel = match rel {
        Some(rel) if !rel.external => rel,
        rel => {
            let link = blip.rel_attr("link").filter(|link| !link.is_empty());
            if rel.as_ref().is_some_and(|rel| rel.external) || link.is_some() {
                let target = match &rel {
                    Some(rel) => rel.target.clone(),
                    None => ctx
                        .rels
                        .get(link.unwrap_or(""))
                        .map(|rel| rel.target.clone())
                        .unwrap_or_default(),
                };
                ctx.report(codes::PX2301, site().data("target", clip(&target, 256)));
            } else {
                ctx.report(
                    codes::PX2307,
                    site()
                        .data("part", embed.unwrap_or("(none)"))
                        .data("reason", "no picture is referenced"),
                );
            }
            return Ok(None);
        }
    };
    let target = svg_blip(blip, ctx, node).unwrap_or(rel.target);
    match ctx.media.add(ctx.pkg, &target) {
        Ok(sha256) => Ok(Some(sha256)),
        Err(LowerError::Part { part, reason }) => {
            ctx.report(
                codes::PX2307,
                site().data("part", part).data("reason", reason),
            );
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

/// How a text body on an element reads: reports land on the element, links resolve in its container.
pub(crate) struct BodyCx<'l, 'a> {
    pub ctx: &'l mut Lowering<'a>,
    pub node: NodeId,
}

impl Report for BodyCx<'_, '_> {
    fn report(&mut self, code: DiagnosticCode, site: DiagnosticSite) {
        self.ctx.report_on(&self.node, code, site);
    }

    fn dropped(&mut self, modifier: &str) {
        Report::dropped(self.ctx, modifier);
    }
}

impl TextCx for BodyCx<'_, '_> {
    fn link(&mut self, hlink: &Element) -> Option<Link> {
        self.ctx.link(hlink)
    }

    fn seen(&mut self, feature: FeatureId) {
        self.ctx.seen(feature);
    }

    fn text_limit(&self) -> usize {
        self.ctx.limits.text_per_body
    }
}
