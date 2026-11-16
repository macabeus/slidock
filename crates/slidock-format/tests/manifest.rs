//! The document-set manifest.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod common;

use common::sample_manifest;
use serde_json::Value;
use slidock_format::manifest::{Manifest, ManifestError, read_manifest, serialize_manifest};

#[test]
fn serialises_as_canonical_json() {
    let text = serialize_manifest(&sample_manifest()).unwrap();
    assert!(text.ends_with("}\n"));
    assert!(text.starts_with("{\n  \"generator\": {\n    \"name\": \"slidock\",\n"));
    let keys: Vec<&str> = text
        .lines()
        .filter_map(|line| {
            line.strip_prefix("  \"")
                .and_then(|rest| rest.split_once('"'))
                .map(|(key, _)| key)
        })
        .collect();
    assert_eq!(
        keys,
        [
            "generator",
            "layouts",
            "size",
            "slides",
            "source",
            "themes",
            "title"
        ]
    );
    assert!(text.contains("\"durationMs\": 500,"));
    assert!(text.contains("\"folHlink\": \"#888888\""));
    assert!(text.contains("\"w\": 720"));
}

#[test]
fn reads_back_what_it_writes() {
    let manifest = sample_manifest();
    assert_eq!(
        read_manifest(&serialize_manifest(&manifest).unwrap()),
        Ok(manifest)
    );
}

#[test]
fn omits_optional_keys_that_are_absent() {
    let mut manifest = sample_manifest();
    manifest.source = None;
    manifest.generator = None;
    let text = serialize_manifest(&manifest).unwrap();
    assert!(!text.contains("\"source\""));
    assert!(!text.contains("\"generator\""));
    assert!(!text.contains("null"));
}

fn mutated(change: impl FnOnce(&mut Value)) -> Result<Manifest, ManifestError> {
    let mut json: Value =
        serde_json::from_str(&serialize_manifest(&sample_manifest()).unwrap()).unwrap();
    change(&mut json);
    read_manifest(&json.to_string())
}

#[test]
fn refuses_what_the_schema_refuses() {
    let invalid =
        |change: fn(&mut Value)| matches!(mutated(change), Err(ManifestError::Invalid(_)));
    assert!(invalid(|m| m["importedAt"] = "2026-10-12".into()));
    assert!(invalid(|m| m["size"]["w"] = 0.into()));
    assert!(invalid(|m| m["size"]["unit"] = "px".into()));
    assert!(invalid(
        |m| m["slides"][0]["transition"] =
            serde_json::json!({ "effect": "fade", "fallback": "dissolve", "durationMs": 1 })
    ));
    assert!(invalid(
        |m| m["slides"][1]["transition"]["durationMs"] = 1.5.into()
    ));
    assert!(invalid(
        |m| m["slides"][1]["transition"]["durationMs"] = 3_600_001.into()
    ));
    assert!(invalid(
        |m| m["slides"][1]["transition"]["fallback"] = "zoom".into()
    ));
    assert!(invalid(|m| m["slides"][0]["failed"] = "not a code".into()));
    assert!(invalid(|m| m["slides"][0]["id"] = "-x".into()));
    assert!(invalid(
        |m| m["themes"]["ir-t1"]["colors"]["dk1"] = "red".into()
    ));
    assert!(invalid(|m| {
        m["themes"]["ir-t1"]["colors"]
            .as_object_mut()
            .unwrap()
            .remove("accent6");
    }));
    assert!(invalid(|m| m["title"] = "x".repeat(257).into()));
    // Accepted forms.
    assert!(mutated(|m| m["slides"][1]["transition"]["durationMs"] = 500.0.into()).is_ok());
    assert!(
        mutated(
            |m| m["themes"]["ir-t1"]["colors"]["dk1"] = "color(display-p3 0.5 1 0 / 0.25)".into()
        )
        .is_ok()
    );
    assert!(mutated(|m| m["slides"][0]["failed"] = "IR0908".into()).is_ok());
    assert!(
        mutated(|m| m["slides"][1]["transition"]["advance"] =
            serde_json::json!({ "onClick": false, "afterMs": 0 }))
        .is_ok()
    );
}

#[test]
fn prints_numbers_as_json_stringify_does() {
    for (w, printed) in [
        (1e-7, "1e-7"),
        (1.5e-7, "1.5e-7"),
        (5e-324, "5e-324"),
        (0.000_001, "0.000001"),
    ] {
        let mut manifest = sample_manifest();
        manifest.size.w = w;
        let text = serialize_manifest(&manifest).unwrap();
        assert!(text.contains(&format!("\"w\": {printed}\n")), "{text}");
        assert_eq!(read_manifest(&text), Ok(manifest));
    }
}

#[test]
fn refuses_integers_beyond_the_safe_range_as_not_integers() {
    let message = |change: fn(&mut Value)| match mutated(change) {
        Err(ManifestError::Invalid(message)) => message,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        message(|m| m["source"]["bytes"] = 9_007_199_254_740_992_u64.into()),
        "source.bytes: Invalid input: expected int, received number"
    );
    assert_eq!(
        message(
            |m| m["slides"][1]["transition"]["durationMs"] = (-9_007_199_254_740_992_i64).into()
        ),
        "slides.1.transition.durationMs: Invalid input: expected int, received number"
    );
    assert!(mutated(|m| m["source"]["bytes"] = 9_007_199_254_740_991_u64.into()).is_ok());
}

#[test]
fn accepts_effect_ids_of_up_to_120_characters_after_the_namespace() {
    let with_effect =
        |effect: String| mutated(move |m| m["slides"][1]["transition"]["effect"] = effect.into());
    assert!(with_effect(format!("pptx:{}", "a".repeat(120))).is_ok());
    assert!(with_effect("x:!~".into()).is_ok());
    for refused in [
        format!("pptx:{}", "a".repeat(121)),
        "pptx:".into(),
        ":a".into(),
        "Pptx:a".into(),
        "pptx:a b".into(),
        "pptx".into(),
    ] {
        assert!(
            matches!(with_effect(refused.clone()), Err(ManifestError::Invalid(_))),
            "{refused}"
        );
    }
}

#[test]
fn refuses_files_that_do_not_follow_from_the_ids_and_themes_it_does_not_hold() {
    let mut misnamed = sample_manifest();
    misnamed.slides[1].file = "slides/ir-S2.html".into();
    assert_eq!(
        serialize_manifest(&misnamed),
        Err(ManifestError::Invalid(
            "slide ir-S2 must be in slides/ir-s2-da7f247d.html".into()
        ))
    );
    let mut themeless = sample_manifest();
    themeless.layouts[0].theme = Some("ir-t9".into());
    assert!(matches!(
        serialize_manifest(&themeless),
        Err(ManifestError::Invalid(_))
    ));
}

#[test]
fn reads_without_panicking_and_names_the_path() {
    assert!(matches!(read_manifest("{"), Err(ManifestError::NotJson(_))));
    assert!(matches!(
        read_manifest("[]"),
        Err(ManifestError::Invalid(_))
    ));
    match mutated(|m| m["title"] = 3.into()) {
        Err(ManifestError::Invalid(message)) => assert!(message.contains("title"), "{message}"),
        other => panic!("{other:?}"),
    }
    match mutated(|m| m["slides"][1]["hidden"] = "no".into()) {
        Err(ManifestError::Invalid(message)) => {
            assert!(message.starts_with("slides.1.hidden: "), "{message}")
        }
        other => panic!("{other:?}"),
    }
}
