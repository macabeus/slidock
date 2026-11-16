//! Resolved-only values are pass outputs, recomputed on every run, so a value an agent wrote is dropped here rather
//! than pass for authoritative (IR0012, once per kind per container). Resolved forms that are valid inputs by design
//! stay: `paths` geometry, frame-local gradients, a stored autofit fontScale and lineSpacingReduction, labelled
//! bullets.

use serde_json::{Map, Value};

const LINE_METRICS: [&str; 5] = [
    "lineHeight",
    "leadingSkew",
    "lineTailPt",
    "spaceBeforePt",
    "spaceAfterPt",
];
const ELEMENT_KINDS: [&str; 6] = ["shape", "picture", "media", "table", "group", "foreign"];
/// Raw source values, kept as written.
pub(crate) const OPAQUE: [&str; 2] = ["ext", "payload"];

/// Hears each kind of resolved-only field a strip drops.
pub(crate) type Note<'a> = dyn FnMut(&'static str) + 'a;

fn is_opaque(key: &str) -> bool {
    OPAQUE.contains(&key)
}

fn strip_object(object: &mut Map<String, Value>, note: &mut Note<'_>) {
    for (key, value) in object.iter_mut() {
        if !is_opaque(key) {
            strip(value, note);
        }
    }
    let mut drop = |object: &mut Map<String, Value>, key: &str, field: &'static str| {
        if object.shift_remove(key).is_some() {
            note(field);
        }
    };
    let kind = object
        .get("kind")
        .and_then(Value::as_str)
        .map(str::to_owned);
    if kind.as_deref() == Some("theme") && object.contains_key("mods") {
        drop(object, "resolved", "colour resolved value");
    }
    if kind.as_deref() == Some("shrink") {
        drop(object, "fitted", "autofit fitted flag");
    }
    if object.get("paragraphs").is_some_and(Value::is_array) {
        drop(object, "rotation", "text rotation");
    }
    for key in LINE_METRICS {
        drop(object, key, key);
    }
    if kind
        .as_deref()
        .is_some_and(|kind| ELEMENT_KINDS.contains(&kind))
        && object.contains_key("origin")
    {
        drop(object, "inherits", "inheritance edge");
    }
}

/// `value` minus its resolved-only fields; `note` hears each field kind dropped.
pub(crate) fn strip(value: &mut Value, note: &mut Note<'_>) {
    match value {
        Value::Array(items) => items.iter_mut().for_each(|item| strip(item, note)),
        Value::Object(object) => strip_object(object, note),
        _ => {}
    }
}

/// [`strip`] for a slide, which also drops its a11y outputs.
pub(crate) fn strip_slide(slide: &mut Value, note: &mut Note<'_>) {
    let Value::Object(object) = slide else { return };
    strip_object(object, note);
    if object.shift_remove("title").is_some() {
        note("slide title");
    }
    if object.shift_remove("readingOrder").is_some() {
        note("reading order");
    }
}

/// Which decided field a record's entries lose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Decided {
    /// Font bindings, which bind-fonts decides.
    Binding,
    /// Media plans, which plan-media decides.
    Plan,
}

impl Decided {
    fn key(self) -> &'static str {
        match self {
            Decided::Binding => "binding",
            Decided::Plan => "plan",
        }
    }

    fn field(self) -> &'static str {
        match self {
            Decided::Binding => "font binding",
            Decided::Plan => "media plan",
        }
    }
}

/// Drops the field `decided` names from each entry of a record (`fonts` or `media`).
pub(crate) fn strip_entries(record: &mut Value, decided: Decided, note: &mut Note<'_>) {
    let Value::Object(entries) = record else {
        return;
    };
    for entry in entries.values_mut() {
        if let Value::Object(entry) = entry
            && entry.shift_remove(decided.key()).is_some()
        {
            note(decided.field());
        }
    }
}

/// Where a container holds an origin a pass created, if anywhere: authored IR describes sources only. The path is
/// relative to the container (`elements[2].origin`).
pub(crate) fn synth_origin_path(value: &Value) -> Option<String> {
    find_synth(value, "")
}

fn find_synth(value: &Value, path: &str) -> Option<String> {
    match value {
        Value::Array(items) => items
            .iter()
            .enumerate()
            .find_map(|(index, item)| find_synth(item, &format!("{path}[{index}]"))),
        Value::Object(object) => {
            let synth = object.get("kind").and_then(Value::as_str) == Some("synth")
                && object.get("pass").is_some_and(Value::is_string);
            if synth {
                return Some(if path.is_empty() {
                    "(container)".to_owned()
                } else {
                    path.to_owned()
                });
            }
            object
                .iter()
                .filter(|(key, _)| !is_opaque(key))
                .find_map(|(key, child)| {
                    let at = if path.is_empty() {
                        key.clone()
                    } else {
                        format!("{path}.{key}")
                    };
                    find_synth(child, &at)
                })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn stripped(mut value: Value, slide: bool) -> (Value, Vec<&'static str>) {
        let mut notes = Vec::new();
        let mut note = |field| notes.push(field);
        if slide {
            strip_slide(&mut value, &mut note);
        } else {
            strip(&mut value, &mut note);
        }
        (value, notes)
    }

    #[test]
    fn drops_each_resolved_only_field_and_names_it() {
        let (value, notes) = stripped(
            json!({
                "color": { "kind": "theme", "slot": "accent1", "mods": [], "resolved": { "kind": "rgb" } },
                "autofit": { "kind": "shrink", "fontScale": 0.9, "fitted": true },
                "text": { "paragraphs": [{ "props": { "lineHeight": 1, "spaceBeforePt": 0 } }], "rotation": 90 },
            }),
            false,
        );
        assert_eq!(
            value,
            json!({
                "color": { "kind": "theme", "slot": "accent1", "mods": [] },
                "autofit": { "kind": "shrink", "fontScale": 0.9 },
                "text": { "paragraphs": [{ "props": {} }] },
            })
        );
        assert_eq!(
            notes,
            [
                "colour resolved value",
                "autofit fitted flag",
                "lineHeight",
                "spaceBeforePt",
                "text rotation"
            ]
        );
    }

    #[test]
    fn keeps_a_resolved_value_without_mods_and_an_edge_of_an_element_without_origin() {
        let input = json!({
            "resolved": { "kind": "theme", "resolved": 1 },
            "element": { "kind": "shape", "inherits": "ir-l.title" },
            "rotation": 90,
        });
        let (value, notes) = stripped(input.clone(), false);
        assert_eq!(value, input);
        assert!(notes.is_empty());
        let (value, notes) = stripped(
            json!({ "kind": "picture", "origin": { "kind": "agent" }, "inherits": "ir-l.pic" }),
            false,
        );
        assert_eq!(
            value,
            json!({ "kind": "picture", "origin": { "kind": "agent" } })
        );
        assert_eq!(notes, ["inheritance edge"]);
    }

    #[test]
    fn leaves_raw_source_values_as_written() {
        let input =
            json!({ "ext": { "lineHeight": 1 }, "payload": [{ "spaceAfterPt": 2 }], "a": [] });
        let (value, notes) = stripped(input.clone(), false);
        assert_eq!(value, input);
        assert!(notes.is_empty());
    }

    #[test]
    fn drops_a_slides_a11y_outputs_even_when_null() {
        let (value, notes) = stripped(
            json!({ "id": "ir-s1", "title": null, "readingOrder": [] }),
            true,
        );
        assert_eq!(value, json!({ "id": "ir-s1" }));
        assert_eq!(notes, ["slide title", "reading order"]);
        let (value, notes) = stripped(json!({ "id": "ir-s1", "title": "Kept" }), false);
        assert_eq!(value, json!({ "id": "ir-s1", "title": "Kept" }));
        assert!(notes.is_empty());
    }

    #[test]
    fn drops_bindings_and_plans_of_object_entries_only() {
        let mut fonts = json!({ "Lato": { "request": {}, "binding": {} }, "Odd": 3 });
        let mut notes = Vec::new();
        strip_entries(&mut fonts, Decided::Binding, &mut |field| notes.push(field));
        assert_eq!(fonts, json!({ "Lato": { "request": {} }, "Odd": 3 }));
        let mut media = json!({ "a": { "plan": {} }, "b": { "plan": {} } });
        strip_entries(&mut media, Decided::Plan, &mut |field| notes.push(field));
        assert_eq!(media, json!({ "a": {}, "b": {} }));
        assert_eq!(notes, ["font binding", "media plan", "media plan"]);
    }

    #[test]
    fn finds_the_first_synthesized_origin_outside_raw_values() {
        let container = json!({
            "id": "ir-s1",
            "ext": { "kind": "synth", "pass": "x" },
            "elements": [
                { "origin": { "kind": "agent" } },
                { "children": [{ "origin": { "kind": "synth", "pass": "bind-placeholders" } }] },
            ],
        });
        assert_eq!(
            synth_origin_path(&container).as_deref(),
            Some("elements[1].children[0].origin")
        );
        assert_eq!(
            synth_origin_path(&json!({ "kind": "synth", "pass": "verify" })).as_deref(),
            Some("(container)")
        );
        assert_eq!(
            synth_origin_path(&json!({ "origin": { "kind": "synth", "pass": 3 } })),
            None
        );
    }
}
