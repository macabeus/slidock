//! The URLs an attribute loads, read in one place for the lint, which checks each, and the composer, which rewrites the
//! media ones, so a reference the lint accepts is always one the composer finds.

use slidock_canon::{is_js_whitespace, js_trim, js_trim_start};

use crate::style::css_urls;

/// A URL an attribute loads, the byte span it takes in the value, and whether that span is a whole CSS `url()`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttributeReference {
    pub url: String,
    pub start: usize,
    pub end: usize,
    pub css: bool,
}

/// Attributes whose value the browser loads. `href` loads everywhere but on links.
const LOADING_ATTRIBUTES: &[&str] = &["src", "poster", "data", "background", "href"];

/// Attributes that load through a CSS `url()`: `style`, and SVG presentation attributes.
const CSS_URL_ATTRIBUTES: &[&str] = &[
    "style",
    "fill",
    "stroke",
    "clip-path",
    "mask",
    "filter",
    "marker-start",
    "marker-mid",
    "marker-end",
    "cursor",
];

/// Every URL the attribute `name` of a `tag` element loads, in order. A link's `href` navigates instead.
pub fn attribute_references(tag: &str, name: &str, value: &str) -> Vec<AttributeReference> {
    if name == "href" && tag == "a" {
        return Vec::new();
    }
    if LOADING_ATTRIBUTES.contains(&name) {
        let start = value.len() - js_trim_start(value).len();
        let url = js_trim(value);
        return vec![AttributeReference {
            url: url.to_owned(),
            start,
            end: start + url.len(),
            css: false,
        }];
    }
    if CSS_URL_ATTRIBUTES.contains(&name) {
        return css_urls(value)
            .into_iter()
            .map(|found| AttributeReference {
                url: found.url,
                start: found.start,
                end: found.end,
                css: true,
            })
            .collect();
    }
    match name {
        // Each comma-separated candidate starts with its URL.
        "srcset" => {
            let mut references = Vec::new();
            let mut offset = 0;
            for candidate in value.split(',') {
                let trimmed = js_trim_start(candidate);
                let start = offset + candidate.len() - trimmed.len();
                let url = trimmed.split(is_js_whitespace).next().unwrap_or("");
                if !url.is_empty() {
                    references.push(AttributeReference {
                        url: url.to_owned(),
                        start,
                        end: start + url.len(),
                        css: false,
                    });
                }
                offset += candidate.len() + 1;
            }
            references
        }
        // A link's ping list: URLs the browser posts to when the link is followed.
        "ping" => {
            let mut references = Vec::new();
            let mut start = None;
            for (index, c) in value.char_indices().chain([(value.len(), ' ')]) {
                match (is_js_whitespace(c), start) {
                    (false, None) => start = Some(index),
                    (true, Some(from)) => {
                        references.push(AttributeReference {
                            url: value.get(from..index).unwrap_or("").to_owned(),
                            start: from,
                            end: index,
                            css: false,
                        });
                        start = None;
                    }
                    _ => {}
                }
            }
            references
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn urls(tag: &str, name: &str, value: &str) -> Vec<(String, usize, usize, bool)> {
        attribute_references(tag, name, value)
            .into_iter()
            .map(|r| (r.url, r.start, r.end, r.css))
            .collect()
    }

    #[test]
    fn reads_each_kind_of_attribute() {
        assert_eq!(
            urls("img", "src", "  a.png "),
            [("a.png".into(), 2, 7, false)]
        );
        assert_eq!(urls("a", "href", "x"), []);
        assert_eq!(urls("use", "href", "#g"), [("#g".into(), 0, 2, false)]);
        assert_eq!(
            urls("rect", "fill", "url(#g) red"),
            [("#g".into(), 0, 7, true)]
        );
        assert_eq!(
            urls("img", "srcset", "a.png 1x, b.png 2x,,"),
            [
                ("a.png".into(), 0, 5, false),
                ("b.png".into(), 10, 15, false)
            ]
        );
        assert_eq!(
            urls("a", "ping", " p q\tr "),
            [
                ("p".into(), 1, 2, false),
                ("q".into(), 3, 4, false),
                ("r".into(), 5, 6, false)
            ]
        );
        assert_eq!(urls("div", "title", "url(x)"), []);
    }

    #[test]
    fn reads_every_loading_attribute_on_any_element() {
        for (tag, name) in [
            ("img", "src"),
            ("video", "poster"),
            ("object", "data"),
            ("td", "background"),
            ("body", "background"),
            ("use", "href"),
            ("image", "href"),
        ] {
            assert_eq!(
                urls(tag, name, " x "),
                [("x".into(), 1, 2, false)],
                "{tag} {name}"
            );
        }
        assert_eq!(urls("td", "data-x", "x"), []);
    }
}
