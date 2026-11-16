//! The package and the containers: sniffing, refusals, references, the page order and ids, master pages as layouts,
//! the slide size, backgrounds, display flags, transitions and animations, and an unreadable part failing only what
//! it holds.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use serde_json::{Value, json};
use slidock_core_api::{Frontend, SniffHints};
use slidock_odp::OdpFrontend;
use support::odp::{
    NS, ODP_MIMETYPE, OdpSpec, PageSpec, files, odp, page_style, replace, unzip, zip_entries,
    zip_with,
};
use support::{compile, fixtures, js, lower, refusal};

fn solid(r: f64, g: f64, b: f64) -> Value {
    js(
        &json!({ "kind": "solid", "color": { "kind": "rgb", "space": "srgb", "r": r, "g": g, "b": b, "a": 1 } }),
    )
}

fn head(bytes: &[u8]) -> &[u8] {
    &bytes[..bytes.len().min(4096)]
}

#[test]
fn sniffs_a_stored_mimetype_entry_naming_a_presentation_at_the_head_of_a_zip() {
    let hints = SniffHints::default();
    assert!(OdpFrontend.sniff(head(&fixtures::basic()), &hints));
    let pptx = zip_with(&[
        ("[Content_Types].xml", b"<Types/>", true),
        ("ppt/presentation.xml", b"<p/>", true),
    ]);
    assert!(!OdpFrontend.sniff(head(&pptx), &hints));
    let text = zip_with(&[(
        "mimetype",
        b"application/vnd.oasis.opendocument.text",
        false,
    )]);
    assert!(!OdpFrontend.sniff(head(&text), &hints));
    assert!(!OdpFrontend.sniff(&[0; 10], &hints));
}

#[test]
fn refuses_what_is_not_an_opendocument_presentation() {
    assert_eq!(refusal(b"not a zip at all, no").code.code(), "OD9001");
    let content_first = zip_with(&[
        ("content.xml", b"<x/>", false),
        ("mimetype", ODP_MIMETYPE.as_bytes(), false),
    ]);
    assert_eq!(refusal(&content_first).code.code(), "OD9002");
    let text = refusal(&odp(&OdpSpec {
        mimetype: Some("application/vnd.oasis.opendocument.text".to_owned()),
        ..OdpSpec::pages(vec![PageSpec::new()])
    }));
    assert_eq!(text.code.code(), "OD9002");
    assert_eq!(
        serde_json::to_value(&text.data).unwrap()["reason"],
        "mimetype is application/vnd.oasis.opendocument.text"
    );
    let compressed = zip_with(&[
        ("mimetype", ODP_MIMETYPE.as_bytes(), true),
        ("content.xml", b"<x/>", true),
    ]);
    assert_eq!(
        serde_json::to_value(&refusal(&compressed).data).unwrap()["reason"],
        "mimetype is compressed"
    );
    let mut without = files(&OdpSpec::pages(vec![PageSpec::new()]));
    without.retain(|(name, _)| name != "content.xml");
    assert_eq!(refusal(&zip_entries(&without)).code.code(), "OD9001");
    let mut not_content = files(&OdpSpec::pages(vec![PageSpec::new()]));
    replace(&mut not_content, "content.xml", "<x/>");
    assert_eq!(refusal(&zip_entries(&not_content)).code.code(), "OD9001");
}

#[test]
fn names_the_app_from_meta_xml_and_hashes_the_whole_input() {
    let bytes = fixtures::basic();
    let provenance = serde_json::to_value(&lower(&bytes).lowered.provenance).unwrap();
    assert_eq!(provenance["format"], "odp");
    assert_eq!(provenance["name"], "presentation.odp");
    assert_eq!(provenance["bytes"], bytes.len());
    assert_eq!(
        provenance["app"],
        json!({ "name": "LibreOffice", "version": "7.6.4.1" })
    );
    assert_eq!(provenance["sha256"], slidock_canon::sha256_hex(&bytes));
    let bare = OdpSpec {
        generator: Some("Some Writer".to_owned()),
        ..OdpSpec::pages(vec![PageSpec::new()])
    };
    let provenance = serde_json::to_value(&lower(&odp(&bare)).lowered.provenance).unwrap();
    assert_eq!(provenance["app"], json!({ "name": "Some Writer" }));
}

#[test]
fn lists_pages_in_document_order_with_native_or_index_ids_and_hidden_flags_and_lowers_the_same_list()
 {
    let lowered = lower(&fixtures::basic());
    let listed: Vec<(&str, bool)> = lowered
        .analysis
        .slides
        .iter()
        .map(|slide| (slide.id.as_str(), slide.hidden))
        .collect();
    assert_eq!(
        listed,
        [
            ("od-p1", false),
            ("od-p2", true),
            ("od-p3", false),
            ("od-p4", false),
            ("od-p5", false),
            ("od-closing", false),
            ("od-p7", false)
        ]
    );
    assert_eq!(
        lowered.json["slides"][1]["origin"],
        json!({
            "kind": "source",
            "part": "content.xml",
            "path": "/office:document-content/office:body/office:presentation/draw:page[2]"
        })
    );
    let features: Vec<&str> = lowered
        .analysis
        .features
        .iter()
        .map(|f| f.as_str())
        .collect();
    assert_eq!(features, ["presentation.hidden-slide"]);
    assert!(lowered.analysis.media_bytes > 0);
}

#[test]
fn reads_the_slide_size_from_the_page_layout_of_the_first_pages_master_page() {
    let size = |bytes: &[u8]| js(&lower(bytes).analysis.size);
    assert_eq!(size(&fixtures::basic()), json!({ "w": 720, "h": 405 }));
    let cm = OdpSpec {
        size: Some(("25.4cm".to_owned(), "14.288cm".to_owned())),
        ..OdpSpec::pages(vec![PageSpec::new()])
    };
    assert_eq!(size(&odp(&cm)), json!({ "w": 720, "h": 405.014 }));
    let zero_width = OdpSpec {
        size: Some(("0cm".to_owned(), "1cm".to_owned())),
        ..OdpSpec::pages(vec![PageSpec::new()])
    };
    assert_eq!(
        size(&odp(&zero_width)),
        json!({ "w": 793.701, "h": 446.457 })
    );
}

#[test]
fn makes_every_master_page_a_layout_of_its_own_and_links_each_page_to_its_own() {
    let lowered = lower(&fixtures::basic());
    let presentation = &lowered.json;
    let layouts: Vec<&String> = presentation["layouts"]
        .as_object()
        .unwrap()
        .keys()
        .collect();
    let order: Vec<&str> = lowered
        .presentation
        .layouts
        .keys()
        .map(|id| id.as_str())
        .collect();
    assert_eq!(
        order,
        ["od-m.Title_20_Slide", "od-m.Content", "od-m.Unused"]
    );
    assert_eq!(layouts.len(), 3);
    assert_eq!(
        presentation["layouts"]["od-m.Title_20_Slide"]["name"],
        "Title Slide"
    );
    assert_eq!(
        presentation["layouts"]["od-m.Title_20_Slide"]["showMaster"],
        true
    );
    assert!(
        presentation["layouts"]["od-m.Content"]
            .get("master")
            .is_none()
    );
    assert_eq!(presentation["masters"], json!({}));
    assert_eq!(presentation["themes"], json!({}));
    let links: Vec<Value> = presentation["slides"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.get("layout").cloned().unwrap_or(Value::Null))
        .collect();
    assert_eq!(
        links,
        [
            json!("od-m.Title_20_Slide"),
            json!("od-m.Content"),
            json!("od-m.Content"),
            json!("od-m.Content"),
            json!("od-m.Content"),
            json!("od-m.Content"),
            Value::Null
        ]
    );
}

#[test]
fn reads_backgrounds_solid_through_a_parent_style_gradients_as_odf_sources_bitmaps_hatches() {
    let lowered = lower(&fixtures::basic());
    let presentation = &lowered.json;
    assert_eq!(
        presentation["layouts"]["od-m.Title_20_Slide"]["background"],
        solid(1.0, 1.0, 1.0)
    );
    // ODF 1.2 by LibreOffice 7.6: the unitless angle is in degrees; the end intensity darkens the end colour.
    assert_eq!(
        presentation["layouts"]["od-m.Content"]["background"],
        json!({
            "kind": "odf",
            "style": "linear",
            "angle": 300,
            "border": 0.2,
            "center": { "x": 0, "y": 0 },
            "stops": [
                { "pos": 0, "color": { "kind": "rgb", "space": "srgb", "r": 1, "g": 0, "b": 0, "a": 1 } },
                { "pos": 1, "color": { "kind": "rgb", "space": "srgb", "r": 0, "g": 0, "b": 0.5, "a": 1 } }
            ]
        })
    );
    let slides = presentation["slides"].as_array().unwrap();
    assert!(slides[0].get("background").is_none());
    assert_eq!(slides[2]["background"], solid(0.2, 0.4, 0.6));
    assert_eq!(slides[3]["background"]["kind"], "image");
    assert_eq!(slides[3]["background"]["mode"], "stretch");
    assert_eq!(slides[4]["background"], solid(0.93333, 0.93333, 0.93333));
    assert_eq!(slides[5]["background"]["kind"], "odf");
    assert_eq!(slides[5]["background"]["style"], "ellipsoid");
    assert_eq!(
        slides[5]["background"]["center"],
        json!({ "x": 0.25, "y": 0.75 })
    );
    assert_eq!(slides[6]["background"], solid(1.0, 1.0, 1.0));
    let codes: Vec<(String, Option<String>)> = lowered.codes();
    let expected: Vec<(String, Option<String>)> = [
        ("OD9102", "od-p5"),
        ("OD9401", "od-closing"),
        ("OD9402", "od-closing"),
        ("OD9007", "od-p7"),
    ]
    .iter()
    .map(|(c, n)| ((*c).to_owned(), Some((*n).to_owned())))
    .collect();
    assert_eq!(codes, expected);
    assert_eq!(lowered.features("od-p3"), ["slide.background"]);
}

#[test]
fn hides_the_master_pages_objects_where_a_page_says_so_and_drops_transitions_and_animations() {
    let lowered = lower(&fixtures::basic());
    let shown: Vec<bool> = lowered
        .presentation
        .slides
        .iter()
        .map(|slide| slide.show_layout)
        .collect();
    assert_eq!(shown, [true, true, false, true, true, true, true]);
    assert_eq!(
        lowered.features("od-closing"),
        ["animation", "slide.background", "slide.transition"]
    );
    assert!(lowered.presentation.slides[5].transition.is_none());
    assert!(lowered.presentation.slides[5].timeline.is_empty());
}

#[test]
fn fails_every_layout_when_styles_xml_cannot_be_read_and_still_compiles_the_slides() {
    let mut parts = unzip(&fixtures::basic());
    replace(&mut parts, "styles.xml", "<office:document-styles");
    let bytes = zip_entries(&parts);
    let lowered = lower(&bytes);
    assert_eq!(
        lowered.json["layouts"]["od-m.Content"]["failed"],
        json!({ "code": "OD9003" })
    );
    assert_eq!(
        lowered.json["layouts"]["od-m.Content"]["elements"],
        json!([])
    );
    let failed: Vec<&str> = lowered
        .diagnostics
        .iter()
        .filter(|d| d.code.code() == "OD9003")
        .map(|d| d.container.as_ref().unwrap().as_str())
        .collect();
    assert_eq!(
        failed,
        ["od-m.Title_20_Slide", "od-m.Content", "od-m.Missing"]
    );
    let out = compile(&bytes);
    assert_eq!(out.errors(), ["OD9003"; 3]);
    let presentation = out.presentation.unwrap();
    assert_eq!(presentation.slides.len(), 7);
    let content = presentation
        .layouts
        .iter()
        .find(|(id, _)| id.as_str() == "od-m.Content")
        .unwrap()
        .1;
    assert_eq!(
        serde_json::to_value(content).unwrap()["failed"],
        json!({ "code": "OD9003" })
    );
}

fn red_page() -> Vec<(String, Vec<u8>)> {
    files(&OdpSpec {
        automatic_styles: page_style("dp1", r##"draw:fill="solid" draw:fill-color="#ff0000""##),
        ..OdpSpec::pages(vec![PageSpec::new().style("dp1")])
    })
}

fn content_of(parts: &[(String, Vec<u8>)]) -> String {
    String::from_utf8(
        parts
            .iter()
            .find(|(name, _)| name == "content.xml")
            .unwrap()
            .1
            .clone(),
    )
    .unwrap()
}

#[test]
fn reads_attributes_under_any_prefix_a_document_binds_to_the_odf_namespaces() {
    let mut parts = red_page();
    let renamed = NS
        .replace("xmlns:draw=", "xmlns:d=")
        .replace("xmlns:style=", "xmlns:s=");
    let content = content_of(&parts)
        .replace(NS, &renamed)
        .replace("<draw:", "<d:")
        .replace("</draw:", "</d:")
        .replace("<style:", "<s:")
        .replace("</style:", "</s:")
        .replace(" draw:", " d:")
        .replace(" style:", " s:");
    replace(&mut parts, "content.xml", content);
    let lowered = lower(&zip_entries(&parts));
    assert_eq!(
        lowered.json["slides"][0]["background"],
        solid(1.0, 0.0, 0.0)
    );
}

#[test]
fn never_reads_an_attribute_of_another_namespace_bound_to_a_canonical_prefix() {
    let mut parts = red_page();
    // The page style's colour moves to a namespace of the writer's own, which binds the prefix `draw` there.
    let content = content_of(&parts).replace(
        r##"draw:fill-color="#ff0000""##,
        r##"xmlns:draw="urn:example:other" draw:fill-color="#ff0000""##,
    );
    replace(&mut parts, "content.xml", content);
    let lowered = lower(&zip_entries(&parts));
    assert_ne!(
        lowered.json["slides"][0]["background"],
        solid(1.0, 0.0, 0.0)
    );
}

#[test]
fn compiles_to_a_lint_clean_file_set() {
    let out = compile(&fixtures::basic());
    assert_eq!(out.errors(), Vec::<&str>::new());
    let documents = out.documents.unwrap();
    assert_eq!(documents.lint(), vec![]);
    assert!(documents.text["presentation.json"].contains("\"format\": \"odp\""));
}

#[test]
fn hands_the_fonts_the_package_embeds_to_the_host_and_records_none_in_the_ir() {
    let face = concat!(
        r#"<style:font-face style:name="Barlow" svg:font-family="&apos;Barlow&apos;"><svg:font-face-src>"#,
        r#"<svg:font-face-uri xlink:href="Fonts/Barlow-Bold.ttf" xlink:type="simple" loext:font-style="normal" "#,
        r#"loext:font-weight="bold"><svg:font-face-format svg:string="truetype"/></svg:font-face-uri>"#,
        r#"<svg:font-face-uri xlink:href="Fonts/Barlow-Italic.ttf" loext:font-style="italic"/>"#,
        r#"<svg:font-face-uri xlink:href="Fonts/missing.ttf"/>"#,
        r#"<svg:font-face-uri xlink:href="https://example.com/x.ttf"/>"#,
        "</svg:font-face-src></style:font-face>"
    );
    let bytes = odp(&OdpSpec {
        font_faces: face.to_owned(),
        ..OdpSpec::pages(vec![PageSpec::new()])
    }
    .part("Fonts/Barlow-Bold.ttf", b"bold face".to_vec())
    .part("Fonts/Barlow-Italic.ttf", b"italic face".to_vec()));
    let lowered = lower(&bytes);
    // content.xml and styles.xml both declare the face; each file is handed over once.
    let fonts: Vec<(String, u32, String, usize)> = lowered
        .lowered
        .fonts
        .iter()
        .map(|font| {
            // A deflated entry is handed on compressed and inflated on first read.
            let len = match &font.source {
                slidock_core_api::MediaSource::Reader(reader) => {
                    reader.read_at(0, reader.len() as usize).unwrap().len()
                }
                other => panic!("{other:?}"),
            };
            (
                font.family.clone(),
                font.faces[0].weight,
                format!("{:?}", font.faces[0].style),
                len,
            )
        })
        .collect();
    assert_eq!(
        fonts,
        [
            ("Barlow".to_owned(), 700, "Normal".to_owned(), 9),
            ("Barlow".to_owned(), 400, "Italic".to_owned(), 11)
        ]
    );
    assert_eq!(lowered.json["fonts"], json!({}));
}

#[test]
fn a_stored_font_is_a_range_of_the_input_and_an_undeclared_face_is_left_to_the_file() {
    let face = concat!(
        r#"<style:font-face style:name="Lato" svg:font-family="Lato"><svg:font-face-src>"#,
        r#"<svg:font-face-uri xlink:href="Fonts/Lato.ttf" xlink:type="simple"/>"#,
        "</svg:font-face-src></style:font-face>"
    );
    let spec = OdpSpec {
        font_faces: face.to_owned(),
        ..OdpSpec::pages(vec![PageSpec::new()])
    }
    .part("Fonts/Lato.ttf", b"lato face".to_vec());
    let built = files(&spec);
    let entries: Vec<(&str, &[u8], bool)> = built
        .iter()
        .map(|(path, bytes)| {
            (
                path.as_str(),
                bytes.as_slice(),
                path != "mimetype" && path != "Fonts/Lato.ttf",
            )
        })
        .collect();
    let bytes = zip_with(&entries);
    let fonts = lower(&bytes).lowered.fonts;
    assert_eq!(fonts.len(), 1);
    assert!(
        fonts[0].faces.is_empty(),
        "no loext attribute: the file says which face it is"
    );
    match fonts[0].source {
        slidock_core_api::MediaSource::InputRange { offset, len } => {
            assert_eq!(
                &bytes[offset as usize..(offset + len) as usize],
                b"lato face"
            );
        }
        ref other => panic!("{other:?}"),
    }
}
