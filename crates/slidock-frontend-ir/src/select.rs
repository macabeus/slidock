//! A partial lower: the slides `only` names, with just the layouts, masters, themes and media they use.

use std::collections::BTreeSet;

use serde_json::{Map, Value};

/// Every media hash a value references (pictures, posters, image fills, Foreign fallbacks).
fn referenced_media(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::Array(items) => items.iter().for_each(|item| referenced_media(item, out)),
        Value::Object(object) => {
            for (key, child) in object {
                match child {
                    Value::String(sha) if key == "media" || key == "poster" => {
                        out.insert(sha.clone());
                    }
                    _ if key == "ext" || key == "payload" => {}
                    _ => referenced_media(child, out),
                }
            }
        }
        _ => {}
    }
}

/// The record's entries whose key is in `keys`; a value that is not an object stays as it is, for the verifier to
/// report.
fn pick(record: &Value, keys: &BTreeSet<String>) -> Value {
    match record {
        Value::Object(entries) => Value::Object(
            entries
                .iter()
                .filter(|(key, _)| keys.contains(*key))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// The string each object value of a record holds under `field`.
fn links(record: Option<&Value>, field: &str) -> BTreeSet<String> {
    record
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(Map::values)
        .filter_map(|entry| entry.get(field)?.as_str().map(str::to_owned))
        .collect()
}

/// The slides in `only`, in show order, with just the layouts, masters, themes and media they use. The slides were
/// checked to be an array of objects with string ids.
pub(crate) fn select_slides(json: &Map<String, Value>, only: &[String]) -> Map<String, Value> {
    let slides: Vec<Value> = json
        .get("slides")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|slide| {
            slide
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| only.iter().any(|wanted| wanted == id))
        })
        .cloned()
        .collect();
    let mut out = json.clone();
    let layout_keys: BTreeSet<String> = slides
        .iter()
        .filter_map(|slide| slide.get("layout")?.as_str().map(str::to_owned))
        .collect();
    let replace = |out: &mut Map<String, Value>, key: &str, keys: &BTreeSet<String>| {
        if let Some(record) = json.get(key) {
            out.insert(key.to_owned(), pick(record, keys));
        }
    };
    replace(&mut out, "layouts", &layout_keys);
    let master_keys = links(out.get("layouts"), "master");
    replace(&mut out, "masters", &master_keys);
    let theme_keys = links(out.get("masters"), "theme");
    replace(&mut out, "themes", &theme_keys);
    let mut media = BTreeSet::new();
    for value in [
        Some(&Value::Array(slides.clone())),
        out.get("layouts"),
        out.get("masters"),
    ]
    .into_iter()
    .flatten()
    {
        referenced_media(value, &mut media);
    }
    replace(&mut out, "media", &media);
    out.insert("slides".to_owned(), Value::Array(slides));
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn keeps_the_chain_and_the_media_of_the_chosen_slides() {
        let json = json!({
            "slides": [
                { "id": "ir-s1", "layout": "ir-l1", "elements": [{ "media": "m1", "ext": { "media": "mx" } }] },
                { "id": "ir-s2", "layout": "ir-l2", "background": { "kind": "image", "media": "m2" } },
            ],
            "layouts": { "ir-l1": { "master": "ir-m1", "elements": [{ "poster": "m3" }] }, "ir-l2": { "master": "ir-m2" } },
            "masters": { "ir-m1": { "theme": "ir-t1" }, "ir-m2": { "theme": "ir-t2" } },
            "themes": { "ir-t1": {}, "ir-t2": {} },
            "media": { "m1": {}, "m2": {}, "m3": {}, "mx": {} },
            "size": { "w": 720, "h": 405 },
        });
        let selected = select_slides(json.as_object().unwrap(), &["ir-s1".to_owned()]);
        assert_eq!(
            Value::Object(selected),
            json!({
                "slides": [
                    { "id": "ir-s1", "layout": "ir-l1", "elements": [{ "media": "m1", "ext": { "media": "mx" } }] },
                ],
                "layouts": { "ir-l1": { "master": "ir-m1", "elements": [{ "poster": "m3" }] } },
                "masters": { "ir-m1": { "theme": "ir-t1" } },
                "themes": { "ir-t1": {} },
                "media": { "m1": {}, "m3": {} },
                "size": { "w": 720, "h": 405 },
            })
        );
    }

    #[test]
    fn leaves_absent_and_non_object_records_as_they_are() {
        let json = json!({ "slides": [{ "id": "ir-s1" }], "layouts": [1], "media": "none" });
        let selected = select_slides(json.as_object().unwrap(), &["ir-s1".to_owned()]);
        assert_eq!(Value::Object(selected), json);
    }
}
