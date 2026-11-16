//! The diagnostics sink and order, the visitors and the frontend registry.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use slidock_core::{FrontendRegistry, RegistryError};
use slidock_core_api::visit::{
    TextSite, map_elements, map_text_bodies, slide_shows, walk_elements,
};
use slidock_core_api::{ContainerOrder, Sink, has_errors, sort_diagnostics};
use slidock_ir::diagnostics::{
    Diagnostic, DiagnosticCode, DiagnosticSink, DiagnosticSite, Severity, Stage, codes,
    make_diagnostic,
};
use slidock_ir::formats::{FileFormat, SourceFormat};
use slidock_ir::source::Element;
use support::*;

fn diag(
    code: DiagnosticCode,
    container: Option<&str>,
    node: Option<&str>,
    data: &[(&'static str, &str)],
) -> Diagnostic {
    let mut site = DiagnosticSite::new()
        .container(container.map(id))
        .node(node.map(id));
    for (key, value) in data {
        site = site.data(*key, *value);
    }
    make_diagnostic(code, Stage::Verify, "test", site)
}

#[test]
fn the_fixtures_are_valid_sources_whose_resolved_invariants_hold() {
    let options = slidock_ir::verify::VerifyOptions::default();
    for fixture in [full_json(), minimal_json()] {
        assert_eq!(
            slidock_ir::verify::verify_source_json(&fixture, &options),
            Vec::<Diagnostic>::new()
        );
        assert!(slidock_ir::verify::verify_resolved_json(&fixture, &options).is_ok());
    }
}

#[test]
fn stamps_stage_and_emitter_and_takes_the_severity_from_the_registry() {
    let mut out = Vec::new();
    Sink::new(&mut out, Stage::Pass, "cascade-text").report(
        codes::IR0908,
        DiagnosticSite::new()
            .container(id("ir-s1"))
            .data("pass", "p")
            .data("error", "e"),
    );
    let d = &out[0];
    assert_eq!(
        (d.stage, d.emitter.as_str(), d.severity),
        (Stage::Pass, "cascade-text", Severity::Error)
    );
    assert_eq!(d.container, Some(id("ir-s1")));
    assert!(has_errors(&out));
    assert!(!has_errors(&[diag(
        codes::IR0910,
        None,
        None,
        &[("type", "chart")]
    )]));
}

#[test]
fn sorts_presentation_level_first_then_masters_layouts_and_slides_in_show_order_then_code_and_node()
{
    let order = ContainerOrder::of(Some(&full()));
    let sorted = sort_diagnostics(
        vec![
            diag(
                codes::IR0003,
                Some("ir-s1"),
                Some("ir-s1.b"),
                &[("kind", "media"), ("target", "x")],
            ),
            diag(codes::IR0002, Some("ir-s2"), None, &[("id", "a")]),
            diag(
                codes::IR0003,
                Some("ir-s1"),
                Some("ir-s1.a"),
                &[("kind", "media"), ("target", "x")],
            ),
            diag(codes::IR0002, Some("ir-layout"), None, &[("id", "a")]),
            diag(codes::IR0006, None, None, &[("count", "301")]),
            diag(codes::IR0002, Some("ir-master"), None, &[("id", "a")]),
        ],
        &order,
    );
    let keys: Vec<(Option<String>, &str, Option<String>)> = sorted
        .iter()
        .map(|d| {
            (
                d.container.as_ref().map(|c| c.to_string()),
                d.code.code(),
                d.node.as_ref().map(|n| n.to_string()),
            )
        })
        .collect();
    let some = |s: &str| Some(s.to_owned());
    assert_eq!(
        keys,
        [
            (None, "IR0006", None),
            (some("ir-master"), "IR0002", None),
            (some("ir-layout"), "IR0002", None),
            (some("ir-s1"), "IR0003", some("ir-s1.a")),
            (some("ir-s1"), "IR0003", some("ir-s1.b")),
            (some("ir-s2"), "IR0002", None),
        ]
    );
}

#[test]
fn ranks_containers_the_presentation_does_not_hold_after_its_own_and_by_id() {
    let order = ContainerOrder::of(Some(&minimal()));
    let sorted = sort_diagnostics(
        vec![
            diag(codes::IR0002, Some("ir-zz"), None, &[("id", "a")]),
            diag(codes::IR0002, Some("ir-aa"), None, &[("id", "a")]),
            diag(codes::IR0002, Some("ir-s1"), None, &[("id", "a")]),
        ],
        &order,
    );
    let ids: Vec<String> = sorted
        .iter()
        .map(|d| d.container.as_ref().unwrap().to_string())
        .collect();
    assert_eq!(ids, ["ir-s1", "ir-aa", "ir-zz"]);
}

#[test]
fn compares_data_by_utf16_code_unit() {
    // U+FF21 sorts after U+1F600 by code point, before it by UTF-16 code unit (0xFF21 > 0xD83D).
    let order = ContainerOrder::default();
    let wide = diag(
        codes::MID5401,
        Some("ir-s1"),
        None,
        &[("family", "\u{FF21}"), ("binding", "x")],
    );
    let astral = diag(
        codes::MID5401,
        Some("ir-s1"),
        None,
        &[("family", "\u{1F600}"), ("binding", "x")],
    );
    let sorted = sort_diagnostics(vec![wide.clone(), astral.clone()], &order);
    assert_eq!(sorted, [astral.clone(), wide.clone()]);
    assert_eq!(
        sort_diagnostics(vec![astral.clone(), wide.clone()], &order),
        [astral, wide]
    );
}

#[test]
fn merges_identical_diagnostics_with_a_count_whatever_the_input_order() {
    let order = ContainerOrder::of(Some(&full()));
    let a = diag(codes::IR0002, Some("ir-s1"), None, &[("id", "x")]);
    let b = diag(codes::IR0002, Some("ir-s1"), None, &[("id", "y")]);
    let merged = sort_diagnostics(vec![a.clone(), b.clone(), a.clone(), a.clone()], &order);
    let counts: Vec<(String, Option<u32>)> = merged
        .iter()
        .map(|d| (data(d, "id").unwrap(), d.count))
        .collect();
    assert_eq!(counts, [("x".to_owned(), Some(3)), ("y".to_owned(), None)]);
    assert_eq!(
        sort_diagnostics(vec![a.clone(), a.clone(), b, a.clone()], &order),
        merged
    );
    // Merged counts add up.
    let again = sort_diagnostics([merged.clone(), merged].concat(), &order);
    assert_eq!(
        again.iter().map(|d| d.count).collect::<Vec<_>>(),
        [Some(6), Some(2)]
    );
}

#[test]
fn walks_group_children_and_foreign_fallbacks_depth_first() {
    let presentation = full();
    let elements = &presentation.slides[0].elements;
    let ids: Vec<String> = walk_elements(elements)
        .map(|e| e.id().to_string())
        .collect();
    assert_eq!(ids[..3], ["ir-s1.title", "ir-s1.body", "ir-s1.arrow"]);
    assert_eq!(
        ids[4..],
        [
            "ir-s1.group",
            "ir-s1.group.dot",
            "ir-s1.group.inner",
            "ir-s1.group.inner.logo",
            "ir-s1.table",
            "ir-s1.table.cell"
        ]
    );
    assert!(ids.len() > elements.len());
}

#[test]
fn maps_bottom_up_and_reports_unchanged_trees() {
    let presentation = full();
    let elements = &presentation.slides[0].elements;
    assert_eq!(map_elements(elements, &mut |_| None), None);
    let mut seen = Vec::new();
    let renamed = map_elements(elements, &mut |element| {
        seen.push(element.id().to_string());
        match element {
            Element::Picture(picture) => {
                let mut picture = picture.clone();
                picture.alt = Some("changed".into());
                Some(Element::Picture(picture))
            }
            _ => None,
        }
    })
    .unwrap();
    let pictures: Vec<&Element> = walk_elements(&renamed)
        .filter(|e| matches!(e, Element::Picture(_)))
        .collect();
    assert_eq!(pictures.len(), 2);
    assert!(pictures.iter().all(|e| e.alt() == Some("changed")));
    assert_eq!(seen.len(), walk_elements(elements).count());
    // Children are mapped before their parent sees them.
    let mut parents = Vec::new();
    map_elements(elements, &mut |element| {
        if let Element::Group(group) = element {
            parents.push(
                group
                    .children
                    .iter()
                    .map(|c| c.alt().map(str::to_owned))
                    .collect::<Vec<_>>(),
            );
        }
        match element {
            Element::Shape(shape) => {
                let mut shape = shape.clone();
                shape.alt = Some("seen".into());
                Some(Element::Shape(shape))
            }
            _ => None,
        }
    });
    assert_eq!(parents[1], [Some("seen".to_owned()), None]);
}

#[test]
fn maps_the_text_bodies_an_element_draws_itself() {
    let presentation = full();
    let grid = &presentation.slides[1].elements[1];
    let mut sites = Vec::new();
    let mapped = map_text_bodies(grid, &mut |text, site| {
        sites.push(site);
        let mut text = text.clone();
        text.paragraphs.clear();
        Some(text)
    })
    .unwrap();
    assert_eq!(sites, [TextSite::Cell, TextSite::Cell, TextSite::Cell]);
    let Element::Table(table) = mapped else {
        panic!()
    };
    assert!(
        table
            .rows
            .iter()
            .flat_map(|r| r.cells.iter().flatten())
            .all(|c| c.text.paragraphs.is_empty())
    );
    assert!(table.rows[0].cells[1].is_none());
    assert_eq!(map_text_bodies(grid, &mut |_, _| None), None);
    let note = &presentation.slides[1].elements[0];
    let mut shape_sites = Vec::new();
    map_text_bodies(note, &mut |_, site| {
        shape_sites.push(site);
        None
    });
    assert_eq!(shape_sites, [TextSite::Shape]);
    let photo = &presentation.slides[0].elements[3];
    assert_eq!(
        map_text_bodies(photo, &mut |text, _| Some(text.clone())),
        None
    );
}

#[test]
fn lists_what_each_slide_shows_behind_itself() {
    let shows = slide_shows(&full());
    assert_eq!(shows[&id("ir-s1")], [id("ir-layout"), id("ir-master")]);
    assert!(shows[&id("ir-s2")].is_empty());
    let mut hidden_master = full_json();
    hidden_master["layouts"]["ir-layout"]["showMaster"] = serde_json::json!(false);
    assert_eq!(
        slide_shows(&typed(hidden_master))[&id("ir-s1")],
        [id("ir-layout")]
    );
}

#[test]
fn sniffs_in_registration_order_and_refuses_two_frontends_for_one_format() {
    let pdf = SourceFormat::File(FileFormat::Pdf);
    let pptx = SourceFormat::File(FileFormat::Pptx);
    let registry = FrontendRegistry::new(vec![
        Box::new(FakeFrontend::sniffing(pdf, false)),
        Box::new(FakeFrontend::sniffing(pptx, true)),
        Box::new(FakeFrontend::sniffing(SourceFormat::Ir, true)),
    ])
    .unwrap();
    let hints = slidock_core_api::SniffHints {
        name: "x",
        zip_names: None,
    };
    assert_eq!(registry.sniff(&[], &hints).map(|f| f.format()), Some(pptx));
    assert_eq!(
        registry.get(SourceFormat::Ir).map(|f| f.format()),
        Some(SourceFormat::Ir)
    );
    assert!(registry.get(SourceFormat::File(FileFormat::Odp)).is_none());
    let error = FrontendRegistry::new(vec![
        Box::new(FakeFrontend::sniffing(pdf, true)),
        Box::new(FakeFrontend::sniffing(pdf, true)),
    ])
    .unwrap_err();
    assert_eq!(error, RegistryError::Duplicate(pdf));
    assert_eq!(error.to_string(), "Two frontends read pdf");
}
