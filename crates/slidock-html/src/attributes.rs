//! The attributes every element carries, in a fixed order: its IR id (the anchor the player and editors find it by),
//! its kind, its match key (off the parity path), its placeholder, link, visibility, and role and accessible name.
//! Kind-specific data attributes come after these, then the style.

use slidock_canon::js_trim;
use slidock_format::markers::{MATCH, attr};
use slidock_ir::elements::Placeholder;
use slidock_ir::ids::NodeId;
use slidock_ir::text::{Inline, Link, ResolvedTextBody};

use crate::context::{Motion, PrintCtx};
use crate::error::Result;
use crate::text::link_value;
use crate::tree::Attrs;

/// The fields every element has that its root prints.
pub struct Common<'a> {
    pub id: &'a NodeId,
    pub kind: Kind<'a>,
    pub placeholder: Option<&'a Placeholder>,
    pub link: Option<&'a Link>,
    pub hidden: Option<bool>,
    pub alt: Option<&'a str>,
    pub match_key: Option<&'a str>,
}

/// An element's kind, with what decides its role: a shape's text.
pub enum Kind<'a> {
    Shape(Option<&'a ResolvedTextBody>),
    Picture,
    Media,
    Table,
    Group,
    Foreign,
}

impl Kind<'_> {
    fn name(&self) -> &'static str {
        match self {
            Kind::Shape(_) => "shape",
            Kind::Picture => "picture",
            Kind::Media => "media",
            Kind::Table => "table",
            Kind::Group => "group",
            Kind::Foreign => "foreign",
        }
    }
}

/// The common fields of an element struct, of the given kind.
macro_rules! common {
    ($element:expr, $kind:expr) => {
        $crate::attributes::Common {
            id: &$element.id,
            kind: $kind,
            placeholder: $element.placeholder.as_ref(),
            link: $element.link.as_ref(),
            hidden: $element.hidden,
            alt: $element.alt.as_deref(),
            match_key: $element.match_key.as_deref(),
        }
    };
}
pub(crate) use common;

/// The tag an element's attributes go on: a `div`, or the `table` a table prints as itself.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Root {
    Div,
    Table,
}

pub fn element_attributes(
    element: Common<'_>,
    ctx: &mut PrintCtx<'_>,
    root: Root,
) -> Result<Attrs> {
    let placeholder = match element.placeholder {
        None => None,
        Some(placeholder) => {
            let role = placeholder.role.as_str().to_owned();
            Some(match placeholder.index {
                None => role,
                Some(index) => format!("{role}:{index}"),
            })
        }
    };
    let link = match element.link {
        None => None,
        Some(link) => link_value(link, ctx, element.id)?,
    };
    let mut attrs = Attrs::new()
        .val(attr::ELEMENT, element.id.to_string())
        .val("data-kind", element.kind.name().to_owned())
        .val(
            MATCH,
            element
                .match_key
                .filter(|_| ctx.motion == Motion::On)
                .map(str::to_owned),
        )
        .val("data-placeholder", placeholder)
        .val("data-link", link)
        .flag("hidden", element.hidden == Some(true));
    // A picture names itself through its image's alt text; a table takes the name itself, keeping its table role.
    if let Some(alt) = element.alt
        && !matches!(element.kind, Kind::Picture)
    {
        let role = (root == Root::Div).then(|| named_role(&element.kind).to_owned());
        attrs = attrs.val("role", role).val("aria-label", alt.to_owned());
    }
    Ok(attrs)
}

/// The role that carries an element's alt text, since a plain `div` takes no accessible name: an image when nothing
/// inside it is read, else a group, which keeps its text and children readable.
fn named_role(kind: &Kind<'_>) -> &'static str {
    let Kind::Shape(text) = kind else {
        return "group";
    };
    let read = text.is_some_and(|text| {
        text.paragraphs.iter().any(|paragraph| {
            paragraph.inlines.iter().any(|inline| match inline {
                Inline::Field { .. } => true,
                Inline::Run { text, .. } => !js_trim(text).is_empty(),
                _ => false,
            })
        })
    });
    if read { "group" } else { "img" }
}
