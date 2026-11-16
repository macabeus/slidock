//! The printers' rules one at a time, on the fixture of `support` with one change each.
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod common;
mod support;

use slidock_format::dom::{Document, NodeId};
use slidock_format::lint_presentation;
use slidock_html::css::{MaybeDecl, sides};
use slidock_html::{
    Autosize, box_decls, css_color, literal_color, notes_text, supported_features, text_turn_decl,
};
use slidock_ir::color::{
    Color, ColorModOp, ColorSpace, MappedSlot, ResolvedColor, RgbColor, ThemeColor, ThemeTag,
};
use slidock_ir::diagnostics::{DataValue, Severity};
use slidock_ir::elements::MediaKind;
use slidock_ir::features::FeatureId;
use slidock_ir::frame::Frame;

use slidock_ir::diagnostics::codes;
use slidock_ir::ids::NodeId as IrId;
use slidock_ir::paint::{
    Cap, Dash, DashPreset, Fill, GradientStop, Join, LinearAngled, LinearLowered, RadialLowered,
    ResolvedCellFill, ResolvedFill, StrokePaint,
};
use slidock_ir::presentation::Failed;
use slidock_ir::resolved::{Body, Cell, Element, FailedA11y, Presentation};
use slidock_ir::table::{Border, CellBorders};
use slidock_ir::text::{
    Align, Anchor, AnchorX, AutoNumScheme, AutoNumbering, Caps, Decoration, FamilyFont, FieldKind,
    Inline, Insets, LineHeight, Link, LinkAction, Overflow, ResolvedAutofit, ResolvedBullet,
    ResolvedInline, ResolvedParagraph, ResolvedRunProps, ResolvedTextBody, TextRotation,
};
use slidock_ir::units::Size;
use support::*;

fn decls(list: Vec<MaybeDecl>) -> Vec<(String, String)> {
    list.into_iter()
        .flatten()
        .map(|(property, value)| (property.to_owned(), value))
        .collect()
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(p, v)| ((*p).to_owned(), (*v).to_owned()))
        .collect()
}

/// The element whose `data-el` is `id`.
fn element(doc: &Document, element: &str) -> NodeId {
    common::element_by_id(doc, element).unwrap_or_else(|| panic!("no element {element}"))
}

fn attr<'d>(doc: &'d Document, node: NodeId, name: &str) -> Option<&'d str> {
    doc.attribute(node, name)
}

/// `<tag …>` and the text that follows it up to the next tag, for each `<tag` whose opening tag contains `needle`.
fn tags_with(html: &str, tag: &str, needle: &str) -> Vec<String> {
    let open = format!("<{tag}");
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(at) = rest.find(&open) {
        let from = &rest[at..];
        let end = from.find('>').unwrap() + 1;
        let text_end = from[end..].find('<').map_or(from.len(), |i| end + i);
        if from[..end].contains(needle) {
            out.push(from[..text_end].to_owned());
        }
        rest = &from[end..];
    }
    out
}

/// The title element, its own text box: from its opening tag to the first `</div>`.
fn title_text(html: &str) -> String {
    let start = html.find("<div data-el=\"ir-s1.title\"").unwrap();
    let end = html[start..].find("</div>").unwrap() + start + "</div>".len();
    html[start..end].to_owned()
}

/// The opening tag of an element's markup.
fn opening(markup: &str) -> &str {
    &markup[..markup.find('>').unwrap() + 1]
}

/// The first `<p>` (or `<li>`) after the title's opening tag, through its end tag.
fn first_p(html: &str) -> String {
    let from = &html[html.find("data-el=\"ir-s1.title\"").unwrap()..];
    let start = [from.find("<p "), from.find("<li ")]
        .into_iter()
        .flatten()
        .min()
        .unwrap();
    let tag = if from[start..].starts_with("<p ") {
        "</p>"
    } else {
        "</li>"
    };
    let end = from[start..].find(tag).unwrap() + start + tag.len();
    from[start..end].to_owned()
}

fn rgb_f(r: f64, g: f64, b: f64, a: f64) -> RgbColor {
    rgba(r, g, b, a)
}

fn title_paragraph(presentation: &mut Presentation) -> &mut ResolvedParagraph {
    &mut shape_mut(presentation, "ir-s1.title")
        .text
        .as_mut()
        .unwrap()
        .paragraphs[0]
}

/// The title's first run's text and properties.
fn title_run(presentation: &mut Presentation) -> (String, ResolvedRunProps) {
    match &title_paragraph(presentation).inlines[0] {
        Inline::Run { text, props, .. } => (text.clone(), props.clone()),
        _ => panic!("run"),
    }
}

fn props_of(inline: &mut ResolvedInline) -> &mut ResolvedRunProps {
    match inline {
        Inline::Run { props, .. } | Inline::Field { props, .. } => props,
        _ => panic!("not a run"),
    }
}

fn skewed(text: &str, props: ResolvedRunProps, skew: f64) -> ResolvedInline {
    Inline::Run {
        text: text.to_owned(),
        props,
        line_height: None,
        leading_skew: Some(ratio(skew)),
    }
}

fn brk() -> ResolvedInline {
    Inline::Break { props: None }
}

mod colours {
    use super::*;

    #[test]
    fn prints_literals_as_hex_with_alpha_when_not_opaque_and_display_p3_as_color() {
        let color = rgb_f(1.0, 0.5, 0.0, 1.0);
        assert_eq!(literal_color(&color).unwrap(), "#ff8000");
        assert_eq!(
            literal_color(&rgb_f(1.0, 0.5, 0.0, 0.5)).unwrap(),
            "#ff800080"
        );
        let p3 = RgbColor {
            space: ColorSpace::P3,
            ..rgb_f(1.0, 0.5, 0.0, 0.25)
        };
        assert_eq!(
            literal_color(&p3).unwrap(),
            "color(display-p3 1 0.5 0 / 0.25)"
        );
    }

    #[test]
    fn prints_an_unmodified_theme_reference_as_a_variable_with_its_value_as_fallback() {
        let color = rgb_f(1.0, 0.5, 0.0, 1.0);
        assert_eq!(
            css_color(&theme_color(MappedSlot::Accent1, color.clone())).unwrap(),
            "var(--sd-color-accent1,#ff8000)"
        );
        assert_eq!(
            css_color(&modified(
                MappedSlot::Accent1,
                ColorModOp::LumMod,
                0.5,
                color
            ))
            .unwrap(),
            "#ff8000"
        );
    }
}

mod frames {
    use super::*;

    fn turned() -> Frame {
        Frame {
            rotation: angle(30.0),
            flip_h: true,
            ..frame(10.0, 20.0, 100.0, 50.0)
        }
    }

    #[test]
    fn places_by_left_top_and_turns_with_rotate_and_scale_never_transform() {
        assert_eq!(
            decls(box_decls(&turned(), None).unwrap()),
            pairs(&[
                ("position", "absolute"),
                ("left", "10pt"),
                ("top", "20pt"),
                ("width", "100pt"),
                ("height", "50pt"),
                ("rotate", "30deg"),
                ("scale", "-1 1"),
            ])
        );
    }

    #[test]
    fn gives_a_box_of_zero_width_or_height_a_transparent_extent_and_no_other_box_one() {
        let extent = ("box-shadow", "0 0 0 1pt transparent");
        let has_extent = |frame: &Frame, autosize: Option<&Autosize>| {
            decls(box_decls(frame, autosize).unwrap()).contains(&pairs(&[extent])[0])
        };
        // A straight connector, either way, turned or not; a point.
        assert!(has_extent(&frame(10.0, 20.0, 100.0, 0.0), None));
        assert!(has_extent(&frame(10.0, 20.0, 0.0, 100.0), None));
        assert!(has_extent(&frame(10.0, 20.0, 0.0, 0.0), None));
        assert!(has_extent(
            &Frame {
                rotation: angle(30.0),
                ..frame(10.0, 20.0, 100.0, 0.0)
            },
            None
        ));
        assert_eq!(
            decls(box_decls(&frame(10.0, 20.0, 100.0, 0.0), None).unwrap()),
            pairs(&[
                ("position", "absolute"),
                ("left", "10pt"),
                ("top", "20pt"),
                ("width", "100pt"),
                ("height", "0pt"),
                extent,
            ])
        );
        // Negative controls: a box with an extent (however thin), and the axes an autosized box grows on.
        assert!(!has_extent(&frame(10.0, 20.0, 100.0, 50.0), None));
        assert!(!has_extent(&frame(10.0, 20.0, 100.0, 0.012), None));
        let grows = Autosize {
            width: true,
            height: true,
            anchor_x: AnchorX::Left,
            anchor_y: Anchor::Top,
            max_width: None,
        };
        assert!(!has_extent(&frame(10.0, 20.0, 0.0, 0.0), Some(&grows)));
        let fixed_height = Autosize {
            height: false,
            ..grows
        };
        assert!(has_extent(
            &frame(10.0, 20.0, 0.0, 0.0),
            Some(&fixed_height)
        ));
    }

    #[test]
    fn anchors_an_autosized_box_with_a_percentage_translate() {
        let autosize = Autosize {
            width: true,
            height: true,
            anchor_x: AnchorX::Right,
            anchor_y: Anchor::Middle,
            max_width: Some(size(300.0)),
        };
        assert_eq!(
            decls(box_decls(&frame(10.0, 20.0, 100.0, 50.0), Some(&autosize)).unwrap()),
            pairs(&[
                ("position", "absolute"),
                ("left", "110pt"),
                ("top", "45pt"),
                ("width", "max-content"),
                ("max-width", "300pt"),
                ("translate", "-100% -50%"),
            ])
        );
    }

    #[test]
    fn unmirrors_readable_text_and_adds_its_half_turn_but_leaves_text_that_flips_with_its_frame_mirrored()
     {
        let base = frame(10.0, 20.0, 100.0, 50.0);
        let scale = |value: &str| Some(("scale", value.to_owned()));
        let body = |rotation: TextRotation, flips_with_frame: bool| ResolvedTextBody {
            rotation,
            flips_with_frame,
            ..text_body(Anchor::Top, ResolvedAutofit::None {}, vec![])
        };
        let flipped = |flip_h: bool, flip_v: bool, rotation: f64| Frame {
            flip_h,
            flip_v,
            rotation: angle(rotation),
            ..base.clone()
        };
        let readable = |rotation| body(rotation, false);
        assert_eq!(text_turn_decl(&base, &readable(TextRotation::Zero)), None);
        assert_eq!(
            text_turn_decl(&flipped(true, false, 0.0), &readable(TextRotation::Zero)),
            scale("-1 1")
        );
        // A vertical flip shows text turned half way, which is a horizontal flip of the flipped box.
        assert_eq!(
            text_turn_decl(&flipped(false, true, 0.0), &readable(TextRotation::Half)),
            scale("-1 1")
        );
        assert_eq!(
            text_turn_decl(&flipped(true, true, 0.0), &readable(TextRotation::Zero)),
            scale("-1 -1")
        );
        // Text that flips with its frame takes the element's flips and turn as they are: under Keynote's H, V (H
        // plus a half turn), both (a half turn) and H then 30°, the text box adds nothing.
        let mirrored = body(TextRotation::Zero, true);
        for frame in [
            flipped(true, false, 0.0),
            flipped(true, false, 180.0),
            flipped(false, false, 180.0),
            flipped(true, false, 330.0),
            flipped(false, true, 0.0),
            flipped(true, true, 0.0),
        ] {
            assert_eq!(text_turn_decl(&frame, &mirrored), None);
        }
        // Its half turn, were lower-geometry to give one, still applies.
        assert_eq!(
            text_turn_decl(&flipped(true, false, 0.0), &body(TextRotation::Half, true)),
            scale("-1 -1")
        );
    }
}

mod links {
    use super::*;

    #[test]
    fn prints_url_links_on_text_as_anchors_and_slide_links_as_data() {
        let html = slide_one_html(&full());
        assert!(html.contains(
            "<a href=\"https://example.com/spec\" style=\"color:inherit;text-decoration:inherit\">"
        ));
        assert!(slide_two_html(&full()).contains("data-link=\"slide:ir-s1\""));
    }

    #[test]
    fn drops_a_link_a_document_may_not_hold_keeping_its_text_with_html6001() {
        let mut presentation = full();
        let body = shape_mut(&mut presentation, "ir-s1.body");
        let paragraph = &mut body.text.as_mut().unwrap().paragraphs[0];
        props_of(&mut paragraph.inlines[0]).link = Some(Link::Url {
            url: "javascript:alert(1)".to_owned(),
        });
        shape_mut(&mut presentation, "ir-s1.arrow").link = Some(Link::Url {
            url: "file:///etc/passwd".to_owned(),
        });
        let emitted = emit(&presentation);
        let html = &emitted.files.text["slides/ir-s1.html"];
        assert!(!html.contains("javascript:"));
        assert!(!html.contains("file:"));
        assert!(html.contains("Read the "));
        let nodes: Vec<String> = emitted
            .diagnostics
            .iter()
            .filter(|d| d.code == codes::HTML6001)
            .map(|d| d.node.as_ref().unwrap().to_string())
            .collect();
        assert_eq!(nodes, ["ir-s1.body", "ir-s1.arrow"]);
        assert_eq!(emitted.files.lint(), vec![]);
    }

    #[test]
    fn quotes_the_first_120_utf16_units_of_a_dropped_url() {
        let mut presentation = full();
        let url = format!("file:///{}", "😀".repeat(100));
        shape_mut(&mut presentation, "ir-s1.arrow").link = Some(Link::Url { url });
        let emitted = emit(&presentation);
        let quoted: Vec<&DataValue> = emitted
            .diagnostics
            .iter()
            .filter(|d| d.code == codes::HTML6001)
            .map(|d| &d.data.as_ref().unwrap()["url"])
            .collect();
        let clipped = format!("file:///{}", "😀".repeat(56));
        assert_eq!(quoted, [&DataValue::Str(clipped)]);
    }

    #[test]
    fn prints_element_links_placeholders_hidden_elements_and_alt_text() {
        let mut presentation = full();
        let arrow = shape_mut(&mut presentation, "ir-s1.arrow");
        arrow.link = Some(Link::Action {
            action: LinkAction::Next,
        });
        arrow.hidden = Some(true);
        arrow.alt = Some("An arrow".to_owned());
        let html = slide_one_html(&presentation);
        assert!(html.contains(
            "<div data-el=\"ir-s1.arrow\" data-kind=\"shape\" data-link=\"action:next\" hidden role=\"img\" aria-label=\"An arrow\" style="
        ));
        assert!(html.contains("data-placeholder=\"body:1\""));
    }

    #[test]
    fn lists_the_elements_in_reading_order_stacked_by_z_index_in_paint_order_hidden_ones_last() {
        let mut presentation = full();
        let ids: Vec<IrId> = elements_mut(&mut presentation, 0)
            .iter()
            .map(|e| e.id().clone())
            .collect();
        assert!(ids.len() > 2);
        shape_mut(&mut presentation, ids[0].as_str()).hidden = Some(true);
        let order: Vec<IrId> = ids[1..].iter().rev().cloned().collect();
        built_mut(&mut presentation, 0).reading_order = order.clone();
        let html = slide_one_html(&presentation);
        let doc = Document::parse(&html);
        let main = common::elements(&doc)
            .into_iter()
            .find(|n| doc.tag_name(*n) == "main")
            .unwrap();
        let listed: Vec<(String, String)> = common::children(&doc, main)
            .into_iter()
            .filter_map(|child| {
                let id = attr(&doc, child, "data-el")?;
                let style = attr(&doc, child, "style").unwrap();
                Some((
                    id.to_owned(),
                    style.rsplit_once("z-index:").unwrap().1.to_owned(),
                ))
            })
            .collect();
        let mut expected: Vec<String> = order.iter().map(ToString::to_string).collect();
        expected.push(ids[0].to_string());
        assert_eq!(
            listed.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>(),
            expected
        );
        for (id, z) in &listed {
            let paint = ids.iter().position(|each| each.as_str() == id).unwrap();
            assert_eq!(z, &(paint + 1).to_string(), "{id}");
        }

        let mut in_paint_order = full();
        built_mut(&mut in_paint_order, 0).reading_order = ids;
        assert!(!slide_one_html(&in_paint_order).contains("z-index"));
    }

    #[test]
    fn names_an_element_with_text_through_a_group_so_its_text_stays_read() {
        let mut presentation = full();
        shape_mut(&mut presentation, "ir-s1.body").alt = Some("The body".to_owned());
        let html = slide_one_html(&presentation);
        let doc = Document::parse(&html);
        let body = element(&doc, "ir-s1.body");
        assert_eq!(attr(&doc, body, "role"), Some("group"));
        assert_eq!(attr(&doc, body, "aria-label"), Some("The body"));
    }

    #[test]
    fn names_an_element_whose_text_is_javascript_whitespace_as_an_image() {
        // U+FEFF is JavaScript whitespace and U+0085 is not, the reverse of Rust's `char::is_whitespace`.
        for (text, role) in [(" \u{feff}\u{a0}\u{3000}\t", "img"), ("\u{85}", "group")] {
            let mut presentation = full();
            let body = shape_mut(&mut presentation, "ir-s1.body");
            body.alt = Some("The body".to_owned());
            for paragraph in &mut body.text.as_mut().unwrap().paragraphs {
                let props = props_of(&mut paragraph.inlines[0]).clone();
                paragraph.inlines = vec![run(text, props)];
            }
            let html = slide_one_html(&presentation);
            let doc = Document::parse(&html);
            let body = element(&doc, "ir-s1.body");
            assert_eq!(attr(&doc, body, "role"), Some(role), "{text:?}");
        }
    }
}

mod runs {
    use super::*;

    #[test]
    fn prints_only_what_differs_from_css_initial_values() {
        let mut presentation = full();
        let props = props_of(&mut title_paragraph(&mut presentation).inlines[0]);
        props.underline = Decoration::Double;
        props.strike = Decoration::Single;
        props.highlight = Some(literal(rgb(1.0, 1.0, 0.0)));
        props.letter_spacing = pt(1.5);
        props.baseline = ratio(0.3);
        props.caps = Caps::Small;
        assert!(slide_one_html(&presentation).contains(
            "text-decoration-line:underline line-through;text-decoration-style:double;background-color:#ffff00;\
             letter-spacing:1.5pt;vertical-align:0.3em;font-variant-caps:small-caps"
        ));
    }

    #[test]
    fn leaves_a_missing_bold_face_unsynthesized_on_a_run_whose_source_app_draws_the_regular_one() {
        let mut presentation = full();
        assert!(!slide_one_html(&presentation).contains("font-synthesis"));
        let (text, props) = title_run(&mut presentation);
        assert!(props.synthesize_bold);
        props_of(&mut title_paragraph(&mut presentation).inlines[0]).synthesize_bold = false;
        // Every run of the box leaves it out: the box says so once.
        let html = slide_one_html(&presentation);
        let box_tag = opening(&title_text(&html)).to_owned();
        assert!(
            box_tag.contains(" data-sd-text ")
                && box_tag.ends_with(";font-synthesis-weight:none\">"),
            "{box_tag}"
        );
        assert_eq!(html.matches("font-synthesis-weight").count(), 1);
        title_paragraph(&mut presentation).inlines.push(run(
            " synthesized",
            ResolvedRunProps {
                synthesize_bold: true,
                ..props
            },
        ));
        let unsynthesized = tags_with(
            &slide_one_html(&presentation),
            "span",
            "font-synthesis-weight:none",
        );
        assert_eq!(unsynthesized.len(), 1);
        assert!(
            unsynthesized[0].ends_with(&format!(">{text}")),
            "{unsynthesized:?}"
        );
    }

    #[test]
    fn keeps_a_label_a_tab_and_hung_spaces_synthesized_in_a_box_whose_runs_all_leave_it_out() {
        let mut presentation = full();
        let body = shape_mut(&mut presentation, "ir-s1.body")
            .text
            .as_mut()
            .unwrap();
        let first = body.paragraphs[0].inlines[0].clone();
        let with_text = |text: &str| match first.clone() {
            Inline::Run { props, .. } => run(text, props),
            _ => panic!("run"),
        };
        body.paragraphs[0].inlines = vec![with_text("a"), Inline::Tab {}, with_text("b  ")];
        for paragraph in &mut body.paragraphs {
            for inline in &mut paragraph.inlines {
                if let Inline::Run { props, .. } | Inline::Field { props, .. } = inline {
                    props.synthesize_bold = false;
                }
            }
        }
        let html = slide_one_html(&presentation);
        assert_eq!(html.matches("font-synthesis-weight:none").count(), 1);
        let auto = tags_with(&html, "span", "font-synthesis-weight:auto");
        assert_eq!(auto.len(), 4, "{auto:?}");
        assert!(auto[0].starts_with("<span data-sd-bullet "));
        assert!(auto[1].ends_with(">\t"));
        assert!(
            auto[2].starts_with("<span style=\"position:absolute;") && auto[2].ends_with(">  ")
        );
        assert!(auto[3].starts_with("<span data-sd-bullet "));
    }

    #[test]
    fn keeps_overflowing_text_below_the_top_inset_of_a_box_that_runs_it_down() {
        let justify = |presentation: &Presentation| -> Vec<Option<String>> {
            let html = slide_one_html(presentation);
            let doc = Document::parse(&html);
            common::elements(&doc)
                .into_iter()
                .filter(|n| doc.tag_name(*n) == "div" && doc.has_attribute(*n, "data-sd-text"))
                .map(|n| {
                    attr(&doc, n, "style")
                        .unwrap()
                        .split(';')
                        .find_map(|d| d.strip_prefix("justify-content:").map(str::to_owned))
                })
                .collect()
        };
        let mut presentation = full();
        let before = justify(&presentation);
        assert!(before.contains(&Some("flex-end".to_owned())));
        assert!(!before.iter().flatten().any(|v| v.starts_with("safe")));
        let title = shape_mut(&mut presentation, "ir-s1.title")
            .text
            .as_mut()
            .unwrap();
        assert_eq!(
            (title.body.anchor, title.body.overflow),
            (Anchor::Bottom, Overflow::Spread)
        );
        title.body.overflow = Overflow::Below;
        let after = justify(&presentation);
        let changed: Vec<_> = after
            .iter()
            .zip(&before)
            .filter(|(a, b)| a != b)
            .map(|(a, _)| a.clone())
            .collect();
        assert_eq!(changed, [Some("safe flex-end".to_owned())]);
    }
}

mod text_declarations {
    use super::*;

    #[test]
    fn prints_a_run_in_its_paragraphs_face_and_its_boxs_colour_as_its_text_alone() {
        let text = title_text(&slide_one_html(&full()));
        assert!(
            text.contains(";color:var(--sd-color-tx2,#44546a)\">\n<p "),
            "{text}"
        );
        assert!(text.ends_with(">Presentation IR</p>\n</div>"), "{text}");
    }

    #[test]
    fn prints_only_the_face_a_run_does_not_inherit_from_its_paragraph() {
        let mut presentation = full();
        let (_, props) = title_run(&mut presentation);
        title_paragraph(&mut presentation).inlines = vec![
            run("a regular sentence", props.clone()),
            run(
                " Mono",
                ResolvedRunProps {
                    font: family("Liberation Mono"),
                    ..props.clone()
                },
            ),
            run(
                " big",
                ResolvedRunProps {
                    size: size(60.0),
                    ..props.clone()
                },
            ),
            run(
                " bold italic",
                ResolvedRunProps {
                    weight: 700,
                    italic: true,
                    ..props.clone()
                },
            ),
        ];
        let p = title_text(&slide_one_html(&presentation));
        assert!(p.contains(
            "<span style=\"font-family:&#x22;Liberation Mono&#x22;,monospace\"> Mono</span>"
        ));
        assert!(p.contains("<span style=\"font-size:60pt\"> big</span>"));
        assert!(
            p.contains("<span style=\"font-weight:700;font-style:italic\"> bold italic</span>")
        );
        // A regular run in an italic paragraph says so.
        title_paragraph(&mut presentation).inlines = vec![
            run(
                "an italic sentence",
                ResolvedRunProps {
                    italic: true,
                    ..props.clone()
                },
            ),
            run(" regular", props),
        ];
        assert!(title_text(&slide_one_html(&presentation)).contains(
            ";font-style:italic\">an italic sentence<span style=\"font-style:normal\"> regular</span></p>"
        ));
    }

    #[test]
    fn a_family_the_deck_ships_prints_the_families_its_rules_define_and_others_keep_their_stack() {
        use slidock_html::{EmitOptions, FontStacks, Motion, emit_documents_shipping};
        use slidock_ir::fonts::{BindingKind, font_key};
        let mut presentation = full();
        let (_, props) = title_run(&mut presentation);
        title_paragraph(&mut presentation).inlines = vec![
            run("a", props.clone()),
            run(
                " Arial",
                ResolvedRunProps {
                    font: family("A-rial"),
                    ..props.clone()
                },
            ),
            run(
                " Mono",
                ResolvedRunProps {
                    font: family("Liberation Mono"),
                    ..props.clone()
                },
            ),
        ];
        let mut arial = presentation.fonts[&font_key("Liberation Sans")].clone();
        arial.request.family = "Arial".to_owned();
        arial.binding.kind = BindingKind::Substitute;
        presentation.fonts.insert(font_key("Arial"), arial);
        // The paragraph takes the face most of its text draws: the Arial run's.
        let arial_stack = "font-family:&#x22;A-rial&#x22;,&#x22;Liberation Sans&#x22;,sans-serif\"";
        let mono = "<span style=\"font-family:&#x22;Liberation Mono&#x22;,monospace\"> Mono</span>";
        let plain = slide_one_html(&presentation);
        assert!(title_text(&plain).contains(arial_stack));
        let shipping = |stacks: &FontStacks| {
            emit_documents_shipping(
                &presentation,
                &EmitOptions::new(provenance()),
                stacks,
                Motion::Off,
            )
            .unwrap()
            .files
            .text["slides/ir-s1.html"]
                .clone()
        };
        let stacks = FontStacks::from([(
            font_key("Arial"),
            vec!["Liberation Sans".to_owned(), "DejaVu Sans".to_owned()],
        )]);
        let shipped = shipping(&stacks);
        let p = title_text(&shipped);
        assert!(
            p.contains(
                "font-family:&#x22;Liberation Sans&#x22;,&#x22;DejaVu Sans&#x22;,sans-serif\""
            ),
            "{p}"
        );
        assert!(!p.contains("A-rial"));
        assert!(p.contains(mono));
        // Negative control: with no stacks the output is the plain one, byte for byte.
        assert_eq!(shipping(&FontStacks::new()), plain);
    }

    #[test]
    fn an_emoji_family_prints_the_platform_emoji_stack_shipping_or_not() {
        use slidock_html::{EmitOptions, FontStacks, Motion, emit_documents_shipping};
        use slidock_ir::fonts::{BindingKind, GenericFamily, font_key};
        let mut presentation = full();
        let (_, props) = title_run(&mut presentation);
        title_paragraph(&mut presentation).inlines = vec![
            run("a", props.clone()),
            run(
                "\u{1f600}",
                ResolvedRunProps {
                    font: family("AppleColorEmoji"),
                    ..props.clone()
                },
            ),
        ];
        let mut emoji = presentation.fonts[&font_key("Liberation Sans")].clone();
        emoji.request.family = "AppleColorEmoji".to_owned();
        emoji.binding.kind = BindingKind::Generic;
        emoji.binding.family = "emoji".to_owned();
        emoji.binding.generic = GenericFamily::Emoji;
        presentation
            .fonts
            .insert(font_key("AppleColorEmoji"), emoji);
        let stack = "font-family:&#x22;Apple Color Emoji&#x22;,&#x22;Segoe UI Emoji&#x22;,&#x22;Segoe UI \
                     Symbol&#x22;,&#x22;Noto Color Emoji&#x22;,emoji\"";
        let plain = slide_one_html(&presentation);
        assert!(title_text(&plain).contains(stack), "{}", title_text(&plain));
        // A deck that ships its fonts prints the same stack: no shipped face draws the emoji.
        let stacks = FontStacks::from([(
            font_key("AppleColorEmoji"),
            vec!["Liberation Sans".to_owned()],
        )]);
        let shipped = emit_documents_shipping(
            &presentation,
            &EmitOptions::new(provenance()),
            &stacks,
            Motion::Off,
        )
        .unwrap()
        .files
        .text["slides/ir-s1.html"]
            .clone();
        assert!(title_text(&shipped).contains(stack));
        // Negative control: bound to sans-serif, the family prints its own stack.
        let entry = presentation
            .fonts
            .get_mut(&font_key("AppleColorEmoji"))
            .unwrap();
        entry.binding.generic = GenericFamily::SansSerif;
        entry.binding.family = "Liberation Sans".to_owned();
        let text = title_text(&slide_one_html(&presentation));
        assert!(
            text.contains("&#x22;AppleColorEmoji&#x22;,&#x22;Liberation Sans&#x22;,sans-serif")
        );
        assert!(!text.contains("Segoe UI Emoji"));
    }

    #[test]
    fn prints_a_bullets_face_and_colour_only_where_they_differ_from_its_paragraphs() {
        let html = slide_one_html(&full());
        let labels: Vec<String> = tags_with(&html, "span", "data-sd-bullet")
            .iter()
            .map(|t| opening(t).to_owned())
            .collect();
        assert_eq!(
            labels[0],
            "<span data-sd-bullet aria-hidden=\"true\" style=\"display:inline-block;min-width:18pt;text-indent:0pt;color:#335593\">"
        );
        assert!(labels[1].contains(
            ";text-indent:0pt;font-family:&#x22;Liberation Mono&#x22;,monospace;font-size:16pt\""
        ));
    }

    #[test]
    fn sets_the_colour_most_of_the_text_draws_in_on_the_box_and_prints_each_other_colour_on_its_run()
     {
        let mut presentation = full();
        let (_, props) = title_run(&mut presentation);
        title_paragraph(&mut presentation).inlines = vec![
            run(
                "Dark ",
                ResolvedRunProps {
                    color: literal(black()),
                    ..props.clone()
                },
            ),
            run("the longest run", props.clone()),
            run(
                " underlined",
                ResolvedRunProps {
                    underline: Decoration::Single,
                    ..props
                },
            ),
        ];
        let text = title_text(&slide_one_html(&presentation));
        let tag = opening(&text);
        assert!(
            tag.contains(" data-sd-text ")
                && tag.ends_with(";color:var(--sd-color-tx2,#44546a)\">"),
            "{tag}"
        );
        // A decoration draws in its run's colour, inherited here.
        assert!(text.contains(
            "<span style=\"color:#000000\">Dark </span>the longest run<span style=\"text-decoration-line:underline\"> underlined</span>"
        ));
    }

    #[test]
    fn weighs_the_colours_and_faces_of_runs_by_the_utf16_length_of_their_text() {
        // Each pair has the black regular run longer in UTF-8 bytes (`ééé`) or in chars (`abc`), and the bold run in
        // the box colour longer in UTF-16 units.
        for (shorter, longer) in [("ééé", "abcde"), ("abc", "😀😀")] {
            let mut presentation = full();
            let (_, props) = title_run(&mut presentation);
            title_paragraph(&mut presentation).inlines = vec![
                run(
                    shorter,
                    ResolvedRunProps {
                        color: literal(black()),
                        weight: 400,
                        ..props.clone()
                    },
                ),
                run(
                    longer,
                    ResolvedRunProps {
                        weight: 700,
                        ..props
                    },
                ),
            ];
            let html = slide_one_html(&presentation);
            let text = title_text(&html);
            assert!(
                opening(&text).ends_with(";color:var(--sd-color-tx2,#44546a)\">"),
                "{longer}: {text}"
            );
            let p = first_p(&html);
            assert!(
                opening(&p).ends_with(";font-weight:700\">"),
                "{longer}: {p}"
            );
        }
    }

    #[test]
    fn keeps_a_link_and_a_field_in_their_spans_when_they_print_nothing_of_their_own() {
        let mut presentation = full();
        let (_, props) = title_run(&mut presentation);
        title_paragraph(&mut presentation).inlines = vec![
            run("see ", props.clone()),
            run(
                "spec",
                ResolvedRunProps {
                    link: Some(Link::Url {
                        url: "https://example.com/spec".to_owned(),
                    }),
                    ..props.clone()
                },
            ),
            Inline::Field {
                field: FieldKind::SlideNumber,
                cached: "1".to_owned(),
                props,
                line_height: None,
                leading_skew: None,
            },
        ];
        assert!(title_text(&slide_one_html(&presentation)).contains(
            ">see <a href=\"https://example.com/spec\" style=\"color:inherit;text-decoration:inherit\"><span>spec</span></a>\
             <span data-field=\"slideNumber\">1</span></p>"
        ));
    }

    #[test]
    fn keeps_the_edge_between_two_runs_that_print_alike_as_text_shapes_apart_there() {
        let mut presentation = full();
        let (_, props) = title_run(&mut presentation);
        let big = ResolvedRunProps {
            size: size(60.0),
            ..props.clone()
        };
        title_paragraph(&mut presentation).inlines = vec![
            run("Presen", props.clone()),
            run("", props.clone()),
            run("tation", props.clone()),
            run(" I", big.clone()),
            run("R", big),
            run("!", props),
        ];
        assert!(title_text(&slide_one_html(&presentation)).contains(
            ">Presen<span>tation</span><span style=\"font-size:60pt\"> I</span><span style=\"font-size:60pt\">R</span>!</p>"
        ));
    }

    #[test]
    fn keeps_each_run_of_a_paragraph_a_break_splits_in_its_span() {
        let mut presentation = full();
        let (_, props) = title_run(&mut presentation);
        title_paragraph(&mut presentation).inlines = vec![
            run("one", props.clone()),
            brk(),
            run("", props.clone()),
            run("two", props.clone()),
        ];
        assert!(
            title_text(&slide_one_html(&presentation))
                .contains("><span>one</span><br><span>two</span></p>")
        );
        title_paragraph(&mut presentation).inlines = vec![run("one line", props)];
        assert!(title_text(&slide_one_html(&presentation)).contains(">one line</p>"));
    }

    #[test]
    fn leaves_out_the_paragraph_values_css_starts_from() {
        let mut presentation = full();
        let open = |presentation: &Presentation| {
            opening(&first_p(&slide_one_html(presentation))).to_owned()
        };
        assert!(open(&presentation).ends_with(
            " style=\"line-height:1.2;font-size:40pt;font-family:&#x22;Liberation Serif&#x22;,serif\">"
        ));
        let paragraph = title_paragraph(&mut presentation);
        paragraph.props.rtl = true;
        paragraph.props.align = Align::Right;
        let tag = open(&presentation);
        assert!(
            tag.contains(" style=\"line-height:") && tag.ends_with(";direction:rtl\">"),
            "{tag}"
        );
        title_paragraph(&mut presentation).props.align = Align::Left;
        let tag = open(&presentation);
        assert!(
            tag.contains(" style=\"text-align:left;line-height:")
                && tag.ends_with(";direction:rtl\">"),
            "{tag}"
        );
    }

    #[test]
    fn sets_the_tab_size_most_paragraphs_with_a_tab_use_on_the_box_and_prints_each_other_on_its_paragraph()
     {
        let mut presentation = full();
        let (_, props) = title_run(&mut presentation);
        let body = shape_mut(&mut presentation, "ir-s1.title")
            .text
            .as_mut()
            .unwrap();
        body.paragraphs[0].inlines = vec![
            run("a", props.clone()),
            Inline::Tab {},
            run("b", props.clone()),
        ];
        let paragraph = body.paragraphs[0].clone();
        let mut wider = paragraph.clone();
        wider.props.tab_size = size(36.0);
        let mut without = paragraph.clone();
        without.inlines = vec![run("no tab", props)];
        without.props.tab_size = size(18.0);
        body.paragraphs.extend([paragraph, wider, without]);
        let text = title_text(&slide_one_html(&presentation));
        let tag = opening(&text);
        assert!(
            tag.contains(" data-sd-text ") && tag.ends_with(";tab-size:72pt\">"),
            "{tag}"
        );
        let tabs: Vec<Option<String>> = tags_with(&text, "p", "")
            .iter()
            .map(|p| {
                opening(p).split(';').find_map(|d| {
                    d.strip_prefix("tab-size:")
                        .map(|v| v.trim_end_matches("\">").to_owned())
                })
            })
            .collect();
        assert_eq!(tabs, [None, None, Some("36pt".to_owned()), None]);
    }
}

mod text_boxes {
    use super::*;

    /// The title element and the element that holds its text.
    fn title_of(doc: &Document) -> (NodeId, NodeId) {
        let title = element(doc, "ir-s1.title");
        let text_box = std::iter::once(title)
            .chain(common::descendants(doc, title))
            .find(|n| doc.has_attribute(*n, "data-sd-text"))
            .unwrap();
        (title, text_box)
    }

    #[test]
    fn makes_a_shape_with_no_geometry_layer_its_own_text_box_its_insets_as_padding_inside_its_frame()
     {
        let html = slide_one_html(&full());
        let doc = Document::parse(&html);
        let (title, text_box) = title_of(&doc);
        assert_eq!(text_box, title);
        assert_eq!(
            attr(&doc, title, "style").unwrap(),
            "position:absolute;left:36pt;top:20pt;width:648pt;height:60pt;box-sizing:border-box;padding:3.6pt 7.2pt;\
             display:flex;flex-direction:column;justify-content:flex-end;white-space:pre-wrap;overflow-wrap:break-word;\
             color:var(--sd-color-tx2,#44546a)"
        );
        let tags: Vec<&str> = common::children(&doc, title)
            .iter()
            .map(|c| doc.tag_name(*c))
            .collect();
        assert_eq!(tags, ["p"]);
    }

    #[test]
    fn keeps_a_text_box_of_its_own_over_a_geometry_layer_on_a_hidden_shape_and_under_a_text_turn() {
        let mut geometry = full();
        let arrow = shape_mut(&mut geometry, "ir-s1.arrow").clone();
        let title = shape_mut(&mut geometry, "ir-s1.title");
        title.geometry = arrow.geometry;
        title.fill = arrow.fill;
        title.stroke = arrow.stroke;
        let html = slide_one_html(&geometry);
        let doc = Document::parse(&html);
        let (title, text_box) = title_of(&doc);
        let tags: Vec<&str> = common::descendants(&doc, title)
            .iter()
            .take(2)
            .map(|n| doc.tag_name(*n))
            .collect();
        assert_eq!(tags, ["svg", "defs"]);
        assert!(
            attr(&doc, text_box, "style")
                .unwrap()
                .starts_with("position:absolute;inset:0;padding:3.6pt 7.2pt;display:flex;")
        );

        let mut hidden = full();
        shape_mut(&mut hidden, "ir-s1.title").hidden = Some(true);
        let html = slide_one_html(&hidden);
        let doc = Document::parse(&html);
        let (title, text_box) = title_of(&doc);
        assert_ne!(text_box, title);
        assert_eq!(attr(&doc, title, "hidden"), Some(""));

        let mut turned = full();
        shape_mut(&mut turned, "ir-s1.title")
            .text
            .as_mut()
            .unwrap()
            .rotation = TextRotation::Half;
        let html = slide_one_html(&turned);
        let doc = Document::parse(&html);
        let (title, text_box) = title_of(&doc);
        assert_ne!(text_box, title);
        assert!(
            attr(&doc, text_box, "style")
                .unwrap()
                .ends_with(";scale:-1 -1")
        );
    }

    #[test]
    fn grows_an_autosized_shape_with_its_text_from_the_top() {
        let mut presentation = full();
        let title = shape_mut(&mut presentation, "ir-s1.title");
        title.text.as_mut().unwrap().body.autofit = ResolvedAutofit::Resize {
            width: true,
            height: false,
            anchor_x: AnchorX::Left,
        };
        let html = slide_one_html(&presentation);
        let doc = Document::parse(&html);
        assert_eq!(
            attr(&doc, title_of(&doc).0, "style").unwrap(),
            "position:absolute;left:36pt;top:20pt;width:max-content;height:60pt;box-sizing:border-box;\
             padding:3.6pt 7.2pt;display:flex;flex-direction:column;white-space:pre-wrap;overflow-wrap:break-word;\
             color:var(--sd-color-tx2,#44546a)"
        );
        shape_mut(&mut presentation, "ir-s1.title").hidden = Some(true);
        let html = slide_one_html(&presentation);
        let doc = Document::parse(&html);
        assert!(
            attr(&doc, title_of(&doc).1, "style")
                .unwrap()
                .starts_with("position:relative;padding:3.6pt 7.2pt;display:flex;")
        );
    }

    #[test]
    fn leaves_out_the_padding_of_a_box_with_no_insets_and_the_alignment_of_top_anchored_text() {
        let mut presentation = full();
        let text = shape_mut(&mut presentation, "ir-s1.title")
            .text
            .as_mut()
            .unwrap();
        text.body.insets = Insets {
            l: pt(0.0),
            t: pt(0.0),
            r: pt(0.0),
            b: pt(0.0),
        };
        text.body.anchor = Anchor::Top;
        let html = slide_one_html(&presentation);
        let doc = Document::parse(&html);
        let style = attr(&doc, title_of(&doc).0, "style").unwrap();
        assert!(
            !style.contains("padding")
                && !style.contains("box-sizing")
                && !style.contains("justify-content")
        );
        assert!(
            style.contains(";height:60pt;display:flex;flex-direction:column;white-space:pre-wrap;")
        );
    }
}

mod box_declarations {
    use super::*;

    #[test]
    fn writes_four_sides_as_the_shortest_padding_value() {
        assert_eq!(sides(1.0, 1.0, 1.0, 1.0).unwrap(), "1pt");
        assert_eq!(sides(1.0, 2.0, 1.0, 2.0).unwrap(), "1pt 2pt");
        assert_eq!(sides(1.0, 2.0, 3.0, 2.0).unwrap(), "1pt 2pt 3pt");
        assert_eq!(sides(1.0, 2.0, 1.0, 3.0).unwrap(), "1pt 2pt 1pt 3pt");
        assert_eq!(sides(0.0, 0.0, 0.0, 0.0).unwrap(), "0pt");
    }

    #[test]
    fn fills_a_plain_rectangle_and_a_background_with_background_color_and_places_full_box_layers_by_inset()
     {
        let emitted = emit(&full());
        let html: Vec<&String> = emitted
            .files
            .text
            .iter()
            .filter(|(p, _)| p.ends_with(".html"))
            .map(|(_, t)| t)
            .collect();
        let all = html
            .iter()
            .map(|t| t.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(all.contains("background-color:"));
        for bad in [
            ";background:#",
            "\"background:#",
            ";background:var(",
            "\"background:var(",
        ] {
            assert!(!all.contains(bad), "{bad}");
        }
        assert!(!all.contains("height:auto"));
        assert!(!all.contains("justify-content:flex-start"));
        assert!(!all.contains("border-spacing"));
        // A replaced element keeps its size; a box takes it from its insets.
        for text in html {
            let doc = Document::parse(text);
            for node in common::elements(&doc) {
                let style = attr(&doc, node, "style").unwrap_or("");
                match doc.tag_name(node) {
                    "img" => assert!(!style.contains("display:block")),
                    "div" => assert!(!style.contains("left:0pt;top:0pt;width:100%;height:100%")),
                    _ => {}
                }
            }
        }
    }
}

mod lists {
    use super::*;

    const LIST: &str = "margin:0;padding:0;list-style:none;display:flex;flex-direction:column";

    struct Parts {
        numbered: ResolvedParagraph,
        dashed: ResolvedParagraph,
        blank: ResolvedParagraph,
        plain: ResolvedParagraph,
    }

    fn parts(presentation: &mut Presentation) -> Parts {
        let body = shape_mut(presentation, "ir-s1.body").text.as_ref().unwrap();
        let numbered = body.paragraphs[0].clone();
        let dashed = body.paragraphs[1].clone();
        let mut blank = numbered.clone();
        let Inline::Run { props, .. } = &numbered.inlines[0] else {
            panic!("run")
        };
        blank.inlines = vec![run("", props.clone())];
        let mut plain = numbered.clone();
        plain.props.bullet = ResolvedBullet::None {};
        Parts {
            numbered,
            dashed,
            blank,
            plain,
        }
    }

    fn set(presentation: &mut Presentation, paragraphs: Vec<ResolvedParagraph>) {
        shape_mut(presentation, "ir-s1.body")
            .text
            .as_mut()
            .unwrap()
            .paragraphs = paragraphs;
    }

    /// The body's text box in a parsed slide.
    fn box_of(doc: &Document) -> NodeId {
        let body = element(doc, "ir-s1.body");
        std::iter::once(body)
            .chain(common::descendants(doc, body))
            .find(|n| doc.has_attribute(*n, "data-sd-text"))
            .unwrap()
    }

    /// The text box's children: each tag, and for a list its start and its items' tags.
    fn blocks(presentation: &Presentation) -> Vec<String> {
        let html = slide_one_html(presentation);
        let doc = Document::parse(&html);
        common::children(&doc, box_of(&doc))
            .into_iter()
            .map(|child| {
                let tag = doc.tag_name(child);
                if tag == "p" {
                    return "p".to_owned();
                }
                let start =
                    attr(&doc, child, "start").map_or(String::new(), |s| format!(" start={s}"));
                let items: Vec<&str> = common::children(&doc, child)
                    .iter()
                    .map(|i| doc.tag_name(*i))
                    .collect();
                format!("{tag}{start}({})", items.join(","))
            })
            .collect()
    }

    #[test]
    fn prints_labelled_paragraphs_as_the_items_of_a_list_each_with_its_paragraphs_attributes_and_its_label()
     {
        let mut presentation = full();
        let html = slide_one_html(&presentation);
        let doc = Document::parse(&html);
        let list = common::children(&doc, box_of(&doc))[0];
        assert_eq!(
            (doc.tag_name(list), attr(&doc, list, "style")),
            ("ol", Some(LIST))
        );
        let items = common::children(&doc, list);
        for item in &items {
            assert_eq!(doc.tag_name(*item), "li");
            assert_eq!(
                attr(&doc, common::children(&doc, *item)[0], "data-sd-bullet"),
                Some("")
            );
        }
        let item_attrs: Vec<Vec<(String, String)>> = items
            .iter()
            .map(|i| {
                doc.attrs(*i)
                    .iter()
                    .map(|a| (a.local().to_owned(), a.value.clone()))
                    .collect()
            })
            .collect();
        // Unlabelled, the same paragraphs print as <p>s with the same attributes.
        let unlabelled: Vec<ResolvedParagraph> = {
            let body = shape_mut(&mut presentation, "ir-s1.body")
                .text
                .as_ref()
                .unwrap();
            body.paragraphs
                .iter()
                .map(|p| {
                    let mut p = p.clone();
                    p.props.bullet = ResolvedBullet::None {};
                    p
                })
                .collect()
        };
        set(&mut presentation, unlabelled);
        let html = slide_one_html(&presentation);
        let doc = Document::parse(&html);
        let paragraphs = common::children(&doc, box_of(&doc));
        let tags: Vec<&str> = paragraphs.iter().map(|p| doc.tag_name(*p)).collect();
        assert_eq!(tags, ["p", "p"]);
        let paragraph_attrs: Vec<Vec<(String, String)>> = paragraphs
            .iter()
            .map(|i| {
                doc.attrs(*i)
                    .iter()
                    .map(|a| (a.local().to_owned(), a.value.clone()))
                    .collect()
            })
            .collect();
        assert_eq!(item_attrs, paragraph_attrs);
    }

    #[test]
    fn makes_a_list_of_each_run_of_labelled_paragraphs_with_the_empty_ones_between_them() {
        let mut presentation = full();
        let Parts {
            numbered,
            dashed,
            blank,
            plain,
        } = parts(&mut presentation);
        set(
            &mut presentation,
            vec![
                blank.clone(),
                dashed.clone(),
                blank.clone(),
                numbered.clone(),
                plain,
                dashed.clone(),
                blank,
            ],
        );
        assert_eq!(
            blocks(&presentation),
            ["p", "ol(li,li,li)", "p", "ul(li)", "p"]
        );
        set(
            &mut presentation,
            vec![
                dashed,
                ResolvedParagraph {
                    level: 1,
                    ..numbered
                },
            ],
        );
        assert_eq!(blocks(&presentation), ["ul(li,li)"]);
    }

    #[test]
    fn prints_the_alignment_of_a_right_to_left_item_which_a_browser_may_take_from_its_lists_start_side()
     {
        let mut presentation = full();
        let numbered = parts(&mut presentation).numbered;
        let mut item = |rtl: bool, align: Align| {
            let mut paragraph = numbered.clone();
            paragraph.props.rtl = rtl;
            paragraph.props.align = align;
            set(&mut presentation, vec![paragraph]);
            let html = slide_one_html(&presentation);
            let doc = Document::parse(&html);
            let list = common::children(&doc, box_of(&doc))[0];
            attr(&doc, common::children(&doc, list)[0], "style")
                .unwrap()
                .to_owned()
        };
        let style = item(true, Align::Right);
        assert!(style.starts_with("text-align:right;") && style.ends_with(";direction:rtl"));
        let style = item(true, Align::Left);
        assert!(style.starts_with("text-align:left;") && style.ends_with(";direction:rtl"));
        assert!(!item(false, Align::Left).contains("text-align"));
        assert!(item(false, Align::Right).starts_with("text-align:right;"));
    }

    #[test]
    fn starts_a_numbered_list_at_the_number_its_first_label_shows() {
        for (scheme, text, start) in [
            (AutoNumScheme::ArabicParenR, "12)", " start=12"),
            (AutoNumScheme::ArabicPeriod, "0X1F.", " start=31"),
            (AutoNumScheme::ArabicPlain, "0o17", " start=15"),
            (AutoNumScheme::ArabicParenBoth, "(0b101)", " start=5"),
            (AutoNumScheme::ArabicPeriod, "1e3.", " start=1000"),
            (AutoNumScheme::ArabicPeriod, "-0x1.", " start=NaN"),
            (AutoNumScheme::AlphaLcParenBoth, "(bb)", " start=28"),
            (AutoNumScheme::RomanUcPeriod, "XIV.", " start=14"),
            (AutoNumScheme::RomanLcParenR, "i)", ""),
        ] {
            let mut presentation = full();
            let Parts {
                numbered, dashed, ..
            } = parts(&mut presentation);
            let mut relabelled = numbered.clone();
            let ResolvedBullet::Label {
                text: label, auto, ..
            } = &mut relabelled.props.bullet
            else {
                panic!("label")
            };
            *label = text.to_owned();
            *auto = Some(AutoNumbering {
                scheme,
                start_at: 1,
            });
            set(
                &mut presentation,
                vec![ResolvedParagraph { level: 1, ..dashed }, relabelled],
            );
            assert_eq!(
                blocks(&presentation),
                [format!("ol{start}(li,li)")],
                "{text}"
            );
        }
    }
}

mod document_roots {
    use super::*;

    #[test]
    fn lay_text_out_with_unrounded_glyph_advances_so_lines_break_alike_at_every_zoom() {
        let emitted = emit(&full());
        let documents: Vec<&String> = emitted
            .files
            .text
            .iter()
            .filter(|(p, _)| p.ends_with(".html"))
            .map(|(_, t)| t)
            .collect();
        assert!(documents.len() > 1);
        for html in documents {
            let doc = Document::parse(html);
            let main = common::elements(&doc)
                .into_iter()
                .find(|n| doc.tag_name(*n) == "main")
                .unwrap();
            assert!(
                attr(&doc, main, "style")
                    .unwrap()
                    .ends_with(";text-rendering:geometricPrecision")
            );
        }
    }
}

mod paragraph_lines {
    use super::*;

    /// The opening tag of the span whose text is `text`.
    fn span_with_text<'h>(html: &'h str, text: &str) -> &'h str {
        let end = html.find(&format!(">{text}</span>")).unwrap();
        let start = html[..end].rfind("<span").unwrap();
        &html[start..=end]
    }

    #[test]
    fn sizes_the_paragraph_by_its_smallest_run_so_each_line_is_as_tall_as_its_own_largest_run() {
        let mut presentation = full();
        let (_, props) = title_run(&mut presentation);
        title_paragraph(&mut presentation).inlines.extend([
            brk(),
            run(
                "big",
                ResolvedRunProps {
                    size: size(60.0),
                    ..props
                },
            ),
        ]);
        let p = first_p(&slide_one_html(&presentation));
        assert!(p.contains("font-size:40pt;font-family"));
        assert!(p.contains("font-size:60pt"));
    }

    #[test]
    fn gives_the_paragraphs_own_line_box_the_face_of_that_run_so_a_bold_or_italic_line_keeps_its_height()
     {
        let mut presentation = full();
        let open = |presentation: &Presentation| {
            opening(&first_p(&slide_one_html(presentation))).to_owned()
        };
        assert!(
            !open(&presentation).contains("font-weight")
                && !open(&presentation).contains("font-style")
        );
        let props = props_of(&mut title_paragraph(&mut presentation).inlines[0]);
        props.weight = 700;
        props.italic = true;
        let tag = open(&presentation);
        assert!(
            tag.contains(";font-family:") && tag.ends_with(";font-weight:700;font-style:italic\">"),
            "{tag}"
        );
        // A bold lead-in on a regular sentence: the face most of the text uses.
        let (_, props) = title_run(&mut presentation);
        title_paragraph(&mut presentation).inlines.push(run(
            "a longer regular run",
            ResolvedRunProps {
                weight: 400,
                italic: false,
                ..props
            },
        ));
        let tag = open(&presentation);
        assert!(
            !tag.contains("font-weight") && !tag.contains("font-style"),
            "{tag}"
        );
    }

    #[test]
    fn prints_a_runs_own_line_height_and_gives_the_paragraph_its_floor_runs() {
        let mut presentation = full();
        let (text, props) = title_run(&mut presentation);
        let taller = Inline::Run {
            text: " taller".to_owned(),
            props: props.clone(),
            line_height: Some(LineHeight::Ratio { value: ratio(1.3) }),
            leading_skew: None,
        };
        title_paragraph(&mut presentation).inlines.push(taller);
        let p = first_p(&slide_one_html(&presentation));
        assert!(
            span_with_text(&p, " taller")
                .split(['"', ';'])
                .any(|d| d == "line-height:1.3"),
            "{p}"
        );
        let _ = text;
        // A short taller lead-in on a longer sentence: the sentence's own line is the paragraph's line box.
        title_paragraph(&mut presentation).inlines = vec![
            run(
                "Lead:",
                ResolvedRunProps {
                    weight: 700,
                    ..props.clone()
                },
            ),
            Inline::Run {
                text: " a longer sentence".to_owned(),
                props: ResolvedRunProps {
                    weight: 400,
                    ..props
                },
                line_height: Some(LineHeight::Ratio { value: ratio(1.1) }),
                leading_skew: None,
            },
        ];
        let p = first_p(&slide_one_html(&presentation));
        assert!(opening(&p).contains("line-height:1.1;"));
        assert!(
            span_with_text(&p, "Lead:")
                .split(['"', ';'])
                .any(|d| d == "line-height:1.2"),
            "{p}"
        );
        assert!(p.contains("</span> a longer sentence</p>"));
    }

    #[test]
    fn shifts_a_superscript_by_a_fraction_of_its_own_size_whatever_the_line_height() {
        let mut presentation = full();
        let (text, props) = title_run(&mut presentation);
        let paragraph = title_paragraph(&mut presentation);
        paragraph.props.line_height = LineHeight::Ratio { value: ratio(1.5) };
        paragraph.inlines = vec![
            run(
                &text,
                ResolvedRunProps {
                    baseline: ratio(0.3),
                    ..props.clone()
                },
            ),
            run(
                &text,
                ResolvedRunProps {
                    baseline: ratio(-0.25),
                    ..props
                },
            ),
        ];
        let p = first_p(&slide_one_html(&presentation));
        let aligns: Vec<&str> = p
            .split(['"', ';'])
            .filter(|d| d.starts_with("vertical-align:"))
            .collect();
        assert_eq!(aligns, ["vertical-align:0.3em", "vertical-align:-0.25em"]);
    }

    #[test]
    fn raises_the_box_of_a_run_by_half_its_leading_skew_and_lowers_its_text_back() {
        let mut presentation = full();
        let (text, props) = title_run(&mut presentation);
        let paragraph = title_paragraph(&mut presentation);
        paragraph.props.line_height = LineHeight::Ratio { value: ratio(1.4) };
        paragraph.props.leading_skew = Some(ratio(0.2));
        paragraph.inlines = vec![skewed(
            &text,
            ResolvedRunProps {
                baseline: ratio(0.3),
                ..props.clone()
            },
            0.2,
        )];
        let p = first_p(&slide_one_html(&presentation));
        let span = span_with_text(&p, &text);
        assert!(
            span.contains("line-height:1.4;")
                && span.ends_with(";vertical-align:0.4em;position:relative;top:0.1em\">"),
            "{span}"
        );
        // The line a final break opens in a smaller mark.
        let paragraph = title_paragraph(&mut presentation);
        paragraph.inlines = vec![skewed(&text, props.clone(), 0.2), brk()];
        paragraph.end_props = Some(ResolvedRunProps {
            size: size(15.0),
            ..props
        });
        assert!(first_p(&slide_one_html(&presentation)).contains(
            "<span style=\"font-size:15pt;line-height:1.4;vertical-align:0.1em;position:relative;top:0.1em\"><br></span>"
        ));
    }

    #[test]
    fn sizes_an_empty_line_between_two_breaks_as_a_paragraph_marks_line_where_spans_give_the_lines_their_height()
     {
        let mut presentation = full();
        let (text, props) = title_run(&mut presentation);
        let bold = ResolvedRunProps {
            weight: 700,
            ..props.clone()
        };
        let paragraph = title_paragraph(&mut presentation);
        paragraph.props.line_height = LineHeight::Ratio { value: ratio(1.4) };
        paragraph.props.leading_skew = Some(ratio(0.2));
        // A bold line in its own face: each line takes its height from its spans, and the paragraph's own is 0.
        paragraph.inlines = vec![
            skewed(&text, props.clone(), 0.2),
            brk(),
            Inline::Break {
                props: Some(ResolvedRunProps {
                    size: size(15.0),
                    ..props.clone()
                }),
            },
            skewed("next", bold, 0.2),
        ];
        let p = first_p(&slide_one_html(&presentation));
        assert!(opening(&p).contains("line-height:0;"), "{p}");
        assert!(
            p.contains(
                "</span><br><span style=\"font-size:15pt;line-height:1.4;vertical-align:0.1em;position:relative;top:0.1em\"><br></span><span "
            ),
            "{p}"
        );
    }

    #[test]
    fn raises_a_runs_box_by_at_most_half_its_line_and_still_lowers_its_text_by_half_the_skew() {
        let mut presentation = full();
        let (text, props) = title_run(&mut presentation);
        // A line a tenth of a single one with the leading of a whole one above the text: a box raised by the half
        // skew, 0.5 em, would leave the strut below its 0.2 em line and make the line taller.
        let paragraph = title_paragraph(&mut presentation);
        paragraph.props.line_height = LineHeight::Ratio { value: ratio(0.2) };
        paragraph.props.leading_skew = Some(ratio(1.0));
        paragraph.inlines = vec![skewed(&text, props.clone(), 1.0), brk()];
        paragraph.end_props = Some(ResolvedRunProps {
            size: size(15.0),
            ..props.clone()
        });
        let p = first_p(&slide_one_html(&presentation));
        assert!(
            span_with_text(&p, &text)
                .ends_with("line-height:0.2;vertical-align:0.1em;position:relative;top:0.5em\">"),
            "{p}"
        );
        assert!(
            p.contains(
                "<span style=\"font-size:15pt;line-height:0.2;vertical-align:0.1em;position:relative;top:0.5em\"><br></span>"
            ),
            "{p}"
        );
        // A line in points is halved in the run's own size.
        let size_pt = props.size.to_f64();
        let paragraph = title_paragraph(&mut presentation);
        paragraph.props.line_height = LineHeight::Pt {
            value: size(0.3 * size_pt),
        };
        paragraph.inlines = vec![skewed(&text, props, 1.0), brk()];
        let p = first_p(&slide_one_html(&presentation));
        assert!(
            span_with_text(&p, &text)
                .ends_with(";vertical-align:0.15em;position:relative;top:0.5em\">"),
            "{p}"
        );
    }

    #[test]
    fn lifts_a_skewed_paragraph_as_a_whole_where_each_line_draws_one_run_in_its_face_and_line() {
        let mut presentation = full();
        let (text, props) = title_run(&mut presentation);
        let paragraph = title_paragraph(&mut presentation);
        paragraph.props.line_height = LineHeight::Ratio { value: ratio(1.2) };
        paragraph.props.leading_skew = Some(ratio(-0.08));
        paragraph.end_props = Some(props.clone());
        let lift = ";position:relative;top:-0.04em\">";
        paragraph.inlines = vec![skewed(&text, props.clone(), -0.08)];
        let p = first_p(&slide_one_html(&presentation));
        assert!(
            p.starts_with("<p ") && p.contains("\"line-height:1.2;"),
            "{p}"
        );
        assert!(p.ends_with(&format!("{lift}Presentation IR</p>")), "{p}");
        // A run on a line of its own prints no line, whatever else it sets; nor does an empty paragraph's mark.
        title_paragraph(&mut presentation).inlines = vec![
            skewed(&text, props.clone(), -0.08),
            brk(),
            skewed(
                "next",
                ResolvedRunProps {
                    underline: Decoration::Single,
                    ..props
                },
                -0.08,
            ),
        ];
        assert!(first_p(&slide_one_html(&presentation)).ends_with(&format!(
            "{lift}<span>Presentation IR</span><br><span style=\"text-decoration-line:underline\">next</span></p>"
        )));
        title_paragraph(&mut presentation).inlines = Vec::new();
        assert!(first_p(&slide_one_html(&presentation)).ends_with(&format!("{lift}<br></p>")));
    }

    #[test]
    fn keeps_the_skew_on_each_span_where_runs_meet_in_a_line_or_a_run_or_label_would_size_the_line_apart()
     {
        let mut presentation = full();
        let (text, props) = title_run(&mut presentation);
        let paragraph = title_paragraph(&mut presentation);
        paragraph.props.line_height = LineHeight::Ratio { value: ratio(1.2) };
        paragraph.props.leading_skew = Some(ratio(-0.08));
        paragraph.end_props = Some(props.clone());
        let skew = |text: &str, props: &ResolvedRunProps| skewed(text, props.clone(), -0.08);
        let spans = |presentation: &Presentation| {
            let p = first_p(&slide_one_html(presentation));
            let lifts = p
                .matches("vertical-align:-0.04em;position:relative;top:-0.04em")
                .count();
            (opening(&p).to_owned(), lifts)
        };
        // Spans that each set a vertical-align shape apart; text in one line box would kern across their edge.
        title_paragraph(&mut presentation).inlines =
            vec![skew(&text, &props), skew(" more", &props)];
        let (strut, lifts) = spans(&presentation);
        assert!(
            strut.contains("line-height:0;") && lifts == 2,
            "{strut} {lifts}"
        );
        title_paragraph(&mut presentation).inlines = vec![skew(&text, &props), Inline::Tab {}];
        let (strut, lifts) = spans(&presentation);
        assert!(
            strut.contains("line-height:0;") && lifts == 2,
            "{strut} {lifts}"
        );
        let larger = ResolvedRunProps {
            size: Size::from_steps(props.size.steps() + 4000),
            ..props.clone()
        };
        title_paragraph(&mut presentation).inlines =
            vec![skew(&text, &props), brk(), skew("larger", &larger)];
        let (strut, lifts) = spans(&presentation);
        assert!(
            strut.contains("line-height:0;") && lifts == 2,
            "{strut} {lifts}"
        );
        title_paragraph(&mut presentation).inlines = vec![skew(&text, &props)];
        let label = |font: FamilyFont| ResolvedBullet::Label {
            text: "•".to_owned(),
            font,
            size: props.size,
            color: props.color.clone(),
            auto: None,
        };
        title_paragraph(&mut presentation).props.bullet = label(family("Symbol"));
        let (strut, lifts) = spans(&presentation);
        assert!(
            strut.contains("line-height:0;") && lifts == 1,
            "{strut} {lifts}"
        );
        title_paragraph(&mut presentation).props.bullet = label(props.font.clone());
        let (strut, lifts) = spans(&presentation);
        assert!(
            strut.contains("position:relative;top:-0.04em") && lifts == 0,
            "{strut} {lifts}"
        );
        // A skewed run in a paragraph sized by an unskewed one.
        let smaller = ResolvedRunProps {
            size: Size::from_steps(props.size.steps() - 4000),
            ..props.clone()
        };
        let paragraph = title_paragraph(&mut presentation);
        paragraph.props.bullet = ResolvedBullet::None {};
        paragraph.inlines = vec![run("small", smaller), brk(), skew(&text, &props)];
        let (strut, lifts) = spans(&presentation);
        assert!(
            !strut.contains("position:relative") && lifts == 1,
            "{strut} {lifts}"
        );
    }

    #[test]
    fn sizes_a_skewed_line_by_its_spans_alone_a_tab_and_the_line_a_final_break_opens_included() {
        let mut presentation = full();
        let (text, props) = title_run(&mut presentation);
        let paragraph = title_paragraph(&mut presentation);
        paragraph.props.line_height = LineHeight::Ratio { value: ratio(1.38) };
        paragraph.props.leading_skew = Some(ratio(-0.15));
        paragraph.inlines = vec![skewed(&text, props, -0.15), Inline::Tab {}, brk()];
        let p = first_p(&slide_one_html(&presentation));
        // A centred strut would round its ascent and descent apart from the spans' and grow each line by a pixel.
        assert!(opening(&p).contains("line-height:0;"));
        let line = "line-height:1.38;vertical-align:-0.075em;position:relative;top:-0.075em";
        let span = span_with_text(&p, "Presentation IR");
        assert!(
            span.contains("line-height:1.38;")
                && span.ends_with(";vertical-align:-0.075em;position:relative;top:-0.075em\">")
        );
        assert!(p.contains(&format!(
            "<span style=\"{line}\">\t</span><br><span style=\"{line}\"><br></span></p>"
        )));
    }

    #[test]
    fn keeps_the_line_of_an_empty_paragraph_and_of_a_final_break_sized_by_the_paragraph_mark() {
        let mut presentation = full();
        let (_, props) = title_run(&mut presentation);
        let paragraph = title_paragraph(&mut presentation);
        paragraph.end_props = Some(ResolvedRunProps {
            size: size(15.0),
            ..props.clone()
        });
        paragraph.inlines = vec![run(
            "",
            ResolvedRunProps {
                size: size(15.0),
                ..props.clone()
            },
        )];
        // The mark sizes the paragraph itself, which holds its line break alone.
        let p = first_p(&slide_one_html(&presentation));
        assert!(
            opening(&p).contains(";font-size:15pt;") && p.ends_with("><br></p>"),
            "{p}"
        );
        // An empty run that sets the line otherwise than the mark still sizes it.
        title_paragraph(&mut presentation).inlines = vec![run(
            "",
            ResolvedRunProps {
                size: size(60.0),
                ..props.clone()
            },
        )];
        let p = first_p(&slide_one_html(&presentation));
        assert!(
            p.ends_with("\"></span><br></p>") && span_with_text(&p, "").contains("font-size:60pt"),
            "{p}"
        );
        title_paragraph(&mut presentation).inlines = vec![run("last", props.clone()), brk()];
        assert!(
            first_p(&slide_one_html(&presentation))
                .ends_with("><span>last</span><br><span style=\"font-size:15pt\"><br></span></p>")
        );
        title_paragraph(&mut presentation).end_props = Some(props);
        assert!(
            first_p(&slide_one_html(&presentation)).ends_with("><span>last</span><br><br></p>")
        );
    }

    #[test]
    fn prints_the_spaces_that_end_a_line_out_of_flow_so_they_take_no_room_when_the_line_is_aligned()
    {
        let mut presentation = full();
        let (_, props) = title_run(&mut presentation);
        let paragraph = title_paragraph(&mut presentation);
        paragraph.props.align = Align::Center;
        paragraph.inlines = vec![
            run("1º dia  ", props.clone()),
            brk(),
            run("next ", props.clone()),
            run(" ", props.clone()),
            brk(),
            run("   ", props),
        ];
        let p = first_p(&slide_one_html(&presentation));
        let hung = "<span style=\"position:absolute\"> </span>";
        assert!(
            p.contains("><span>1º dia</span><span style=\"position:absolute\">  </span><br>"),
            "{p}"
        );
        // Spaces split over runs at a line's end all hang; a line of nothing but spaces keeps them in flow.
        assert!(
            p.contains(&format!("><span>next</span>{hung}{hung}<br>")),
            "{p}"
        );
        assert!(p.ends_with("<br><span>   </span></p>"), "{p}");
    }
}

mod media {
    use super::*;

    #[test]
    fn shows_the_poster_when_the_video_has_no_output_and_declines_a_video_with_no_poster() {
        let mut presentation = full();
        let video = video_mut(&mut presentation).clone();
        let media = video.media.unwrap();
        presentation.media.get_mut(&media).unwrap().plan = omitted("unsupported-codec");
        let output = emit(&presentation);
        let html = &output.files.text["slides/ir-s1.html"];
        assert!(!html.contains("<video"));
        assert!(html.contains(&format!(
            "<img src=\"../media/{}.png\"",
            video.poster.unwrap()
        )));
        assert!(
            !output
                .files
                .media
                .contains_key(&format!("media/{media}.mp4"))
        );

        video_mut(&mut presentation).poster = None;
        let output = emit(&presentation);
        assert!(output.files.text["slides/ir-s1.html"].contains("Video without a poster"));
        let found: Vec<(String, Severity, String)> = output
            .diagnostics
            .iter()
            .filter(|d| d.container.as_ref().is_some_and(|c| c.as_str() == "ir-s1"))
            .map(|d| {
                (
                    d.code.code().to_owned(),
                    d.severity,
                    d.node.as_ref().unwrap().to_string(),
                )
            })
            .collect();
        assert_eq!(
            found,
            [(
                "HTML6005".to_owned(),
                Severity::Decline,
                "ir-s1.video".to_owned()
            )]
        );
        assert_eq!(output.files.lint(), vec![]);
    }

    #[test]
    fn marks_a_video_set_to_play_across_slides_for_the_player_and_no_other() {
        let mut presentation = full();
        let video = |html: &str| {
            let start = html.find("<video").unwrap();
            html[start..start + html[start..].find('>').unwrap()].to_owned()
        };
        let plain = video(&emit(&presentation).files.text["slides/ir-s1.html"]);
        assert!(!plain.contains("data-across-slides"), "{plain}");
        video_mut(&mut presentation).across_slides = true;
        let output = emit(&presentation);
        let marked = video(&output.files.text["slides/ir-s1.html"]);
        assert_eq!(marked, format!("{plain} data-across-slides"));
        assert_eq!(output.files.lint(), vec![]);
    }

    #[test]
    fn marks_a_picture_whose_bytes_are_not_written() {
        let mut presentation = full();
        let media = picture_mut(&mut presentation, "ir-s1.photo").media;
        presentation.media.get_mut(&media).unwrap().plan = omitted("too-large");
        let output = emit(&presentation);
        let html = &output.files.text["slides/ir-s1.html"];
        assert!(html.contains("Image not included"));
        // The label sits at the box's centre, which stays on the slide when the box runs off its top or left edge.
        let marker = tags_with(html, "div", "data-sd-marker");
        assert_eq!(marker.len(), 1, "{html}");
        assert!(
            marker[0].contains(
                "display:flex;align-items:center;justify-content:center;text-align:center"
            ),
            "{}",
            marker[0]
        );
        assert_eq!(output.files.lint(), vec![]);
    }
}

mod pictures {
    use super::*;

    fn printed(presentation: &Presentation) -> (Document, NodeId, Option<NodeId>) {
        let doc = Document::parse(&slide_one_html(presentation));
        let element = element(&doc, "ir-s1.photo");
        let image = common::descendants(&doc, element)
            .into_iter()
            .find(|n| doc.tag_name(*n) == "img");
        (doc, element, image)
    }

    #[test]
    fn prints_an_opaque_uncut_image_as_the_element_itself_its_box_on_the_img() {
        for opacity in [None, Some(unit(1.0))] {
            let mut presentation = full();
            let photo = picture_mut(&mut presentation, "ir-s1.photo");
            photo.crop = None;
            photo.clip = None;
            photo.opacity = opacity;
            photo.frame = Frame {
                rotation: angle(30.0),
                ..frame(10.0, 20.0, 100.0, 50.0)
            };
            let (doc, element, _) = printed(&presentation);
            assert_eq!(doc.tag_name(element), "img");
            assert_eq!(attr(&doc, element, "data-kind"), Some("picture"));
            assert_eq!(attr(&doc, element, "alt"), Some("A photo"));
            assert_eq!(
                attr(&doc, element, "style"),
                Some(
                    "position:absolute;left:10pt;top:20pt;width:100pt;height:50pt;rotate:30deg;clip:rect(auto, auto, auto, auto)"
                )
            );
            assert_eq!(emit(&presentation).files.lint(), vec![]);
        }
    }

    #[test]
    fn prints_opacity_below_1_only() {
        for (opacity, printed) in [
            (1.0, None),
            (0.99999, Some("opacity:0.99999")),
            (0.0, Some("opacity:0")),
        ] {
            let mut presentation = full();
            shape_mut(&mut presentation, "ir-s1.arrow").opacity = Some(unit(opacity));
            let html = slide_one_html(&presentation);
            let doc = Document::parse(&html);
            let style = attr(&doc, element(&doc, "ir-s1.arrow"), "style")
                .unwrap()
                .to_owned();
            let found = style.split(';').find(|d| d.starts_with("opacity:"));
            assert_eq!(found, printed, "{opacity}: {style}");
        }
    }

    #[test]
    fn keeps_a_clipping_box_for_a_crop_a_translucent_image_and_an_outline_the_opacity_on_the_img() {
        let mut cropped = full();
        picture_mut(&mut cropped, "ir-s1.photo").clip = None;
        let uncut = |presentation: &mut Presentation| {
            let photo = picture_mut(presentation, "ir-s1.photo");
            photo.crop = None;
            photo.clip = None;
        };
        let mut translucent = full();
        uncut(&mut translucent);
        let mut outlined = full();
        uncut(&mut outlined);
        let stroke = shape_mut(&mut outlined, "ir-s1.arrow").stroke.clone();
        picture_mut(&mut outlined, "ir-s1.photo").stroke = stroke;
        for presentation in [cropped, translucent, outlined] {
            let (doc, element, image) = printed(&presentation);
            assert_eq!(doc.tag_name(element), "div");
            let style = attr(&doc, element, "style").unwrap();
            assert!(
                style.ends_with(";overflow:hidden") || style.contains(";overflow:hidden;"),
                "{style}"
            );
            assert!(!style.contains("opacity"));
            assert!(
                attr(&doc, image.unwrap(), "style")
                    .unwrap()
                    .ends_with(";opacity:0.5")
            );
        }
    }

    /// The `clip-path` declaration of a style, if any.
    fn clip_path_of(style: &str) -> Option<&str> {
        style.split(';').find_map(|d| d.strip_prefix("clip-path:"))
    }

    #[test]
    fn cuts_a_masked_image_to_its_clip_and_its_mask_in_a_box_of_its_own_and_strokes_the_clip() {
        // The left half of the 120 x 90 pt photo, which the fixture clips to an ellipse.
        let left_half = path(vec![
            m(0.0, 0.0),
            l(60.0, 0.0),
            l(60.0, 90.0),
            l(0.0, 90.0),
            slidock_ir::geometry::Seg::Z,
        ]);
        let mut both = full();
        let stroke = shape_mut(&mut both, "ir-s1.arrow").stroke.clone();
        let photo = picture_mut(&mut both, "ir-s1.photo");
        photo.mask = Some(vec![left_half.clone()]);
        photo.stroke = stroke;
        let (doc, element, image) = printed(&both);
        let style = attr(&doc, element, "style").unwrap();
        let ellipse = clip_path_of(style).unwrap();
        assert!(ellipse.starts_with("path(\"M0 60C"), "{style}");
        assert!(
            attr(&doc, element, "data-clip")
                .unwrap()
                .starts_with("M0 45C")
        );
        assert_eq!(
            attr(&doc, element, "data-mask"),
            Some("M0 0L60 0L60 90L0 90Z")
        );
        // The mask cuts a box of its own inside the clip, which holds the image; the outline is the clip's.
        let parts = common::children(&doc, element);
        assert_eq!(
            parts.iter().map(|n| doc.tag_name(*n)).collect::<Vec<_>>(),
            ["div", "svg"]
        );
        let mask_box = parts[0];
        assert!(doc.attribute(mask_box, "data-sd-mask").is_some());
        assert_eq!(
            attr(&doc, mask_box, "style"),
            Some("position:absolute;inset:0;clip-path:path(\"M0 0L80 0L80 120L0 120Z\")")
        );
        assert_eq!(common::children(&doc, mask_box), [image.unwrap()]);
        let outline = common::children(&doc, parts[1]);
        assert_eq!(outline.len(), 1);
        assert_eq!(
            attr(&doc, outline[0], "d"),
            attr(&doc, element, "data-clip")
        );
        assert_eq!(emit(&both).files.lint(), vec![]);

        // A mask alone is the box's clip, with no box of its own.
        let mut masked = full();
        let photo = picture_mut(&mut masked, "ir-s1.photo");
        photo.clip = None;
        photo.mask = Some(vec![left_half]);
        let (doc, element, image) = printed(&masked);
        assert_eq!(
            clip_path_of(attr(&doc, element, "style").unwrap()),
            Some("path(\"M0 0L80 0L80 120L0 120Z\")")
        );
        assert_eq!(common::children(&doc, element), [image.unwrap()]);
        assert!(
            doc.walk(element)
                .iter()
                .all(|n| doc.attribute(*n, "data-sd-mask").is_none())
        );
        assert_eq!(emit(&masked).files.lint(), vec![]);
    }
}

/// A resolved fill in its source form, as a failed container keeps its background.
fn source_fill(fill: &ResolvedFill) -> Fill {
    let color = |c: &ResolvedColor| match c {
        ResolvedColor::Rgb(rgb) => Color::Rgb(rgb.clone()),
        ResolvedColor::Theme(theme) => Color::Theme(ThemeColor {
            kind: ThemeTag,
            slot: theme.slot,
            mods: theme.mods.clone(),
            resolved: Some(theme.resolved.clone()),
        }),
    };
    let stops = |stops: &[GradientStop<ResolvedColor>]| {
        stops
            .iter()
            .map(|s| GradientStop {
                pos: s.pos,
                color: color(&s.color),
            })
            .collect()
    };
    match fill {
        ResolvedFill::Radial(radial) => Fill::Radial(RadialLowered {
            center: radial.center.clone(),
            radius: radial.radius,
            radius_y: radial.radius_y,
            focus: radial.focus.clone(),
            stops: stops(&radial.stops),
            steps: radial.steps,
        }),
        ResolvedFill::Linear(linear) => Fill::Linear(LinearLowered {
            from: linear.from.clone(),
            to: linear.to.clone(),
            stops: stops(&linear.stops),
            steps: linear.steps,
        }),
        ResolvedFill::Solid { color: c } => Fill::Solid { color: color(c) },
        _ => Fill::None,
    }
}

mod failed_containers {
    use super::*;

    #[test]
    fn print_their_background_and_a_marker_and_lint_clean() {
        let mut presentation = full();
        let slide = &mut presentation.slides[1];
        let Body::Built(built) = &slide.body else {
            panic!("built")
        };
        let background = built.background.as_ref().map(source_fill);
        slide.failed_a11y = Some(FailedA11y {
            title: Some(built.title.clone()),
            reading_order: Some(built.reading_order.clone()),
        });
        slide.body = Body::Failed {
            failed: Failed::new(codes::IR0908, Some("cascade-text".to_owned())),
            background,
        };
        slide.timeline = Vec::new();
        let layout = presentation.layouts.get_mut(&id("ir-layout")).unwrap();
        layout.body = Body::Failed {
            failed: Failed::new(codes::IR0909, None),
            background: None,
        };
        let files = emit(&presentation).files;
        let slide = &files.text["slides/ir-s2.html"];
        assert!(slide.contains("data-failed=\"IR0908\""));
        assert!(slide.contains("<div data-sd-bg"));
        assert!(slide.contains(">IR0908</div>"));
        assert!(files.text["layouts/ir-layout.html"].contains("data-failed=\"IR0909\""));
        assert!(files.text["presentation.json"].contains("\"failed\": \"IR0908\""));
        assert_eq!(files.lint(), vec![]);
    }

    #[test]
    fn title_a_failed_slide_by_the_title_it_kept_else_leave_it_untitled() {
        for (kept, printed) in [
            (
                Some("Back to the start"),
                "<title>Back to the start</title>",
            ),
            (None, "<title></title>"),
        ] {
            let mut presentation = full();
            let slide = &mut presentation.slides[1];
            slide.failed_a11y = Some(FailedA11y {
                title: kept.map(str::to_owned),
                reading_order: None,
            });
            slide.body = Body::Failed {
                failed: Failed::new(codes::IR0908, None),
                background: None,
            };
            slide.timeline = Vec::new();
            let html = &emit(&presentation).files.text["slides/ir-s2.html"];
            assert!(html.contains(printed), "{kept:?}: {html}");
        }
    }
}

mod tables {
    use super::*;

    fn table_of(doc: &Document) -> NodeId {
        element(doc, "ir-s2.grid")
    }

    fn cells(doc: &Document) -> Vec<NodeId> {
        common::descendants(doc, table_of(doc))
            .into_iter()
            .filter(|n| doc.tag_name(*n) == "td")
            .collect()
    }

    fn parse_decls(style: &str) -> Vec<(String, String)> {
        style
            .split(';')
            .filter_map(|d| d.split_once(':').map(|(p, v)| (p.to_owned(), v.to_owned())))
            .collect()
    }

    fn get(decls: &[(String, String)], property: &str) -> Option<String> {
        decls
            .iter()
            .find(|(p, _)| p == property)
            .map(|(_, v)| v.clone())
    }

    fn cell_mut(presentation: &mut Presentation, row: usize, column: usize) -> &mut Cell {
        grid_mut(presentation).rows[row].cells[column]
            .as_mut()
            .unwrap()
    }

    /// The right edge of the lower left cell: 1 pt, dashed, grey.
    fn edge(presentation: &mut Presentation) -> Border<ResolvedColor> {
        cell_mut(presentation, 1, 0)
            .borders
            .as_ref()
            .unwrap()
            .right
            .clone()
            .unwrap()
    }

    #[test]
    fn prints_a_fixed_table_as_the_positioned_element_itself() {
        let html = slide_two_html(&full());
        let doc = Document::parse(&html);
        let table = table_of(&doc);
        assert_eq!(
            (doc.tag_name(table), attr(&doc, table, "data-kind")),
            ("table", Some("table"))
        );
        // As tall as its rows: a height would stretch them.
        assert_eq!(
            attr(&doc, table, "style"),
            Some(
                "position:absolute;left:60pt;top:360pt;width:250pt;border-collapse:collapse;table-layout:fixed"
            )
        );
        let inside = common::descendants(&doc, table);
        assert!(inside.iter().all(|n| attr(&doc, *n, "data-el").is_none()));
        let tags: Vec<&str> = common::children(&doc, table)
            .iter()
            .map(|n| doc.tag_name(*n))
            .collect();
        assert_eq!(tags, ["colgroup", "tbody"]);
        let styles = |tag: &str| -> Vec<&str> {
            inside
                .iter()
                .filter(|n| doc.tag_name(**n) == tag)
                .map(|n| attr(&doc, *n, "style").unwrap())
                .collect()
        };
        assert_eq!(styles("col"), ["width:100pt", "width:150pt"]);
        assert_eq!(styles("tr"), ["height:30pt", "height:40pt"]);
        let spans: Vec<(Option<&str>, Option<&str>)> = cells(&doc)
            .iter()
            .map(|td| (attr(&doc, *td, "colspan"), attr(&doc, *td, "rowspan")))
            .collect();
        assert_eq!(spans, [(Some("2"), None), (None, None), (None, None)]);
    }

    #[test]
    fn pads_each_cell_by_its_insets_less_half_the_collapsed_border_and_aligns_it_by_its_anchor() {
        let html = slide_two_html(&full());
        let doc = Document::parse(&html);
        let found: Vec<(String, String)> = cells(&doc)
            .iter()
            .map(|td| {
                let decls = parse_decls(attr(&doc, *td, "style").unwrap());
                (
                    get(&decls, "padding").unwrap(),
                    get(&decls, "vertical-align").unwrap(),
                )
            })
            .collect();
        // The header's 2 pt bottom edge takes 1 pt off its own bottom inset and off the top insets below it; the
        // 1 pt edge between the lower cells takes 0.5 pt off each side.
        assert_eq!(
            found,
            pairs(&[
                ("3.6pt 7.2pt 2.6pt", "middle"),
                ("2.6pt 6.7pt 3.6pt 7.2pt", "top"),
                ("2.6pt 7.2pt 3.6pt 6.7pt", "bottom"),
            ])
        );
    }

    #[test]
    fn keeps_a_cell_text_in_a_flex_column_of_its_own_raised_above_the_gradient_layer_of_its_cell() {
        let html = slide_two_html(&full());
        let doc = Document::parse(&html);
        let layered = cells(&doc)[2];
        assert!(
            attr(&doc, layered, "style")
                .unwrap()
                .contains("position:relative")
        );
        let children = common::children(&doc, layered);
        let tags: Vec<&str> = children.iter().map(|n| doc.tag_name(*n)).collect();
        assert_eq!(tags, ["svg", "div"]);
        assert_eq!(
            attr(&doc, children[1], "style"),
            Some(
                "position:relative;display:flex;flex-direction:column;white-space:pre-wrap;overflow-wrap:break-word;\
                 color:var(--sd-color-tx1,#000000)"
            )
        );
    }

    #[test]
    fn makes_a_cell_with_nothing_under_its_text_its_text_box_its_paragraphs_in_block_flow_with_nothing_between()
     {
        let html = slide_two_html(&full());
        let doc = Document::parse(&html);
        let plain = cells(&doc)[1];
        assert_eq!(attr(&doc, plain, "data-sd-text"), Some(""));
        assert_eq!(
            attr(&doc, plain, "style"),
            Some(
                "padding:2.6pt 6.7pt 3.6pt 7.2pt;vertical-align:top;border-right:1pt dashed #999999;\
                 white-space:pre-wrap;overflow-wrap:break-word;color:var(--sd-color-tx1,#000000)"
            )
        );
        assert_eq!(doc.children(plain).len(), 1);
        assert_eq!(doc.tag_name(doc.children(plain)[0]), "p");
    }

    #[test]
    fn keeps_a_cell_text_box_where_block_flow_would_collapse_the_margins_between_its_paragraphs() {
        let mut presentation = full();
        let paragraph = cell_mut(&mut presentation, 1, 0).text.paragraphs[0].clone();
        let spaced = |before: f64, after: f64| {
            let mut p = paragraph.clone();
            p.props.space_before_pt = pt(before);
            p.props.space_after_pt = pt(after);
            p
        };
        let mut holder = |paragraphs: Vec<ResolvedParagraph>| {
            cell_mut(&mut presentation, 1, 0).text.paragraphs = paragraphs;
            let html = slide_two_html(&presentation);
            let doc = Document::parse(&html);
            let td = cells(&doc)[1];
            let text_box = std::iter::once(td)
                .chain(common::descendants(&doc, td))
                .find(|n| doc.has_attribute(*n, "data-sd-text"))
                .unwrap();
            doc.tag_name(text_box).to_owned()
        };
        // Space after one paragraph and before the next add up in a flex column, where block flow keeps the larger.
        assert_eq!(holder(vec![spaced(0.0, 6.0), spaced(6.0, 0.0)]), "div");
        assert_eq!(holder(vec![spaced(6.0, 6.0), spaced(0.0, 6.0)]), "td");
        assert_eq!(holder(vec![spaced(6.0, 0.0), spaced(6.0, 0.0)]), "td");
    }

    #[test]
    fn keeps_a_cell_text_box_where_the_text_turns_apart_from_the_table() {
        let mut presentation = full();
        cell_mut(&mut presentation, 1, 0).text.rotation = TextRotation::Half;
        let html = slide_two_html(&presentation);
        let doc = Document::parse(&html);
        let td = cells(&doc)[1];
        assert_eq!(attr(&doc, td, "data-sd-text"), None);
        let text_box = common::descendants(&doc, td)
            .into_iter()
            .find(|n| doc.has_attribute(*n, "data-sd-text"))
            .unwrap();
        assert!(
            attr(&doc, text_box, "style")
                .unwrap()
                .ends_with(";scale:-1 -1")
        );
    }

    #[test]
    fn keeps_the_box_of_a_turned_table_as_tall_as_its_frame_which_the_turn_is_about() {
        let mut presentation = full();
        grid_mut(&mut presentation).frame.rotation = angle(90.0);
        let html = slide_two_html(&presentation);
        let doc = Document::parse(&html);
        let outer = table_of(&doc);
        assert_eq!(doc.tag_name(outer), "div");
        assert_eq!(
            attr(&doc, outer, "style"),
            Some("position:absolute;left:60pt;top:360pt;width:250pt;height:70pt;rotate:90deg")
        );
        let inner = common::descendants(&doc, outer)
            .into_iter()
            .find(|n| doc.tag_name(*n) == "table")
            .unwrap();
        assert_eq!(
            attr(&doc, inner, "style"),
            Some("border-collapse:collapse;table-layout:fixed;width:100%")
        );
    }

    #[test]
    fn names_a_table_by_its_alt_text_keeping_its_table_role_and_a_turned_table_through_its_box() {
        let mut presentation = full();
        grid_mut(&mut presentation).alt = Some("Quarterly numbers".to_owned());
        let named = |doc: &Document, n: NodeId| {
            (
                doc.tag_name(n).to_owned(),
                attr(doc, n, "role").map(str::to_owned),
                attr(doc, n, "aria-label").map(str::to_owned),
            )
        };
        let html = slide_two_html(&presentation);
        let doc = Document::parse(&html);
        assert_eq!(
            named(&doc, table_of(&doc)),
            (
                "table".to_owned(),
                None,
                Some("Quarterly numbers".to_owned())
            )
        );
        grid_mut(&mut presentation).frame.rotation = angle(90.0);
        let html = slide_two_html(&presentation);
        let doc = Document::parse(&html);
        let outer = table_of(&doc);
        assert_eq!(
            named(&doc, outer),
            (
                "div".to_owned(),
                Some("group".to_owned()),
                Some("Quarterly numbers".to_owned())
            )
        );
        let inner = common::descendants(&doc, outer)
            .into_iter()
            .find(|n| doc.tag_name(*n) == "table")
            .unwrap();
        assert_eq!(named(&doc, inner), ("table".to_owned(), None, None));
    }

    #[test]
    fn prints_four_alike_edges_of_a_cell_as_one_border() {
        let mut presentation = full();
        let edge = edge(&mut presentation);
        cell_mut(&mut presentation, 1, 0).borders = Some(CellBorders {
            top: Some(edge.clone()),
            right: Some(edge.clone()),
            bottom: Some(edge.clone()),
            left: Some(edge.clone()),
            ..edges()
        });
        let html = slide_two_html(&presentation);
        let doc = Document::parse(&html);
        let style = attr(&doc, cells(&doc)[1], "style").unwrap().to_owned();
        assert!(
            style.contains(";border:1pt dashed #999999;") && !style.contains("border-"),
            "{style}"
        );
        cell_mut(&mut presentation, 1, 0).borders = Some(CellBorders {
            top: Some(edge.clone()),
            right: Some(edge.clone()),
            bottom: Some(edge),
            ..edges()
        });
        let html = slide_two_html(&presentation);
        let doc = Document::parse(&html);
        assert!(
            !attr(&doc, cells(&doc)[1], "style")
                .unwrap()
                .contains(";border:")
        );
    }

    #[test]
    fn paints_a_cell_gradient_in_the_cell_as_drawn_however_far_its_row_grows_past_its_least_height()
    {
        let mut presentation = full();
        let Some(ResolvedCellFill::Fill(ResolvedFill::Linear(linear))) =
            cell_mut(&mut presentation, 1, 1).fill.clone()
        else {
            panic!("fixture")
        };
        let table = grid_mut(&mut presentation);
        table.rows[1].height = size(0.0);
        table.frame.h = table.rows[0].height;
        let mut cells_of = |fill: ResolvedCellFill| {
            cell_mut(&mut presentation, 1, 1).fill = Some(fill);
            slide_two_html(&presentation)
        };
        // An angle holds in the box as drawn: a CSS gradient, a quarter turn on from the IR's x axis.
        let html = cells_of(ResolvedCellFill::Angled(LinearAngled {
            angle: angle(90.0),
            stops: linear.stops.clone(),
        }));
        let doc = Document::parse(&html);
        let angled = cells(&doc)[2];
        assert!(
            attr(&doc, angled, "style")
                .unwrap()
                .contains("background:linear-gradient(180deg, #ffffff 0%, #cccccc 100%)")
        );
        assert!(
            !common::descendants(&doc, angled)
                .iter()
                .any(|n| doc.tag_name(*n) == "svg")
        );
        // Box fractions stretch with the cell: a unit layer over all of it.
        let html = cells_of(ResolvedCellFill::Fill(ResolvedFill::Linear(linear)));
        let doc = Document::parse(&html);
        let layer = common::descendants(&doc, cells(&doc)[2])[0];
        assert_eq!(
            (
                doc.tag_name(layer),
                attr(&doc, layer, "viewBox"),
                attr(&doc, layer, "width"),
                attr(&doc, layer, "height")
            ),
            ("svg", Some("0 0 1 1"), Some("100%"), Some("100%"))
        );
        assert_eq!(
            attr(&doc, layer, "style"),
            Some("position:absolute;left:0pt;top:0pt;overflow:visible")
        );
    }

    #[test]
    fn puts_the_grid_lines_on_the_frame_the_table_moves_out_by_half_its_widest_left_and_top_borders()
     {
        let mut presentation = full();
        let edge = edge(&mut presentation);
        let header = cell_mut(&mut presentation, 0, 0).borders.as_mut().unwrap();
        header.top = Some(Border {
            width: size(2.0),
            ..edge.clone()
        });
        header.left = Some(edge.clone());
        cell_mut(&mut presentation, 1, 0)
            .borders
            .as_mut()
            .unwrap()
            .left = Some(Border {
            width: size(3.0),
            ..edge
        });
        let html = slide_two_html(&presentation);
        let doc = Document::parse(&html);
        assert_eq!(
            attr(&doc, table_of(&doc), "style"),
            Some(
                "position:absolute;left:60pt;top:360pt;width:250pt;border-collapse:collapse;table-layout:fixed;margin:-1pt 0 0 -1.5pt"
            )
        );
    }

    #[test]
    fn ends_a_cell_text_where_its_last_lines_text_ends_short_of_the_lines_tail() {
        let mut presentation = full();
        let cell = cell_mut(&mut presentation, 1, 1);
        for paragraph in &mut cell.text.paragraphs {
            paragraph.props.space_after_pt = pt(1.0);
            paragraph.props.line_tail_pt = Some(pt(4.0));
        }
        let count = cell.text.paragraphs.len();
        let html = slide_two_html(&presentation);
        let doc = Document::parse(&html);
        let margins: Vec<Option<String>> = common::descendants(&doc, cells(&doc)[2])
            .into_iter()
            .filter(|n| doc.tag_name(*n) == "p")
            .map(|p| {
                get(
                    &parse_decls(attr(&doc, p, "style").unwrap()),
                    "margin-bottom",
                )
            })
            .collect();
        let mut expected = vec![Some("1pt".to_owned()); count - 1];
        expected.push(Some("-3pt".to_owned()));
        assert_eq!(margins, expected);
    }

    #[test]
    fn prints_the_labelled_paragraphs_of_a_cell_as_a_list_in_its_text_box() {
        let mut presentation = full();
        let label = shape_mut(&mut presentation, "ir-s1.body")
            .text
            .as_ref()
            .unwrap()
            .paragraphs[1]
            .props
            .bullet
            .clone();
        let cell = cell_mut(&mut presentation, 1, 1);
        for paragraph in &mut cell.text.paragraphs {
            paragraph.props.bullet = label.clone();
        }
        let count = cell.text.paragraphs.len();
        let html = slide_two_html(&presentation);
        let doc = Document::parse(&html);
        let td = cells(&doc)[2];
        let text_box = common::descendants(&doc, td)
            .into_iter()
            .find(|n| doc.has_attribute(*n, "data-sd-text"))
            .unwrap();
        let lists = common::children(&doc, text_box);
        let tags: Vec<&str> = lists.iter().map(|n| doc.tag_name(*n)).collect();
        assert_eq!(tags, ["ul"]);
        assert_eq!(
            common::descendants(&doc, lists[0])
                .iter()
                .filter(|n| doc.tag_name(**n) == "li")
                .count(),
            count
        );
    }

    #[test]
    fn styles_a_border_by_its_dash_pattern_naming_once_per_table_each_pattern_css_does_not_draw() {
        let mut presentation = full();
        let edge = edge(&mut presentation);
        let dashed = |dash: Dash| {
            Some(Border {
                dash,
                ..edge.clone()
            })
        };
        cell_mut(&mut presentation, 0, 0).borders = Some(CellBorders {
            top: dashed(Dash::Preset(DashPreset::SysDot)),
            ..edges()
        });
        // The same pattern, named or written out, prints alike and is reported once.
        cell_mut(&mut presentation, 1, 0).borders = Some(CellBorders {
            right: dashed(Dash::Preset(DashPreset::Dot)),
            ..edges()
        });
        cell_mut(&mut presentation, 1, 1).borders = Some(CellBorders {
            top: dashed(Dash::Custom {
                custom: vec![ratio(1.0), ratio(3.0)],
            }),
            bottom: dashed(Dash::Preset(DashPreset::LgDashDot)),
            ..edges()
        });
        let emitted = emit(&presentation);
        let html = &emitted.files.text["slides/ir-s2.html"];
        assert!(html.contains("border-top:1pt dotted #999999;"));
        assert!(html.contains("border-right:1pt dotted #999999"));
        assert!(html.contains("border-top:1pt dotted #999999;border-bottom:1pt dashed #999999"));
        let found: Vec<(String, Severity, Option<FeatureId>, String, String, String)> = emitted
            .diagnostics
            .iter()
            .map(|d| {
                let data = d.data.as_ref().unwrap();
                (
                    d.code.code().to_owned(),
                    d.severity,
                    d.feature,
                    d.node.as_ref().unwrap().to_string(),
                    data["dash"].to_js_string(),
                    data["style"].to_js_string(),
                )
            })
            .collect();
        let expect = |dash: &str, style: &str| {
            (
                "HTML6006".to_owned(),
                Severity::Lossy,
                Some(FeatureId::StrokeDash),
                "ir-s2.grid".to_owned(),
                dash.to_owned(),
                style.to_owned(),
            )
        };
        assert_eq!(
            found,
            [expect("1 3", "dotted"), expect("8 3 1 3", "dashed")]
        );
        assert_eq!(emitted.files.lint(), vec![]);
    }

    #[test]
    fn leaves_an_edge_that_paints_nothing_out_of_the_collapse_so_the_neighbours_line_facing_it_shows()
     {
        let mut presentation = full();
        let edge = edge(&mut presentation);
        let StrokePaint::Solid {
            color: ResolvedColor::Rgb(grey),
        } = &edge.fill
        else {
            panic!("fixture")
        };
        let clear = Border {
            width: size(2.0),
            fill: StrokePaint::Solid {
                color: literal(RgbColor {
                    a: unit(0.0),
                    ..grey.clone()
                }),
            },
            ..edge.clone()
        };
        cell_mut(&mut presentation, 1, 0).borders = Some(CellBorders {
            right: Some(clear),
            ..edges()
        });
        cell_mut(&mut presentation, 1, 1).borders = Some(CellBorders {
            left: Some(Border {
                dash: Dash::Preset(DashPreset::Solid),
                ..edge
            }),
            ..edges()
        });
        let html = slide_two_html(&presentation);
        let doc = Document::parse(&html);
        let cells = cells(&doc);
        let left = parse_decls(attr(&doc, cells[1], "style").unwrap());
        let right = parse_decls(attr(&doc, cells[2], "style").unwrap());
        assert_eq!(get(&left, "border-right"), None);
        assert_eq!(
            get(&right, "border-left").as_deref(),
            Some("1pt solid #999999")
        );
        // The collapsed edge is the visible 1 pt line: half of it comes off each side's inset.
        assert_eq!(
            get(&left, "padding").unwrap().split(' ').nth(1),
            Some("6.7pt")
        );
    }

    #[test]
    fn draws_the_diagonals_corner_to_corner_in_percentages_so_they_follow_a_growing_row() {
        let mut presentation = full();
        let edge = edge(&mut presentation);
        cell_mut(&mut presentation, 1, 0).borders = Some(CellBorders {
            diagonal_down: Some(edge.clone()),
            diagonal_up: Some(Border {
                dash: Dash::Preset(DashPreset::Solid),
                ..edge
            }),
            ..edges()
        });
        let html = slide_two_html(&presentation);
        assert!(html.contains(
            "<line x1=\"0\" y1=\"0\" x2=\"100%\" y2=\"100%\" stroke-width=\"1pt\" stroke-dasharray=\"4pt 3pt\" \
             stroke-linejoin=\"round\" style=\"stroke:#999999\"></line>"
        ));
        assert!(html.contains(
            "<line x1=\"0\" y1=\"100%\" x2=\"100%\" y2=\"0\" stroke-width=\"1pt\" stroke-linejoin=\"round\" style=\"stroke:#999999\"></line>"
        ));
    }

    #[test]
    fn draws_a_diagonal_with_its_borders_caps_and_joins_so_round_capped_zero_length_dashes_are_dots()
     {
        let mut presentation = full();
        let edge = edge(&mut presentation);
        let dotted = Border {
            width: size(3.0),
            dash: Dash::Custom {
                custom: vec![ratio(0.0), ratio(2.0)],
            },
            cap: Cap::Round,
            join: Join::Round,
            ..edge
        };
        cell_mut(&mut presentation, 1, 0).borders = Some(CellBorders {
            diagonal_down: Some(dotted),
            ..edges()
        });
        assert!(slide_two_html(&presentation).contains(
            "<line x1=\"0\" y1=\"0\" x2=\"100%\" y2=\"100%\" stroke-width=\"3pt\" stroke-dasharray=\"0pt 6pt\" \
             stroke-linecap=\"round\" stroke-linejoin=\"round\" style=\"stroke:#999999\"></line>"
        ));
    }
}

mod supported {
    use super::*;

    #[test]
    fn leaves_out_what_it_cannot_print_and_foreign_content_the_core_names() {
        let supported = supported_features();
        for feature in [FeatureId::ElementAudio, FeatureId::FontEmbedded] {
            assert!(!supported.contains(&feature), "{feature:?}");
        }
        for feature in [
            FeatureId::ImageVector,
            FeatureId::ElementVideo,
            FeatureId::ElementTable,
            FeatureId::TextLink,
            FeatureId::FillRadial,
            FeatureId::SlideTimeline,
        ] {
            assert!(supported.contains(&feature), "{feature:?}");
        }
    }

    #[test]
    fn names_each_unprinted_feature_at_its_floor_decline_lossy_or_info() {
        let mut presentation = full();
        presentation
            .fonts
            .get_index_mut(0)
            .unwrap()
            .1
            .request
            .embedded = true;
        let mut audio = video_mut(&mut presentation).clone();
        audio.id = id("ir-s1.voice");
        audio.media_kind = MediaKind::Audio;
        audio.poster = None;
        elements_mut(&mut presentation, 0).push(Element::Media(Box::new(audio)));
        // The grid's dashed edges print approximated (HTML6006), which the table tests cover.
        let found: Vec<(String, Severity, String, String)> = emit(&presentation)
            .diagnostics
            .iter()
            .filter(|d| d.node.as_ref().is_none_or(|n| n.as_str() != "ir-s2.grid"))
            .map(|d| {
                (
                    d.code.code().to_owned(),
                    d.severity,
                    d.container
                        .as_ref()
                        .map_or("presentation".to_owned(), ToString::to_string),
                    d.data.as_ref().unwrap()["feature"].to_js_string(),
                )
            })
            .collect();
        assert_eq!(
            found,
            [
                (
                    "HTML6004".to_owned(),
                    Severity::Info,
                    "presentation".to_owned(),
                    "font.embedded".to_owned()
                ),
                (
                    "HTML6003".to_owned(),
                    Severity::Lossy,
                    "ir-s1".to_owned(),
                    "element.audio".to_owned()
                ),
            ]
        );
    }
}

mod documents {
    use super::*;

    #[test]
    fn writes_the_same_bytes_twice() {
        assert_eq!(emit(&full()), emit(&full()));
    }

    #[test]
    fn writes_a_file_set_that_lints_clean() {
        let emitted = emit(&full());
        assert_eq!(lint_presentation(&emitted.files.lintable()), vec![]);
        let paths: Vec<&str> = emitted.files.text.keys().map(String::as_str).collect();
        assert_eq!(
            paths,
            [
                "layouts/ir-layout.html",
                "presentation.json",
                "slides/ir-s1.html",
                "slides/ir-s2.html"
            ]
        );
    }

    #[test]
    fn prints_slidock_markers_only() {
        let emitted = emit(&full());
        for (path, text) in &emitted.files.text {
            assert!(
                !text.contains("data-ac-") && !text.contains("--ac-color-"),
                "{path}"
            );
        }
        let slide = &emitted.files.text["slides/ir-s1.html"];
        assert!(
            slide.contains("<div data-sd-bg")
                || emitted.files.text["slides/ir-s2.html"].contains("<div data-sd-bg")
        );
        assert!(slide.contains("var(--sd-color-tx2,"));
        // The build steps are deck data: no timeline template in the document.
        assert!(!slide.contains("<template"));
    }

    #[test]
    fn numbers_each_paragraph_block_by_its_index_in_the_body() {
        let html = slide_one_html(&full());
        let doc = Document::parse(&html);
        let body = element(&doc, "ir-s1.body");
        let numbered: Vec<(&str, Option<&str>)> = common::descendants(&doc, body)
            .into_iter()
            .filter(|n| matches!(doc.tag_name(*n), "p" | "li"))
            .map(|n| (doc.tag_name(n), attr(&doc, n, "data-p")))
            .collect();
        assert_eq!(numbered, [("li", Some("0")), ("li", Some("1"))]);
        // A table cell's body numbers from 0.
        let html = slide_two_html(&full());
        let doc = Document::parse(&html);
        let indices: Vec<Option<&str>> = common::elements(&doc)
            .into_iter()
            .filter(|n| doc.tag_name(*n) == "p")
            .map(|n| attr(&doc, n, "data-p"))
            .collect();
        assert_eq!(indices, [Some("0"); 4]);
    }

    #[test]
    fn records_the_media_it_references_with_their_plans() {
        let emitted = emit(&full());
        let media: Vec<&str> = emitted.files.media.keys().map(String::as_str).collect();
        assert_eq!(media.len(), 5);
        let report = emitted.report().unwrap();
        assert!(report.starts_with("{\n  \"diagnostics\": ["));
        assert!(report.contains("\"action\": \"copy\""));
        assert!(report.contains("\"pixels\": 480000"));
        assert!(report.ends_with("}\n"));
    }

    #[test]
    fn writes_the_manifest_with_notes_transitions_and_themes() {
        let manifest = &emit(&full()).files.text["presentation.json"];
        assert!(manifest.contains("\"notes\": \"Speaker notes\""));
        assert!(manifest.contains("\"effect\": \"pptx:fade\""));
        assert!(manifest.contains("\"accent1\": \"#4472c4\""));
        assert!(manifest.contains("\"name\": \"slidock\""));
        assert!(manifest.contains("\"title\": \"full\""));
    }

    #[test]
    fn trims_the_javascript_whitespace_that_ends_the_notes() {
        let mut presentation = full();
        let notes = presentation.slides[0].notes.as_mut().unwrap();
        let mut ended = notes.paragraphs[0].clone();
        let Inline::Run { text, .. } = &mut ended.inlines[0] else {
            panic!("run")
        };
        // U+0085 is not JavaScript whitespace, so it stays; U+FEFF is, so it goes.
        *text = "\u{85}\u{feff}\u{3000} \t".to_owned();
        notes.paragraphs.push(ended);
        assert_eq!(
            notes_text(presentation.slides[0].notes.as_ref()),
            "Speaker notes\n\u{85}"
        );
        let Inline::Run { text, .. } =
            &mut presentation.slides[0].notes.as_mut().unwrap().paragraphs[1].inlines[0]
        else {
            panic!("run")
        };
        *text = " \u{feff}\u{a0}\u{2028}".to_owned();
        assert_eq!(
            notes_text(presentation.slides[0].notes.as_ref()),
            "Speaker notes"
        );
        assert!(
            emit(&presentation).files.text["presentation.json"]
                .contains("\"notes\": \"Speaker notes\"")
        );
    }
}

/// Gradients as paint servers, on hand-written resolved IR.
mod gradients {
    use super::*;
    use slidock_ir::paint::{Edges, EllipseLowered, Point, RectLowered, Semi};

    fn cell_mut(presentation: &mut Presentation, row: usize, column: usize) -> &mut Cell {
        grid_mut(presentation).rows[row].cells[column]
            .as_mut()
            .unwrap()
    }

    fn p(x: f64, y: f64) -> Point {
        Point {
            x: ratio(x),
            y: ratio(y),
        }
    }

    fn black_white() -> Vec<GradientStop<ResolvedColor>> {
        vec![stop(0.0, literal(black())), stop(1.0, literal(white()))]
    }

    /// The title (648 × 60 pt) filled with `fill`; its slide's markup.
    fn painted(fill: ResolvedFill) -> String {
        let mut presentation = full();
        shape_mut(&mut presentation, "ir-s1.title").fill = Some(fill);
        let emitted = emit(&presentation);
        assert_eq!(emitted.files.lint(), vec![]);
        emitted.files.text["slides/ir-s1.html"].clone()
    }

    /// The markup from `<tag` to its end.
    fn element_markup(html: &str, tag: &str) -> String {
        let start = html
            .find(&format!("<{tag} "))
            .unwrap_or_else(|| panic!("no <{tag}> in {html}"));
        let close = format!("</{tag}>");
        let end = html[start..].find(&close).unwrap() + start + close.len();
        html[start..end].to_owned()
    }

    fn radial(radius_y: Option<f64>, focus: Option<Point>) -> ResolvedFill {
        ResolvedFill::Radial(RadialLowered {
            center: p(0.5, 0.5),
            radius: ratio(0.5),
            radius_y: radius_y.map(ratio),
            focus,
            stops: black_white(),
            steps: None,
        })
    }

    #[test]
    fn prints_linear_and_box_aspect_radial_gradients_as_before() {
        let html = painted(ResolvedFill::Linear(vertical(
            literal(black()),
            literal(white()),
        )));
        assert_eq!(
            element_markup(&html, "linearGradient"),
            "<linearGradient id=\"g-ir-s1.title\" gradientUnits=\"userSpaceOnUse\" gradientTransform=\"scale(648 60)\" \
             x1=\"0.5\" y1=\"0\" x2=\"0.5\" y2=\"1\">\n<stop offset=\"0\" style=\"stop-color:#000000\"></stop>\n\
             <stop offset=\"1\" style=\"stop-color:#ffffff\"></stop>\n</linearGradient>"
        );
        let html = painted(radial(None, None));
        assert!(
            element_markup(&html, "radialGradient").starts_with(
                "<radialGradient id=\"g-ir-s1.title\" gradientUnits=\"userSpaceOnUse\" \
                 gradientTransform=\"scale(648 60)\" cx=\"0.5\" cy=\"0.5\" r=\"0.5\">"
            ),
            "{html}"
        );
    }

    #[test]
    fn maps_a_unit_circle_onto_an_ellipse_with_its_focus_in_its_units() {
        let html = painted(radial(Some(2.0), Some(p(0.25, 0.75))));
        // rx = 0.5 of 648, ry = 2 of 60, about (324, 30); the focus half the x radius left, an eighth of y down.
        assert!(
            element_markup(&html, "radialGradient").starts_with(
                "<radialGradient id=\"g-ir-s1.title\" gradientUnits=\"userSpaceOnUse\" cx=\"0\" cy=\"0\" r=\"1\" \
                 fx=\"-0.5\" fy=\"0.125\" gradientTransform=\"matrix(324 0 0 120 324 30)\">"
            ),
            "{html}"
        );
    }

    #[test]
    fn draws_rect_contours_as_two_ramps_in_one_translated_pattern() {
        let html = painted(ResolvedFill::Rect(Box::new(RectLowered {
            origin: p(0.5, 0.5),
            axis_x: p(0.5, 0.0),
            axis_y: p(0.0, 0.5),
            inner: Edges {
                l: ratio(-0.8),
                t: ratio(-0.4),
                r: ratio(-0.6),
                b: ratio(0.6),
            },
            stops: black_white(),
            steps: None,
        })));
        let pattern = element_markup(&html, "pattern");
        // The tile is the box grown by 1 pt, and its content is drawn back by its corner.
        assert!(
            pattern.starts_with(
                "<pattern id=\"g-ir-s1.title\" patternUnits=\"userSpaceOnUse\" x=\"-1\" y=\"-1\" width=\"650\" \
                 height=\"62\">\n<g transform=\"translate(1 1)\">\n<g transform=\"matrix(324 0 0 30 324 30)\">"
            ),
            "{pattern}"
        );
        assert!(pattern.contains("style=\"fill:url(#g-ir-s1.title-y)\""));
        // The wedges where |x| dominates: from the inner corners out along the mitres.
        assert!(pattern.contains("<path d=\"M-0.8 -0.4L"), "{pattern}");
        assert!(pattern.contains("style=\"fill:url(#g-ir-s1.title-x)\""));
        // The x ramp: white at the left edge, black from the inner rectangle's left to its right, white at the right.
        let x = element_markup(&html, "linearGradient");
        assert!(
            x.starts_with("<linearGradient id=\"g-ir-s1.title-x\""),
            "{x}"
        );
        let offsets: Vec<&str> = x
            .match_indices("offset=\"")
            .map(|(i, _)| &x[i + 8..i + 8 + x[i + 8..].find('"').unwrap()])
            .collect();
        assert_eq!(offsets, ["0", "0.1", "0.2", "1"]);
        assert!(html.contains("fill:url(#g-ir-s1.title)"));
    }

    #[test]
    fn paints_rect_bands_outermost_first_one_rect_each() {
        let html = painted(ResolvedFill::Rect(Box::new(RectLowered {
            origin: p(0.5, 0.5),
            axis_x: p(0.5, 0.0),
            axis_y: p(0.0, 0.5),
            inner: Edges {
                l: ratio(-0.5),
                t: ratio(0.0),
                r: ratio(0.5),
                b: ratio(0.0),
            },
            stops: black_white(),
            steps: Some(4),
        })));
        let pattern = element_markup(&html, "pattern");
        assert!(!html.contains("g-ir-s1.title-x"), "{html}");
        let widths: Vec<&str> = pattern
            .match_indices(" width=\"")
            .map(|(i, _)| &pattern[i + 8..i + 8 + pattern[i + 8..].find('"').unwrap()])
            .collect();
        // The tile, the pad, then each band's contour at its outer edge: the inner sides moved s of the way out.
        assert_eq!(widths, ["650", "2000", "2", "1.75", "1.5", "1.25"]);
        assert!(
            pattern.contains("height=\"2000\" style=\"fill:#ffffff\""),
            "{pattern}"
        );
        assert!(
            pattern.contains(
                "<rect x=\"-0.625\" y=\"-0.25\" width=\"1.25\" height=\"0.5\" style=\"fill:#000000\">"
            ),
            "{pattern}"
        );
    }

    #[test]
    fn paints_ellipse_bands_outermost_first() {
        let html = painted(ResolvedFill::Ellipse(Box::new(EllipseLowered {
            origin: p(0.5, 0.5),
            axis_x: p(0.5, 0.0),
            axis_y: p(0.0, 0.5),
            inner: Semi {
                rx: unit(0.5),
                ry: unit(0.0),
            },
            stops: black_white(),
            steps: 4,
        })));
        let pattern = element_markup(&html, "pattern");
        let radii: Vec<&str> = pattern
            .match_indices(" rx=\"")
            .map(|(i, _)| &pattern[i + 5..i + 5 + pattern[i + 5..].find('"').unwrap()])
            .collect();
        assert_eq!(radii, ["1", "0.875", "0.75", "0.625"]);
        // The pad beyond the outer ellipse is the last stop; the innermost band the first.
        assert!(
            pattern.contains("height=\"2000\" style=\"fill:#ffffff\""),
            "{pattern}"
        );
        assert!(
            pattern.contains("rx=\"0.625\" ry=\"0.25\" style=\"fill:#000000\""),
            "{pattern}"
        );
    }

    #[test]
    fn prints_steps_as_hard_stops_and_p3_stops_as_srgb_sub_stops() {
        let mut stepped = vertical(literal(black()), literal(white()));
        stepped.steps = Some(3);
        let gradient = element_markup(&painted(ResolvedFill::Linear(stepped)), "linearGradient");
        assert_eq!(gradient.matches("<stop ").count(), 6);
        assert!(gradient.contains("offset=\"0.33333\" style=\"stop-color:#000000\"></stop>\n<stop offset=\"0.33333\" style=\"stop-color:#808080\""), "{gradient}");
        let p3 = RgbColor {
            space: ColorSpace::P3,
            ..rgb(0.0823, 0.2228, 0.5373)
        };
        let gradient = element_markup(
            &painted(ResolvedFill::Linear(vertical(
                literal(rgb(0.3376, 0.7577, 1.0)),
                literal(p3),
            ))),
            "linearGradient",
        );
        assert_eq!(gradient.matches("<stop ").count(), 9);
        assert!(!gradient.contains("display-p3"), "{gradient}");
    }

    #[test]
    fn expands_a_cells_css_ramp_only_for_alpha_or_p3() {
        let mut presentation = full();
        let angled = |stops| {
            ResolvedCellFill::Angled(LinearAngled {
                angle: angle(0.0),
                stops,
            })
        };
        cell_mut(&mut presentation, 1, 1).fill = Some(angled(black_white()));
        assert!(
            slide_two_html(&presentation)
                .contains("linear-gradient(90deg, #000000 0%, #ffffff 100%)")
        );
        cell_mut(&mut presentation, 1, 1).fill = Some(angled(vec![
            stop(0.0, literal(rgba(0.0, 1.0, 0.0, 0.2))),
            stop(1.0, literal(rgba(1.0, 0.0, 1.0, 1.0))),
        ]));
        let html = slide_two_html(&presentation);
        assert!(html.contains("#80808099 50%"), "{html}");
        // A rect gradient in a cell paints its unit layer.
        cell_mut(&mut presentation, 1, 1).fill = Some(ResolvedCellFill::Fill(ResolvedFill::Rect(
            Box::new(RectLowered {
                origin: p(0.5, 0.5),
                axis_x: p(0.5, 0.0),
                axis_y: p(0.0, 0.5),
                inner: Edges {
                    l: ratio(0.0),
                    t: ratio(0.0),
                    r: ratio(0.0),
                    b: ratio(0.0),
                },
                stops: black_white(),
                steps: None,
            }),
        )));
        let emitted = emit(&presentation);
        assert_eq!(emitted.files.lint(), vec![]);
        assert!(
            emitted.files.text["slides/ir-s2.html"].contains("<pattern id=\"g-ir-s2.grid-r1c1\"")
        );
    }
}

mod shadows {
    use super::*;
    use slidock_ir::shadow::ResolvedShadow;

    fn shadow(dx: f64, dy: f64, sigma: f64) -> ResolvedShadow {
        ResolvedShadow {
            dx: pt(dx),
            dy: pt(dy),
            sigma: size(sigma),
            color: literal(rgba(0.0, 0.0, 0.0, 0.5)),
            rotate_with_shape: false,
            overflow_casts: false,
        }
    }

    fn style_of(html: &str, id: &str) -> String {
        let doc = Document::parse(html);
        attr(&doc, element(&doc, id), "style").unwrap().to_owned()
    }

    /// The last declaration of an element's style.
    fn last_decl(html: &str, id: &str) -> String {
        style_of(html, id).rsplit(';').next().unwrap().to_owned()
    }

    fn group_mut(presentation: &mut Presentation) -> &mut slidock_ir::resolved::Group {
        for candidate in elements_mut(presentation, 0) {
            if let Element::Group(group) = candidate {
                return group;
            }
        }
        panic!("no group")
    }

    #[test]
    fn an_opaque_plain_rectangle_casts_its_box_and_anything_else_what_it_draws() {
        for (fill, expected) in [
            (
                Some(solid(literal(white()))),
                "box-shadow:0pt 3.287pt 14.28pt #00000080",
            ),
            (
                Some(solid(literal(rgba(1.0, 1.0, 1.0, 0.5)))),
                "filter:drop-shadow(0pt 3pt 7.14pt #00000080)",
            ),
            (None, "filter:drop-shadow(0pt 3pt 7.14pt #00000080)"),
        ] {
            let mut presentation = full();
            let title = shape_mut(&mut presentation, "ir-s1.title");
            title.fill = fill;
            title.shadow = Some(shadow(0.0, 3.287, 7.14));
            let html = slide_one_html(&presentation);
            assert_eq!(last_decl(&html, "ir-s1.title"), expected);
            assert!(!html.contains("transform:"));
        }
        // Text overflowing the opaque box casts too (LibreOffice): what it draws, its glyphs included.
        let mut presentation = full();
        let title = shape_mut(&mut presentation, "ir-s1.title");
        assert!(title.text.is_some());
        title.fill = Some(solid(literal(white())));
        title.shadow = Some(ResolvedShadow {
            overflow_casts: true,
            ..shadow(0.0, 3.287, 7.14)
        });
        assert_eq!(
            last_decl(&slide_one_html(&presentation), "ir-s1.title"),
            "filter:drop-shadow(0pt 3pt 7.14pt #00000080)"
        );
        // A geometry layer (outline, line ends) casts what it draws.
        let mut presentation = full();
        let arrow = shape_mut(&mut presentation, "ir-s1.arrow");
        arrow.frame = frame(0.0, 0.0, 200.0, 60.0);
        arrow.shadow = Some(shadow(0.0, 3.287, 7.14));
        assert_eq!(
            last_decl(&slide_one_html(&presentation), "ir-s1.arrow"),
            "filter:drop-shadow(0pt 3pt 7.14pt #00000080)"
        );
    }

    #[test]
    fn prints_a_page_offset_in_the_carriers_turned_axes() {
        for (rotation, flip_h, expected) in [
            (0.0, false, "20.25pt 0pt"),
            (90.0, false, "0pt -20.25pt"),
            (180.0, false, "-20.25pt 0pt"),
            (30.0, false, "17.25pt -9.75pt"),
            (0.0, true, "-20.25pt 0pt"),
        ] {
            let mut presentation = full();
            let arrow = shape_mut(&mut presentation, "ir-s1.arrow");
            arrow.frame = Frame {
                rotation: angle(rotation),
                flip_h,
                ..frame(0.0, 0.0, 200.0, 60.0)
            };
            arrow.shadow = Some(shadow(20.0, 0.0, 0.0));
            assert_eq!(
                last_decl(&slide_one_html(&presentation), "ir-s1.arrow"),
                format!("filter:drop-shadow({expected} 0pt #00000080)"),
                "{rotation} {flip_h}"
            );
        }
        // A shadow below a box turned 90° is to its local right.
        let mut presentation = full();
        let arrow = shape_mut(&mut presentation, "ir-s1.arrow");
        arrow.frame = Frame {
            rotation: angle(90.0),
            ..frame(0.0, 0.0, 200.0, 60.0)
        };
        arrow.shadow = Some(shadow(0.0, 3.287, 0.0));
        assert_eq!(
            last_decl(&slide_one_html(&presentation), "ir-s1.arrow"),
            "filter:drop-shadow(3pt 0pt 0pt #00000080)"
        );
    }

    #[test]
    fn prints_a_drop_shadows_offset_at_the_nearest_whole_css_px_and_a_box_shadows_as_is() {
        // Chromium floors a drop-shadow offset to whole CSS px and WebKit truncates it: 14.142 pt (18.856 px) would
        // draw at 13.5 pt; 19 px (14.25 pt) draws as printed in both.
        for (dx, dy, expected) in [
            (14.142, 14.142, "14.25pt 14.25pt"),
            (-14.142, 3.287, "-14.25pt 3pt"),
            (0.3, -0.3, "0pt 0pt"),
        ] {
            let mut presentation = full();
            let arrow = shape_mut(&mut presentation, "ir-s1.arrow");
            arrow.frame = frame(0.0, 0.0, 200.0, 60.0);
            arrow.shadow = Some(shadow(dx, dy, 0.0));
            assert_eq!(
                last_decl(&slide_one_html(&presentation), "ir-s1.arrow"),
                format!("filter:drop-shadow({expected} 0pt #00000080)")
            );
        }
        let mut presentation = full();
        let title = shape_mut(&mut presentation, "ir-s1.title");
        title.fill = Some(solid(literal(white())));
        title.shadow = Some(shadow(14.142, 14.142, 0.0));
        assert_eq!(
            last_decl(&slide_one_html(&presentation), "ir-s1.title"),
            "box-shadow:14.142pt 14.142pt 0pt #00000080"
        );
    }

    #[test]
    fn turns_a_shadow_that_rotates_with_its_shape_by_its_groups_too() {
        let mut presentation = full();
        let group = group_mut(&mut presentation);
        group.frame.rotation = angle(60.0);
        let Element::Shape(dot) = &mut group.children[0] else {
            panic!("the diamond")
        };
        dot.frame.rotation = angle(30.0);
        dot.shadow = Some(shadow(20.0, 0.0, 0.0));
        let page_fixed = slide_one_html(&presentation);
        // 90° on the page: the page's +x is the diamond's local −y.
        assert_eq!(
            last_decl(&page_fixed, "ir-s1.group.dot"),
            "filter:drop-shadow(0pt -20.25pt 0pt #00000080)"
        );
        let group = group_mut(&mut presentation);
        let Element::Shape(dot) = &mut group.children[0] else {
            panic!("the diamond")
        };
        dot.shadow.as_mut().unwrap().rotate_with_shape = true;
        assert_eq!(
            last_decl(&slide_one_html(&presentation), "ir-s1.group.dot"),
            "filter:drop-shadow(20.25pt 0pt 0pt #00000080)"
        );
    }

    #[test]
    fn a_group_casts_its_composite_and_a_video_its_poster_rectangle() {
        let mut presentation = full();
        group_mut(&mut presentation).shadow = Some(shadow(0.0, 3.287, 7.14));
        video_mut(&mut presentation).shadow = Some(shadow(0.0, 3.287, 7.14));
        let output = emit(&presentation);
        let html = &output.files.text["slides/ir-s1.html"];
        assert_eq!(
            last_decl(html, "ir-s1.group"),
            "filter:drop-shadow(0pt 3pt 7.14pt #00000080)"
        );
        assert!(!style_of(html, "ir-s1.group.dot").contains("shadow"));
        assert_eq!(
            last_decl(html, "ir-s1.video"),
            "box-shadow:0pt 3.287pt 14.28pt #00000080"
        );
        assert_eq!(output.files.lint(), vec![]);
        // A video without a poster draws a marker, which casts nothing.
        video_mut(&mut presentation).poster = None;
        assert!(!style_of(&slide_one_html(&presentation), "ir-s1.video").contains("shadow"));
    }

    #[test]
    fn a_shadowed_picture_keeps_its_box_and_clips_in_a_child_its_opacity_on_the_box() {
        let mut presentation = full();
        picture_mut(&mut presentation, "ir-s1.photo").shadow = Some(shadow(0.0, 3.287, 7.14));
        let output = emit(&presentation);
        let html = &output.files.text["slides/ir-s1.html"];
        let doc = Document::parse(html);
        let root = element(&doc, "ir-s1.photo");
        let style = attr(&doc, root, "style").unwrap();
        assert!(
            style.ends_with(";opacity:0.5;filter:drop-shadow(0pt 3pt 7.14pt #00000080)"),
            "{style}"
        );
        assert!(
            !style.contains("overflow") && !style.contains("clip-path"),
            "{style}"
        );
        assert!(attr(&doc, root, "data-crop").is_some());
        let children = common::children(&doc, root);
        assert_eq!(children.len(), 1);
        let clip = children[0];
        assert!(doc.attribute(clip, "data-sd-clip").is_some());
        let clip_style = attr(&doc, clip, "style").unwrap();
        assert!(
            clip_style
                .starts_with("position:absolute;inset:0;overflow:hidden;clip-path:path(\"M0 60C"),
            "{clip_style}"
        );
        let image = common::descendants(&doc, clip)
            .into_iter()
            .find(|n| doc.tag_name(*n) == "img")
            .unwrap();
        assert!(!attr(&doc, image, "style").unwrap().contains("opacity"));
        assert_eq!(output.files.lint(), vec![]);

        // An opaque uncut picture with a shadow is no bare `<img>`: its clip would cut the shadow.
        let photo = picture_mut(&mut presentation, "ir-s1.photo");
        photo.crop = None;
        photo.clip = None;
        photo.opacity = None;
        let html = slide_one_html(&presentation);
        let doc = Document::parse(&html);
        let root = element(&doc, "ir-s1.photo");
        assert_eq!(doc.tag_name(root), "div");
        assert_eq!(
            attr(&doc, root, "style"),
            Some(
                "position:absolute;left:36pt;top:300pt;width:120pt;height:90pt;filter:drop-shadow(0pt 3pt 7.14pt #00000080)"
            )
        );
        let clip = common::children(&doc, root)[0];
        assert_eq!(
            attr(&doc, clip, "style"),
            Some("position:absolute;inset:0;overflow:hidden")
        );
    }

    #[test]
    fn a_run_casts_a_text_shadow_in_its_text_boxs_axes() {
        let shadowed = |presentation: &mut Presentation| {
            props_of(&mut title_paragraph(presentation).inlines[0]).shadow =
                Some(shadow(20.0, 0.0, 1.0));
        };
        let span_style = |presentation: &Presentation| {
            let html = slide_one_html(presentation);
            tags_with(&html, "span", "text-shadow")
                .first()
                .cloned()
                .unwrap_or_default()
        };
        let mut presentation = full();
        shadowed(&mut presentation);
        assert!(
            span_style(&presentation).contains("text-shadow:20pt 0pt 2pt #00000080\""),
            "{}",
            span_style(&presentation)
        );
        // The text box undoes its shape's mirror: the run's axes are the page's.
        let mut mirrored = full();
        shadowed(&mut mirrored);
        shape_mut(&mut mirrored, "ir-s1.title").frame.flip_h = true;
        assert!(span_style(&mirrored).contains("text-shadow:20pt 0pt 2pt"));
        // Text that flips with its frame draws mirrored: its shadow still falls to the page's right.
        let title = shape_mut(&mut mirrored, "ir-s1.title");
        title.text.as_mut().unwrap().flips_with_frame = true;
        assert!(span_style(&mirrored).contains("text-shadow:-20pt 0pt 2pt"));
        // A turned shape turns its text's axes.
        let mut turned = full();
        shadowed(&mut turned);
        shape_mut(&mut turned, "ir-s1.title").frame.rotation = angle(90.0);
        assert!(span_style(&turned).contains("text-shadow:0pt -20pt 2pt"));
    }

    #[test]
    fn documents_without_shadows_print_none() {
        let output = emit(&full());
        for html in output.files.text.values() {
            for printed in ["drop-shadow", "text-shadow", "data-sd-clip"] {
                assert!(!html.contains(printed), "{printed}");
            }
        }
    }
}
