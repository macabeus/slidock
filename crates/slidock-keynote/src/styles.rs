//! Style inheritance. A Keynote style holds only the properties it overrides and points at its parent; the effective
//! value of a property is the first one set walking up the chain. A `<name>_null` flag set on the way means
//! "explicitly none" and ends the walk. Decoded messages hold only the fields the file sets, so "set" is `Some`. The
//! chains are format-internal: the frontend resolves them into explicit IR values.

use std::collections::BTreeSet;

use crate::graph::{ObjectGraph, referenced};
use crate::iwa::{Archive, ObjectId};
use crate::proto::{kn, tsd, tsp, tst, tswp};

/// Longer chains are cut here; a cycle ends a chain where it repeats.
pub const MAX_STYLE_DEPTH: usize = 32;

/// A kind of style: the objects that are one, their own properties and their parent.
pub trait StyleLink {
    type Props;

    /// A style's properties and parent, or `None` for an object of another kind.
    fn link(archive: &Archive) -> Option<(Option<&Self::Props>, Option<&tsp::Reference>)>;
}

/// The properties of a style and of each ancestor, nearest first.
pub fn style_chain<'g, L: StyleLink>(
    graph: &'g ObjectGraph,
    reference: Option<&tsp::Reference>,
) -> Vec<&'g L::Props> {
    let mut chain = Vec::new();
    let mut seen: BTreeSet<ObjectId> = BTreeSet::new();
    let mut at = referenced(reference);
    while let Some(id) = at {
        if chain.len() >= MAX_STYLE_DEPTH || !seen.insert(id) {
            break;
        }
        let Some((props, parent)) = graph
            .resolve_id(id)
            .and_then(|object| L::link(object.archive))
        else {
            break;
        };
        if let Some(props) = props {
            chain.push(props);
        }
        at = referenced(parent);
    }
    chain
}

/// The first value of a property set along `chain`, or `None` when unset or set to none. `get` gives a style's value
/// and its `_null` flag.
pub fn inherited<'p, P, T: ?Sized>(
    chain: &[&'p P],
    get: impl Fn(&'p P) -> (Option<&'p T>, Option<bool>),
) -> Option<&'p T> {
    for props in chain {
        let (value, null) = get(props);
        if null == Some(true) {
            return None;
        }
        if value.is_some() {
            return value;
        }
    }
    None
}

macro_rules! link {
    ($(#[$doc:meta])* $name:ident, $props:ty, |$archive:ident| $body:expr) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug)]
        pub struct $name;

        impl StyleLink for $name {
            type Props = $props;

            fn link($archive: &Archive) -> Option<(Option<&$props>, Option<&tsp::Reference>)> {
                $body
            }
        }
    };
}

fn parent(style: Option<&crate::proto::tss::StyleArchive>) -> Option<&tsp::Reference> {
    style.and_then(|style| style.parent.as_ref())
}

link!(
    SlideStyle,
    kn::SlideStylePropertiesArchive,
    |archive| match archive {
        Archive::KnSlideStyleArchive(style) => Some((
            style.slide_properties.as_ref(),
            parent(style.super_.as_ref())
        )),
        _ => None,
    }
);

link!(
    /// Fill, stroke, opacity, shadow and line ends: a text shape's style wraps the plain shape style.
    ShapeStyle,
    tsd::ShapeStylePropertiesArchive,
    |archive| match archive {
        Archive::TsdShapeStyleArchive(style) => Some((
            style.shape_properties.as_ref(),
            parent(style.super_.as_ref())
        )),
        Archive::TswpShapeStyleArchive(style) => {
            let shape = style.super_.as_ref();
            Some((
                shape.and_then(|shape| shape.shape_properties.as_ref()),
                parent(shape.and_then(|shape| shape.super_.as_ref())),
            ))
        }
        _ => None,
    }
);

link!(
    /// A text shape's layout: vertical alignment, padding, shrink to fit.
    TextShapeStyle,
    tswp::ShapeStylePropertiesArchive,
    |archive| match archive {
        Archive::TswpShapeStyleArchive(style) => Some((
            style.shape_properties.as_ref(),
            parent(style.super_.as_ref().and_then(|shape| shape.super_.as_ref())),
        )),
        _ => None,
    }
);

link!(
    MediaStyle,
    tsd::MediaStylePropertiesArchive,
    |archive| match archive {
        Archive::TsdMediaStyleArchive(style) => Some((
            style.media_properties.as_ref(),
            parent(style.super_.as_ref())
        )),
        _ => None,
    }
);

link!(
    /// Character properties: a character style's, or the ones a paragraph style carries for its text.
    CharacterStyle,
    tswp::CharacterStylePropertiesArchive,
    |archive| match archive {
        Archive::TswpCharacterStyleArchive(style) => Some((
            style.char_properties.as_ref(),
            parent(style.super_.as_ref())
        )),
        Archive::TswpParagraphStyleArchive(style) => Some((
            style.char_properties.as_ref(),
            parent(style.super_.as_ref())
        )),
        _ => None,
    }
);

link!(
    ParagraphStyle,
    tswp::ParagraphStylePropertiesArchive,
    |archive| match archive {
        Archive::TswpParagraphStyleArchive(style) => Some((
            style.para_properties.as_ref(),
            parent(style.super_.as_ref())
        )),
        _ => None,
    }
);

link!(
    /// A list style's per-level arrays, which a child style sets whole.
    ListStyle,
    tswp::ListStyleArchive,
    |archive| match archive {
        Archive::TswpListStyleArchive(style) => {
            Some((Some(style.as_ref()), parent(style.super_.as_ref())))
        }
        _ => None,
    }
);

link!(
    /// A table cell's fill, padding, vertical alignment and wrapping.
    CellStyle,
    tst::CellStylePropertiesArchive,
    |archive| match archive {
        Archive::TstCellStyleArchive(style) => Some((
            style.cell_properties.as_ref(),
            parent(style.super_.as_ref())
        )),
        _ => None,
    }
);

link!(
    /// A table's banding and default grid strokes.
    TableStyle,
    tst::TableStylePropertiesArchive,
    |archive| match archive {
        Archive::TstTableStyleArchive(style) => Some((
            style.table_properties.as_ref(),
            parent(style.super_.as_ref())
        )),
        _ => None,
    }
);
