//! Shadows through the whole pipeline, on a presentation of our own: a placeholder inherits its layout placeholder's
//! shadow unless it states `null`, a theme shadow colour resolves, a run shadow cascades from its list style, each
//! carrier prints its CSS shadow, and the loss check tells a dropped text shadow from a dropped box shadow.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "test code")]

mod support;

use serde_json::{Value, json};
use slidock::{Compiled, LossCheckInput, loss_check, supported_features};
use slidock_ir::diagnostics::{DiagnosticSite, Severity, Stage, codes, make_diagnostic};
use slidock_ir::features::FeatureId;
use slidock_ir::ids::NodeId;
use slidock_ir::resolved::Element;
use slidock_ir::text::Inline;
use support::*;

fn shadow(dy: f64, color: Value) -> Value {
    json!({ "dx": 0, "dy": dy, "sigma": 7.14, "color": color })
}

fn black(a: f64) -> Value {
    json!({ "kind": "rgb", "space": "srgb", "r": 0, "g": 0, "b": 0, "a": a })
}

fn with(mut element: Value, key: &str, value: Value) -> Value {
    element[key] = value;
    element
}

fn placeholder(id: &str, inherits: Option<&str>, extra: Value) -> Value {
    let mut element = json!({
        "id": id, "origin": agent(), "kind": "shape", "placeholder": { "role": "body" },
        "geometry": { "kind": "preset", "name": "rect", "adjust": {} },
        "fill": { "kind": "solid", "color": rgb(0.2, 0.4, 0.6) }
    });
    if let Some(inherits) = inherits {
        element["inherits"] = json!(inherits);
    } else {
        element["frame"] = frame(20.0, 20.0, 200.0, 100.0);
    }
    for (key, value) in extra.as_object().unwrap() {
        element[key] = value.clone();
    }
    element
}

/// A layout body placeholder with a shadow; on the slide a placeholder inheriting it, one stating none, an opaque
/// rectangle, a translucent one, and a text box whose run shadow comes from its list style.
fn deck() -> Value {
    let accent = json!({ "kind": "theme", "slot": "accent1", "mods": [] });
    let mut text = text_shape("ir-s1.text", frame(20.0, 300.0, 400.0, 60.0), "Shadow");
    text["text"]["listStyle"] = json!({ "0": { "run": { "shadow": shadow(2.0, black(0.5)) } } });
    let mut p = presentation(vec![slide_on(
        "ir-s1",
        "ir-l",
        vec![
            placeholder("ir-s1.body", Some("ir-l.body"), json!({})),
            with(
                placeholder("ir-s1.none", Some("ir-l.body"), json!({})),
                "shadow",
                Value::Null,
            ),
            with(
                rect("ir-s1.opaque", frame(300.0, 20.0, 100.0, 100.0)),
                "shadow",
                shadow(3.287, accent),
            ),
            with(
                with(
                    rect("ir-s1.glass", frame(420.0, 20.0, 100.0, 100.0)),
                    "fill",
                    json!({ "kind": "solid", "color": {
                        "kind": "rgb", "space": "srgb", "r": 0.2, "g": 0.4, "b": 0.6, "a": 0.5 } }),
                ),
                "shadow",
                shadow(3.287, black(0.5)),
            ),
            text,
        ],
    )]);
    p["masters"] = json!({ "ir-m": master("ir-m", vec![]) });
    p["layouts"] = json!({ "ir-l": layout("ir-l", "ir-m", vec![
        placeholder("ir-l.body", None, json!({ "shadow": shadow(20.0, black(0.5)) }))
    ]) });
    p
}

fn element<'a>(compiled: &'a Compiled, id: &str) -> &'a Element {
    let slide = &compiled.result.presentation.as_ref().unwrap().slides[0];
    slide
        .body
        .elements()
        .iter()
        .find(|element| element.id().as_str() == id)
        .unwrap()
}

/// The `style` of the element `id` in the slide document.
fn style_of(html: &str, id: &str) -> String {
    let at = html.find(&format!("data-el=\"{id}\"")).unwrap();
    let rest = &html[at..];
    let start = rest.find("style=\"").unwrap() + 7;
    let end = rest[start..].find('"').unwrap();
    rest[start..start + end].to_owned()
}

#[test]
fn inherits_resolves_cascades_and_prints_every_shadow() {
    let compiled = compile(&deck());
    let errors: Vec<_> = compiled
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .collect();
    assert!(errors.is_empty(), "{errors:?}");

    // The placeholder takes its layout placeholder's shadow; `null` stops it and is left out once inherited.
    let dy = |id: &str| element(&compiled, id).shadow().map(|s| s.dy.to_f64());
    assert_eq!(dy("ir-s1.body"), Some(20.0));
    assert_eq!(dy("ir-s1.none"), None);
    // The theme colour resolves (against Office's defaults: this presentation has no theme).
    let opaque = element(&compiled, "ir-s1.opaque").shadow().unwrap();
    assert_eq!((opaque.color.rgb().r.to_f64() * 255.0).round(), 68.0);
    // The run takes its list style's shadow.
    let Element::Shape(text) = element(&compiled, "ir-s1.text") else {
        panic!("a shape")
    };
    let paragraph = &text.text.as_ref().unwrap().paragraphs[0];
    let Inline::Run { props, .. } = &paragraph.inlines[0] else {
        panic!("a run")
    };
    assert_eq!(props.shadow.as_ref().map(|s| s.dy.to_f64()), Some(2.0));

    let documents = compiled.documents().unwrap();
    assert_eq!(documents.lint(), vec![]);
    let html = &documents.text["slides/ir-s1.html"];
    assert!(
        style_of(html, "ir-s1.body").ends_with(";box-shadow:0pt 20pt 14.28pt #00000080"),
        "{}",
        style_of(html, "ir-s1.body")
    );
    assert!(!style_of(html, "ir-s1.none").contains("shadow"));
    assert!(
        style_of(html, "ir-s1.opaque")
            .ends_with(";box-shadow:0pt 3.287pt 14.28pt var(--sd-color-accent1,#4472c4)"),
        "{}",
        style_of(html, "ir-s1.opaque")
    );
    assert!(
        style_of(html, "ir-s1.glass").ends_with(";filter:drop-shadow(0pt 3pt 7.14pt #00000080)"),
        "{}",
        style_of(html, "ir-s1.glass")
    );
    assert!(
        html.contains("text-shadow:0pt 2pt 14.28pt #00000080"),
        "{html}"
    );
}

#[test]
fn the_loss_check_tells_a_dropped_text_shadow_from_a_dropped_box_shadow() {
    // The run states its own shadow, so the source census holds text.shadow.
    let mut source = deck();
    source["slides"][0]["elements"][4]["text"]["paragraphs"][0]["inlines"][0]["props"]["shadow"] =
        shadow(2.0, black(0.5));
    let compiled = compile(&source);
    let result = &compiled.result;
    let supported = supported_features();
    let mut diagnostics = result.diagnostics.clone();
    diagnostics.extend(compiled.printed.as_ref().unwrap().backend.clone());
    let mut dropped = result.presentation.as_ref().unwrap().to_source().unwrap();
    let losses = |presentation: &slidock_ir::source::Presentation,
                  extra: &[slidock_ir::diagnostics::Diagnostic]| {
        let mut all = diagnostics.clone();
        all.extend(extra.iter().cloned());
        loss_check(&LossCheckInput {
            detected: &result.features,
            source: result.source.as_ref().unwrap(),
            presentation,
            diagnostics: &all,
            supported: &supported,
        })
        .into_iter()
        .map(|d| d.data.unwrap()["feature"].to_js_string())
        .collect::<Vec<_>>()
    };
    assert_eq!(losses(&dropped, &[]), Vec::<String>::new());

    // A pass that drops the run shadow loses text.shadow, which an effect.shadow diagnostic does not account for.
    for element in &mut dropped.slides[0].elements {
        if let slidock_ir::source::Element::Shape(shape) = element
            && let Some(text) = shape.text.as_mut()
        {
            for paragraph in &mut text.paragraphs {
                for inline in &mut paragraph.inlines {
                    if let Inline::Run { props, .. } = inline {
                        props.shadow = None;
                    }
                }
                if let Some(end) = paragraph.end_props.as_mut() {
                    end.shadow = None;
                }
            }
        }
    }
    assert_eq!(losses(&dropped, &[]), ["text.shadow"]);
    let named = |feature| {
        make_diagnostic(
            codes::IR0909,
            Stage::Pass,
            "test",
            DiagnosticSite::new()
                .container(NodeId::new("ir-s1").unwrap())
                .feature(feature),
        )
    };
    assert_eq!(
        losses(&dropped, &[named(FeatureId::EffectShadow)]),
        ["text.shadow"]
    );
    assert_eq!(
        losses(&dropped, &[named(FeatureId::TextShadow)]),
        Vec::<String>::new()
    );
}
