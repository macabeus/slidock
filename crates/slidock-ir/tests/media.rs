//! A media element's `acrossSlides`: written only when set, so the IR of every clip without it keeps its bytes, and
//! read only as a boolean.
#![allow(clippy::unwrap_used, reason = "test code")]

use serde_json::{Value, json};
use slidock_ir::{resolved, source};

fn video(extra: Value) -> Value {
    let mut element = json!({
        "id": "kn-1", "origin": { "kind": "agent" },
        "frame": { "x": 0, "y": 0, "w": 160, "h": 90, "rotation": 0, "flipH": false, "flipV": false },
        "mediaKind": "video", "loop": false, "muted": false, "autoplay": false
    });
    if let (Some(element), Some(extra)) = (element.as_object_mut(), extra.as_object()) {
        element.extend(extra.clone());
    }
    element
}

fn round_trip<T: serde::Serialize + serde::de::DeserializeOwned>(value: &Value) -> Value {
    serde_json::to_value(serde_json::from_value::<T>(value.clone()).unwrap()).unwrap()
}

#[test]
fn writes_across_slides_only_when_set_in_both_phases() {
    for (given, expected) in [
        (json!({}), None),
        (json!({ "acrossSlides": false }), None),
        (json!({ "acrossSlides": true }), Some(json!(true))),
    ] {
        let element = video(given.clone());
        for written in [
            round_trip::<source::MediaElement>(&element),
            round_trip::<resolved::MediaElement>(&element),
        ] {
            assert_eq!(written.get("acrossSlides"), expected.as_ref(), "{given}");
        }
    }
    let marked: source::MediaElement =
        serde_json::from_value(video(json!({ "acrossSlides": true }))).unwrap();
    assert!(marked.across_slides);
    let plain: resolved::MediaElement = serde_json::from_value(video(json!({}))).unwrap();
    assert!(!plain.across_slides);
}

#[test]
fn refuses_an_across_slides_that_is_not_a_boolean() {
    for value in [json!("true"), json!(1), json!(null)] {
        let element = video(json!({ "acrossSlides": value }));
        assert!(
            serde_json::from_value::<source::MediaElement>(element.clone()).is_err(),
            "{value}"
        );
        assert!(
            serde_json::from_value::<resolved::MediaElement>(element).is_err(),
            "{value}"
        );
    }
}
