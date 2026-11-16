//! Composition: one slide as a viewer shows it, from its document and the one layout it names. The bundle composes
//! every slide of the deck page with it; it reads the format alone.

use slidock_canon::js_trim;
use thiserror::Error;

use crate::dom::{Document, NodeId};
use crate::escape::quoted_css;
use crate::lint::PresentationFiles;
use crate::manifest::{ManifestError, read_manifest};
use crate::markers::{self, attr};
use crate::paths::{MANIFEST_PATH, MEDIA_DIR, parse_media_reference};
use crate::references::attribute_references;

/// Where the composed document finds media.
pub enum MediaUrls<'a> {
    /// As the documents reference it: `../media/<sha256>.<ext>`.
    Unchanged,
    /// Under a folder, for a document written outside the file set (`media` for the deck page, which sits beside
    /// its `media/` folder). Written as a URL path.
    Base(&'a str),
    /// The URL of each media file, by its path in the file set (`media/<sha256>.<ext>`), for media that has no
    /// common folder, such as blob URLs.
    Url(&'a dyn Fn(&str) -> String),
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ComposeError {
    #[error("{0} is not in the file set")]
    NotInFileSet(String),
    #[error("{MANIFEST_PATH}: {0}")]
    Manifest(String),
    #[error("{0} is not a slide of this presentation")]
    NotASlide(String),
    #[error("{path} has no <main {attribute}>")]
    NoRoot {
        path: String,
        attribute: &'static str,
    },
    #[error("{file} names the layout {layout}, which the manifest does not list")]
    UnknownLayout { file: String, layout: String },
}

/// A slide composed with its layout.
#[derive(Clone, Debug)]
pub struct ComposedSlide {
    document: Document,
    main: NodeId,
    /// Whether the composed `<main>`'s first element is the background.
    background: bool,
    /// How many of its elements after the background are the layout's.
    layout_elements: usize,
}

impl ComposedSlide {
    /// The whole document: the slide's head as written (title, reset rule), and the composed `<main>`.
    pub fn document_html(&self) -> String {
        self.document.serialize(NodeId::DOCUMENT)
    }

    /// The composed `<main>` alone, for the deck page's slide template.
    pub fn main_html(&self) -> String {
        self.document.serialize_outer(self.main)
    }

    /// The composed tree.
    pub fn document(&self) -> &Document {
        &self.document
    }

    /// The composed `<main>`.
    pub fn main(&self) -> NodeId {
        self.main
    }

    /// The composed `<main>`'s elements in reading order: the slide's own (in their reading order), then the
    /// layout's; the background left out.
    pub fn reading_order(&self) -> Vec<NodeId> {
        let elements = self.document.child_elements(self.main);
        let start = usize::from(self.background);
        let own = start.saturating_add(self.layout_elements);
        let layout = elements.get(start..own).unwrap_or_default();
        let slide = elements.get(own..).unwrap_or_default();
        slide.iter().chain(layout).copied().collect()
    }
}

/// The slide `slide_id` composed with its layout, as one document: the slide's head, and a `<main>` holding the
/// slide's background (or else the layout's), the layout's elements unless the slide hides them, with slide-number
/// fields set to the slide's 1-based show number, then the slide's elements. Expects a file set that lints clean,
/// and fails on a malformed one.
pub fn compose_slide(
    files: &PresentationFiles,
    slide_id: &str,
    media: &MediaUrls<'_>,
) -> Result<String, ComposeError> {
    compose(files, slide_id, media).map(|composed| composed.document_html())
}

/// [`compose_slide`], keeping the composed tree.
pub fn compose(
    files: &PresentationFiles,
    slide_id: &str,
    media: &MediaUrls<'_>,
) -> Result<ComposedSlide, ComposeError> {
    compose_with_sources(files, slide_id, media, &|_| Vec::new())
}

/// A video's sources by its file's path (`media/<sha256>.<ext>`): each file's path and `type`, in the order the
/// browser tries them; none to leave the video's `src` as it is.
pub type Sources<'a> = dyn Fn(&str) -> Vec<(String, String)> + 'a;

/// [`compose`], each `<video src>` that `sources` gives files for holding a `<source>` of each in its place (before
/// the media URLs are rewritten, so each is rewritten as any other reference).
pub fn compose_with_sources(
    files: &PresentationFiles,
    slide_id: &str,
    media: &MediaUrls<'_>,
    sources: &Sources<'_>,
) -> Result<ComposedSlide, ComposeError> {
    let manifest =
        read_manifest(read_text(files, MANIFEST_PATH)?).map_err(|error| match error {
            ManifestError::NotJson(message) | ManifestError::Invalid(message) => {
                ComposeError::Manifest(message)
            }
        })?;
    let (index, entry) = manifest
        .slides
        .iter()
        .enumerate()
        .find(|(_, slide)| slide.id == slide_id)
        .ok_or_else(|| ComposeError::NotASlide(slide_id.to_owned()))?;

    let mut document = Document::parse(read_text(files, &entry.file)?);
    let main = root_of(&document, attr::SLIDE, &entry.file)?;
    let own = document.child_elements(main);
    let is_background = |document: &Document, element: &NodeId| {
        document.has_attribute(*element, markers::BACKGROUND)
    };
    let mut background = own
        .iter()
        .copied()
        .find(|element| is_background(&document, element));
    let mut behind = Vec::new();

    if let Some(layout_id) = document.attribute(main, attr::LAYOUT).map(str::to_owned) {
        let layout = manifest
            .layouts
            .iter()
            .find(|candidate| candidate.id == layout_id)
            .ok_or_else(|| ComposeError::UnknownLayout {
                file: entry.file.clone(),
                layout: layout_id,
            })?;
        let layout_document = Document::parse(read_text(files, &layout.file)?);
        let layout_main = root_of(&layout_document, attr::LAYOUT_ROOT, &layout.file)?;
        let layout_children = layout_document.child_elements(layout_main);
        if background.is_none() {
            background = layout_children
                .iter()
                .find(|element| is_background(&layout_document, element))
                .and_then(|element| document.import(&layout_document, *element));
        }
        if document.attribute(main, attr::SHOW_LAYOUT) != Some("false") {
            let show_number = (index + 1).to_string();
            for element in layout_children
                .iter()
                .filter(|element| !is_background(&layout_document, element))
            {
                if let Some(copy) = document.import(&layout_document, *element) {
                    number_fields(&mut document, copy, &show_number);
                    behind.push(copy);
                }
            }
        }
    }

    let own: Vec<NodeId> = own
        .into_iter()
        .filter(|element| !is_background(&document, element))
        .collect();
    let background_present = background.is_some();
    let layout_elements = behind.len();
    let mut children = Vec::with_capacity(2 * (own.len() + behind.len()) + 3);
    let newline = document.create_text("\n");
    children.push(newline);
    for element in background.into_iter().chain(behind).chain(own) {
        children.push(element);
        let newline = document.create_text("\n");
        children.push(newline);
    }
    document.set_children(main, children);
    offer_sources(&mut document, sources);
    match media {
        MediaUrls::Unchanged => {}
        MediaUrls::Base(base) => rewrite_media(&mut document, &under_base(base)),
        MediaUrls::Url(url_of) => rewrite_media(&mut document, url_of),
    }
    Ok(ComposedSlide {
        document,
        main,
        background: background_present,
        layout_elements,
    })
}

fn read_text<'f>(files: &'f PresentationFiles, path: &str) -> Result<&'f str, ComposeError> {
    match files.get(path) {
        Some(Some(text)) => Ok(text),
        _ => Err(ComposeError::NotInFileSet(path.to_owned())),
    }
}

/// The first `<main>` with `attribute`, in document order.
fn root_of(
    document: &Document,
    attribute: &'static str,
    path: &str,
) -> Result<NodeId, ComposeError> {
    document
        .walk(NodeId::DOCUMENT)
        .into_iter()
        .find(|element| {
            document.tag_name(*element) == "main" && document.has_attribute(*element, attribute)
        })
        .ok_or_else(|| ComposeError::NoRoot {
            path: path.to_owned(),
            attribute,
        })
}

/// Puts the slide's show number into the slide-number fields of `element` and its descendants.
fn number_fields(document: &mut Document, element: NodeId, show_number: &str) {
    let fields: Vec<NodeId> = std::iter::once(element)
        .chain(document.walk(element))
        .filter(|field| document.attribute(*field, attr::FIELD) == Some(attr::SLIDE_NUMBER_FIELD))
        .collect();
    for field in fields {
        let text = document.create_text(show_number);
        document.set_children(field, vec![text]);
    }
}

/// Replaces the `src` of each video `sources` gives files for with a `<source>` of each, before its own children
/// (a `<track>`, which the format never writes).
fn offer_sources(document: &mut Document, sources: &Sources<'_>) {
    for element in document.walk(NodeId::DOCUMENT) {
        if document.tag_name(element) != "video" {
            continue;
        }
        let Some(media) = document
            .attribute(element, "src")
            .and_then(|src| parse_media_reference(js_trim(src)))
        else {
            continue;
        };
        let offered = sources(&media.path);
        if offered.is_empty() {
            continue;
        }
        if let Some(attrs) = document.attrs_mut(element) {
            attrs.retain(|attribute| attribute.local() != "src");
        }
        let mut children: Vec<NodeId> = offered
            .iter()
            .map(|(path, kind)| {
                let src = format!("../{path}");
                document.create_element("source", &[("src", &src), ("type", kind)])
            })
            .collect();
        children.extend_from_slice(document.children(element));
        document.set_children(element, children);
    }
}

/// Rewrites every media reference the lint accepts to `url_of` its path; a CSS one becomes `url("…")`.
fn rewrite_media(document: &mut Document, url_of: &dyn Fn(&str) -> String) {
    for element in document.walk(NodeId::DOCUMENT) {
        let tag = document.tag_name(element).to_owned();
        let Some(attrs) = document.attrs_mut(element) else {
            continue;
        };
        for attribute in attrs.iter_mut() {
            let mut value = attribute.value.clone();
            let mut references = attribute_references(&tag, attribute.local(), &attribute.value);
            references.reverse();
            for reference in references {
                let Some(media) = parse_media_reference(&reference.url) else {
                    continue;
                };
                let url = if reference.css {
                    format!("url({})", css_string(&url_of(&media.path)))
                } else {
                    url_of(&media.path)
                };
                if value.is_char_boundary(reference.start) && value.is_char_boundary(reference.end)
                {
                    value.replace_range(reference.start..reference.end, &url);
                }
            }
            attribute.value = value;
        }
    }
}

/// `text` as a CSS string, its quotes, backslashes and line breaks escaped.
fn css_string(text: &str) -> String {
    quoted_css(text, |c| matches!(c, '"' | '\\' | '\n' | '\r' | '\u{c}'))
}

/// A media base as a URL function: the base (trailing slashes dropped) as a URL path, then the file's name.
fn under_base(media_base: &str) -> impl Fn(&str) -> String {
    let base = url_path(media_base.trim_end_matches('/'));
    move |path: &str| {
        let name = path
            .strip_prefix(MEDIA_DIR)
            .and_then(|rest| rest.strip_prefix('/'))
            .unwrap_or(path);
        format!("{base}/{name}")
    }
}

/// A folder path as a URL path: each segment percent-encoded as `encodeURIComponent` does, `(`, `)`, `'`, `!` and `*`
/// included, so the result stays one URL in an attribute and one token in a CSS `url()`, whatever the folder's name.
fn url_path(path: &str) -> String {
    path.split('/')
        .map(|segment| {
            if segment == "." || segment == ".." {
                return segment.to_owned();
            }
            let mut encoded = String::with_capacity(segment.len());
            for byte in segment.bytes() {
                if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                    encoded.push(char::from(byte));
                } else {
                    encoded.push_str(&format!("%{byte:02X}"));
                }
            }
            encoded
        })
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_url_paths_as_encode_uri_component_and_more() {
        assert_eq!(
            url_path("../my presentation#1 (it's)/media"),
            "../my%20presentation%231%20%28it%27s%29/media"
        );
        assert_eq!(url_path("./a!*~ü"), "./a%21%2A~%C3%BC");
        assert_eq!(under_base("media//")("media/x.png"), "media/x.png");
    }
}
