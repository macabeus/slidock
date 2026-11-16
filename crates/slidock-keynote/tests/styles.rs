//! Style chains by presence: an unset property comes from the parent, a `_null` flag ends the walk, cycles and
//! long chains stop.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use std::borrow::Cow;

use slidock_keynote::ObjectGraph;
use slidock_keynote::graph::{IndexBudget, IndexSource, ReadMany, read_object_graph};
use slidock_keynote::proto::{tsd, tsp, tswp};
use slidock_keynote::styles::{
    CharacterStyle, ListStyle, MAX_STYLE_DEPTH, ParagraphStyle, ShapeStyle, TextShapeStyle,
    inherited, style_chain,
};
use support::{PresentationBuilder, r, style};

struct Files(Vec<(String, Vec<u8>)>);

impl IndexSource for Files {
    fn entry_sizes(&self) -> Vec<(&str, u64)> {
        self.0
            .iter()
            .map(|(p, d)| (p.as_str(), d.len() as u64))
            .collect()
    }

    fn read_many(&self, _paths: &[&str]) -> ReadMany<'_> {
        let mut out = ReadMany::default();
        for (path, data) in &self.0 {
            out.read
                .insert(path.clone(), Cow::Borrowed(data.as_slice()));
        }
        out
    }
}

fn graph_of(presentation: &mut PresentationBuilder) -> ObjectGraph {
    read_object_graph(&Files(presentation.files()), &IndexBudget::default()).unwrap()
}

fn red() -> tsp::Color {
    tsp::Color {
        model: Some(1),
        r: Some(1.0),
        g: Some(0.0),
        b: Some(0.0),
        a: Some(1.0),
        ..Default::default()
    }
}

#[test]
fn inherits_by_presence_an_unset_size_comes_from_the_parent_never_0_and_a_null_flag_ends_the_walk()
{
    let mut presentation = PresentationBuilder::new();
    let body = presentation.paragraph_style(
        tswp::CharacterStylePropertiesArchive {
            font_size: Some(80.0),
            font_color: Some(red()),
            ..Default::default()
        },
        None,
    );
    let child = presentation.paragraph_style(
        tswp::CharacterStylePropertiesArchive {
            bold: Some(true),
            font_color_null: Some(true),
            ..Default::default()
        },
        Some(body),
    );
    let graph = graph_of(&mut presentation);
    let chain = style_chain::<CharacterStyle>(&graph, Some(&child));
    assert_eq!(chain.len(), 2);
    assert_eq!(
        inherited(&chain, |p| (p.font_size.as_ref(), None)),
        Some(&80.0)
    );
    assert_eq!(inherited(&chain, |p| (p.bold.as_ref(), None)), Some(&true));
    assert_eq!(
        inherited(&chain, |p| (p.font_color.as_ref(), p.font_color_null)),
        None
    );
    assert_eq!(inherited(&chain, |p| (p.italic.as_ref(), None)), None);
    // Without the flag the walk reaches the parent's red.
    assert_eq!(
        inherited(&chain, |p| (p.font_color.as_ref(), None)),
        Some(&red())
    );
    // The paragraph chain of the same styles.
    assert_eq!(style_chain::<ParagraphStyle>(&graph, Some(&child)).len(), 2);
}

#[test]
fn stops_a_chain_at_a_cycle() {
    let mut presentation = PresentationBuilder::new();
    let a = tswp::CharacterStyleArchive {
        super_: Some(style(Some(r(1001)))),
        char_properties: Some(Default::default()),
        ..Default::default()
    };
    let a = presentation.add("TSWP.CharacterStyleArchive", &a);
    let b = tswp::CharacterStyleArchive {
        super_: Some(style(Some(a))),
        char_properties: Some(tswp::CharacterStylePropertiesArchive {
            bold: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    };
    presentation.add("TSWP.CharacterStyleArchive", &b);
    let graph = graph_of(&mut presentation);
    assert_eq!(style_chain::<CharacterStyle>(&graph, Some(&a)).len(), 2);
    // A style that is its own parent.
    let mut presentation = PresentationBuilder::new();
    let own = tswp::CharacterStyleArchive {
        super_: Some(style(Some(r(1000)))),
        char_properties: Some(Default::default()),
        ..Default::default()
    };
    let own = presentation.add("TSWP.CharacterStyleArchive", &own);
    let graph = graph_of(&mut presentation);
    assert_eq!(style_chain::<CharacterStyle>(&graph, Some(&own)).len(), 1);
}

#[test]
fn cuts_a_long_chain_and_ends_at_an_object_of_another_kind() {
    let mut presentation = PresentationBuilder::new();
    let mut parent = None;
    for _ in 0..40 {
        parent = Some(presentation.paragraph_style(Default::default(), parent));
    }
    let leaf = parent.unwrap();
    let graph = graph_of(&mut presentation);
    assert_eq!(
        style_chain::<CharacterStyle>(&graph, Some(&leaf)).len(),
        MAX_STYLE_DEPTH
    );

    let mut presentation = PresentationBuilder::new();
    let other = presentation.storage("x", None);
    let styled = presentation.paragraph_style(
        tswp::CharacterStylePropertiesArchive {
            bold: Some(true),
            ..Default::default()
        },
        Some(other),
    );
    let graph = graph_of(&mut presentation);
    assert_eq!(
        style_chain::<CharacterStyle>(&graph, Some(&styled)).len(),
        1
    );
    assert!(style_chain::<CharacterStyle>(&graph, None).is_empty());
    assert!(style_chain::<CharacterStyle>(&graph, Some(&r(1))).is_empty());
}

#[test]
fn a_text_shape_style_wraps_the_plain_shape_style_and_a_style_without_properties_is_passed() {
    let mut presentation = PresentationBuilder::new();
    let plain = tsd::ShapeStyleArchive {
        super_: Some(style(None)),
        shape_properties: Some(tsd::ShapeStylePropertiesArchive {
            opacity: Some(0.5),
            ..Default::default()
        }),
        ..Default::default()
    };
    let plain = presentation.add("TSD.ShapeStyleArchive", &plain);
    // A plain shape style with no properties of its own, between the text style and the plain one.
    let bare = tsd::ShapeStyleArchive {
        super_: Some(style(Some(plain))),
        ..Default::default()
    };
    let bare = presentation.add("TSD.ShapeStyleArchive", &bare);
    let text = tswp::ShapeStyleArchive {
        super_: Some(tsd::ShapeStyleArchive {
            super_: Some(style(Some(bare))),
            shape_properties: Some(Default::default()),
            ..Default::default()
        }),
        shape_properties: Some(tswp::ShapeStylePropertiesArchive {
            shrink_to_fit: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    };
    let text = presentation.add("TSWP.ShapeStyleArchive", &text);
    let graph = graph_of(&mut presentation);
    let shape = style_chain::<ShapeStyle>(&graph, Some(&text));
    assert_eq!(shape.len(), 2);
    assert_eq!(
        inherited(&shape, |p| (p.opacity.as_ref(), None)),
        Some(&0.5)
    );
    let layout = style_chain::<TextShapeStyle>(&graph, Some(&text));
    assert_eq!(layout.len(), 1);
    assert_eq!(layout[0].shrink_to_fit, Some(true));
}

#[test]
fn a_list_style_is_its_own_properties() {
    let mut presentation = PresentationBuilder::new();
    let parent = tswp::ListStyleArchive {
        super_: Some(style(None)),
        indents: vec![1.0, 2.0],
        ..Default::default()
    };
    let parent = presentation.add("TSWP.ListStyleArchive", &parent);
    let child = tswp::ListStyleArchive {
        super_: Some(style(Some(parent))),
        ..Default::default()
    };
    let child = presentation.add("TSWP.ListStyleArchive", &child);
    let graph = graph_of(&mut presentation);
    let chain = style_chain::<ListStyle>(&graph, Some(&child));
    assert_eq!(chain.len(), 2);
    assert!(chain[0].indents.is_empty());
    assert_eq!(chain[1].indents, [1.0, 2.0]);
}
