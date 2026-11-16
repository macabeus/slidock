//! The lint: everything that would make a surface show a deck wrongly or unsafely, read the way a browser reads the
//! files. The compiler runs it on its own output as a post-condition, and every problem names the file it is in, so a
//! problem fails only the slide or layout that file holds. It returns every problem rather than stopping at the first.

use std::collections::{BTreeMap, BTreeSet};

use slidock_canon::{js_trim, js_trim_start, utf16};

use crate::dom::{Document, NodeId};
use crate::manifest::{MAX_SLIDES, Manifest, ManifestError, read_manifest};
use crate::markers::attr;
use crate::paths::{MANIFEST_PATH, SLIDES_DIR, is_document_path, parse_media_reference};
use crate::problems::{Problem, ProblemCode};
use crate::references::attribute_references;
use crate::reset::RESET_STYLE;
use crate::style::parse_style;

/// A file set by path: the text of the manifest and the documents, and `None` for a media file, which the lint
/// checks for presence only.
pub type PresentationFiles = BTreeMap<String, Option<String>>;

const LINK_SCHEMES: &[&str] = &["http", "https", "mailto"];

/// The elements a document may hold: the document frame, text, lists, tables and media in HTML, and drawing in SVG.
/// An allowlist rather than a list of unsafe elements, so an element that runs, embeds another document, navigates
/// or submits (`<iframe srcdoc>`, `<object>`, `<embed>`, `<form>`, `<foreignObject>`, SVG animation) never reaches a
/// surface. `<script>`, `<style>` and `<link>` have problems of their own. Case-sensitive: SVG names are camelCase.
const ELEMENTS: &[&str] = &[
    "html",
    "head",
    "meta",
    "title",
    "body",
    "main",
    "template", //
    "div",
    "span",
    "p",
    "br",
    "hr",
    "a",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "section",
    "article",
    "header", //
    "footer",
    "figure",
    "figcaption",
    "blockquote",
    "pre",
    "code",
    "b",
    "strong",
    "i",
    "em",
    "u",
    "s",
    "sub", //
    "sup",
    "small",
    "mark",
    "ol",
    "ul",
    "li",
    "table",
    "caption",
    "colgroup",
    "col",
    "thead",
    "tbody",
    "tfoot", //
    "tr",
    "th",
    "td",
    "img",
    "picture",
    "source",
    "video",
    "audio",
    "track", //
    "svg",
    "g",
    "defs",
    "symbol",
    "use",
    "desc",
    "path",
    "rect",
    "circle",
    "ellipse",
    "line",
    "polyline", //
    "polygon",
    "text",
    "tspan",
    "textPath",
    "image",
    "linearGradient",
    "radialGradient",
    "stop",
    "pattern", //
    "clipPath",
    "mask",
    "marker",
    "filter",
    "feBlend",
    "feColorMatrix",
    "feComponentTransfer",
    "feComposite", //
    "feDropShadow",
    "feFlood",
    "feFuncA",
    "feFuncB",
    "feFuncG",
    "feFuncR",
    "feGaussianBlur",
    "feMerge", //
    "feMergeNode",
    "feOffset",
];

/// `^[a-z][a-z0-9+.-]*:`, case-insensitive: the scheme of an absolute URL, without its colon.
fn url_scheme(url: &str) -> Option<&str> {
    let colon = url.find(':')?;
    let scheme = url.get(..colon)?;
    let mut bytes = scheme.bytes();
    let first = bytes.next()?;
    (first.is_ascii_alphabetic()
        && bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'.' | b'-')))
    .then_some(scheme)
}

/// `^\s*javascript:`, case-insensitive.
fn is_javascript_url(value: &str) -> bool {
    js_trim_start(value)
        .get(..11)
        .is_some_and(|head| head.eq_ignore_ascii_case("javascript:"))
}

/// `^on`, case-insensitive.
fn is_event_handler(name: &str) -> bool {
    name.get(..2)
        .is_some_and(|head| head.eq_ignore_ascii_case("on"))
}

/// The first `max` UTF-16 code units of `text`, as the reference's `slice(0, max)` keeps them.
fn clip(text: &str, max: usize) -> &str {
    utf16::clip(text, max)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DocumentKind {
    Slide,
    Layout,
}

#[derive(Default)]
struct DocumentLint {
    problems: Vec<Problem>,
    /// The `data-el` ids, in document order.
    elements: Vec<String>,
    /// The media paths the document references.
    media: Vec<String>,
    /// The root's `data-layout`, when the root matches the entry: `Some(None)` for a root without one.
    root_layout: Option<Option<String>>,
    /// The root's `data-show-layout`.
    shows_layout: bool,
}

/// The problems of a file set, sorted by file, then code, then message.
pub fn lint_presentation(files: &PresentationFiles) -> Vec<Problem> {
    let mut problems: Vec<Problem> = Vec::new();
    let mut referenced: BTreeSet<String> = BTreeSet::new();
    let mut listed: BTreeSet<String> = BTreeSet::from([MANIFEST_PATH.to_owned()]);

    let manifest = lint_manifest(files, &mut problems);
    let mut lint_file =
        |path: &str, kind: DocumentKind, id: Option<&str>, problems: &mut Vec<Problem>| {
            let Some(Some(text)) = files.get(path) else {
                problems.push(Problem::new(ProblemCode::FileMissing, path, &[]));
                return None;
            };
            let mut linted = lint_document(path, text, files, kind, id);
            problems.append(&mut linted.problems);
            referenced.extend(linted.media.iter().cloned());
            Some(linted)
        };

    match &manifest {
        Some(manifest) => {
            let mut layouts: BTreeMap<&str, Option<DocumentLint>> = BTreeMap::new();
            for layout in &manifest.layouts {
                listed.insert(layout.file.clone());
                let linted = lint_file(
                    &layout.file,
                    DocumentKind::Layout,
                    Some(&layout.id),
                    &mut problems,
                );
                layouts.insert(&layout.id, linted);
            }
            for slide in &manifest.slides {
                listed.insert(slide.file.clone());
                let Some(linted) = lint_file(
                    &slide.file,
                    DocumentKind::Slide,
                    Some(&slide.id),
                    &mut problems,
                ) else {
                    continue;
                };
                let Some(named) = &linted.root_layout else {
                    continue;
                };
                if named != &slide.layout {
                    let detail = format!(
                        "its data-layout ({}) differs from the manifest's ({})",
                        named.as_deref().unwrap_or("none"),
                        slide.layout.as_deref().unwrap_or("none")
                    );
                    problems.push(Problem::new(
                        ProblemCode::DocumentRoot,
                        &slide.file,
                        &[("detail", &detail)],
                    ));
                }
                let Some(named) = named else {
                    continue;
                };
                let layout = manifest.layouts.iter().find(|entry| &entry.id == named);
                let Some(layout) =
                    layout.filter(|layout| matches!(files.get(&layout.file), Some(Some(_))))
                else {
                    problems.push(Problem::new(
                        ProblemCode::LayoutMissing,
                        &slide.file,
                        &[("layout", named)],
                    ));
                    continue;
                };
                // The slide shows its layout's elements behind its own, so their ids must stay distinct.
                if !linted.shows_layout {
                    continue;
                }
                let behind: BTreeSet<&str> = layouts
                    .get(named.as_str())
                    .and_then(Option::as_ref)
                    .map(|layout| layout.elements.iter().map(String::as_str).collect())
                    .unwrap_or_default();
                let mut seen = BTreeSet::new();
                for id in &linted.elements {
                    if seen.insert(id.as_str()) && behind.contains(id.as_str()) {
                        let where_ = format!("composed with {}", layout.file);
                        problems.push(Problem::new(
                            ProblemCode::DuplicateElement,
                            &slide.file,
                            &[("id", id), ("where", &where_)],
                        ));
                    }
                }
            }
        }
        None => {
            // Lacking a manifest, each document is checked on its own.
            for path in sorted_paths(files) {
                if is_document_path(path) {
                    let kind = if path.starts_with(&format!("{SLIDES_DIR}/")) {
                        DocumentKind::Slide
                    } else {
                        DocumentKind::Layout
                    };
                    lint_file(path, kind, None, &mut problems);
                }
            }
        }
    }

    for path in sorted_paths(files) {
        if listed.contains(path) || referenced.contains(path) {
            continue;
        }
        let document = is_document_path(path);
        if manifest.is_some() && document {
            problems.push(Problem::new(ProblemCode::UnlistedDocument, path, &[]));
        } else if manifest.is_some() || !document {
            problems.push(Problem::new(ProblemCode::OrphanFile, path, &[]));
        }
    }

    problems.sort_by(|a, b| {
        utf16::cmp(&a.file, &b.file)
            .then_with(|| utf16::cmp(a.code.name(), b.code.name()))
            .then_with(|| utf16::cmp(&a.message, &b.message))
    });
    problems
}

/// The paths of a file set in UTF-16 order.
fn sorted_paths(files: &PresentationFiles) -> Vec<&str> {
    let mut paths: Vec<&str> = files.keys().map(String::as_str).collect();
    paths.sort_by(|a, b| utf16::cmp(a, b));
    paths
}

fn lint_manifest(files: &PresentationFiles, problems: &mut Vec<Problem>) -> Option<Manifest> {
    let Some(Some(text)) = files.get(MANIFEST_PATH) else {
        problems.push(Problem::new(
            ProblemCode::ManifestMissing,
            MANIFEST_PATH,
            &[],
        ));
        return None;
    };
    let manifest = match read_manifest(text) {
        Ok(manifest) => manifest,
        Err(ManifestError::NotJson(detail) | ManifestError::Invalid(detail)) => {
            problems.push(Problem::new(
                ProblemCode::ManifestInvalid,
                MANIFEST_PATH,
                &[("detail", &detail)],
            ));
            return None;
        }
    };
    let mut seen = BTreeSet::new();
    let ids = manifest
        .slides
        .iter()
        .map(|slide| &slide.id)
        .chain(manifest.layouts.iter().map(|layout| &layout.id));
    for id in ids {
        if !seen.insert(id) {
            problems.push(Problem::new(
                ProblemCode::DuplicateId,
                MANIFEST_PATH,
                &[("id", id)],
            ));
        }
    }
    if manifest.slides.len() > MAX_SLIDES {
        let count = manifest.slides.len().to_string();
        let max = MAX_SLIDES.to_string();
        problems.push(Problem::new(
            ProblemCode::TooManySlides,
            MANIFEST_PATH,
            &[("count", &count), ("max", &max)],
        ));
    }
    Some(manifest)
}

/// A reference a document loads: a fragment of itself, or a media file of the set in the one allowed form.
fn check_reference(path: &str, reference: &str, files: &PresentationFiles, out: &mut DocumentLint) {
    let trimmed = js_trim(reference);
    if trimmed.starts_with('#') {
        return;
    }
    if url_scheme(trimmed).is_some() || trimmed.starts_with("//") {
        out.problems.push(Problem::new(
            ProblemCode::RemoteUrl,
            path,
            &[("reference", clip(trimmed, 120))],
        ));
        return;
    }
    match parse_media_reference(trimmed) {
        None => out.problems.push(Problem::new(
            ProblemCode::MediaReference,
            path,
            &[("reference", clip(trimmed, 120))],
        )),
        Some(media) if !files.contains_key(&media.path) => {
            out.problems.push(Problem::new(
                ProblemCode::MediaMissing,
                path,
                &[("reference", trimmed)],
            ));
        }
        Some(media) => out.media.push(media.path),
    }
}

fn check_style(
    path: &str,
    document: &Document,
    element: NodeId,
    files: &PresentationFiles,
    out: &mut DocumentLint,
) {
    let tag = document.tag_name(element);
    let mut declarations: BTreeMap<String, String> = BTreeMap::new();
    if let Some(style) = document.attribute(element, "style") {
        let parsed = match parse_style(style) {
            Ok(parsed) => parsed,
            Err(error) => {
                out.problems.push(Problem::new(
                    ProblemCode::StyleInvalid,
                    path,
                    &[("detail", &error.reason)],
                ));
                return;
            }
        };
        for reference in attribute_references(tag, "style", style) {
            check_reference(path, &reference.url, files, out);
        }
        for declaration in parsed {
            let keyword = declaration
                .keyword()
                .unwrap_or_else(|| js_trim(&declaration.value).to_lowercase());
            if declaration.property == "position" && keyword == "fixed" {
                out.problems
                    .push(Problem::new(ProblemCode::FixedPosition, path, &[]));
            }
            if declaration.uses_viewport_units() {
                out.problems
                    .push(Problem::new(ProblemCode::ViewportUnits, path, &[]));
            }
            declarations.insert(declaration.property, keyword);
        }
    }
    let Some(id) = document.attribute(element, attr::ELEMENT) else {
        return;
    };
    out.elements.push(id.to_owned());
    let mut missing: Vec<&str> = Vec::new();
    if declarations.get("position").map(String::as_str) != Some("absolute") {
        missing.push("position:absolute");
    }
    missing.extend(
        ["left", "top", "width"]
            .into_iter()
            .filter(|property| !declarations.contains_key(*property)),
    );
    if !missing.is_empty() {
        let missing = missing.join(", ");
        out.problems.push(Problem::new(
            ProblemCode::ElementUnpositioned,
            path,
            &[("id", id), ("missing", &missing)],
        ));
    }
}

fn check_attributes(
    path: &str,
    document: &Document,
    element: NodeId,
    files: &PresentationFiles,
    out: &mut DocumentLint,
) {
    let tag = document.tag_name(element);
    for attribute in document.attrs(element) {
        let (name, value) = (attribute.local(), attribute.value.as_str());
        if is_event_handler(name) {
            out.problems.push(Problem::new(
                ProblemCode::EventHandler,
                path,
                &[("attribute", name)],
            ));
        }
        if is_javascript_url(value) {
            out.problems
                .push(Problem::new(ProblemCode::JavascriptUrl, path, &[]));
            continue;
        }
        if name == "href" && tag == "a" {
            // A link goes to a fragment of its document or to an absolute URL of a link scheme.
            let trimmed = js_trim(value);
            let scheme = url_scheme(trimmed).map(str::to_ascii_lowercase);
            let allowed = scheme.is_some_and(|scheme| LINK_SCHEMES.contains(&scheme.as_str()));
            if !trimmed.starts_with('#') && !allowed {
                out.problems.push(Problem::new(
                    ProblemCode::LinkScheme,
                    path,
                    &[("url", clip(trimmed, 120))],
                ));
            }
        } else if name != "style" {
            // The style's URLs are checked with its declarations.
            for reference in attribute_references(tag, name, value) {
                check_reference(path, &reference.url, files, out);
            }
        }
    }
}

/// One slide or layout document on its own.
fn lint_document(
    path: &str,
    html: &str,
    files: &PresentationFiles,
    kind: DocumentKind,
    id: Option<&str>,
) -> DocumentLint {
    let mut out = DocumentLint::default();
    let document = Document::parse(html);
    let resets: Vec<NodeId> = document
        .head()
        .map(|head| document.walk(head))
        .unwrap_or_default()
        .into_iter()
        .filter(|element| {
            document.tag_name(*element) == "style" && document.text_content(*element) == RESET_STYLE
        })
        .collect();
    if resets.len() != 1 {
        let detail = if resets.is_empty() {
            "the reset <style> is missing or changed"
        } else {
            "the reset <style> appears twice"
        };
        out.problems.push(Problem::new(
            ProblemCode::ResetStyle,
            path,
            &[("detail", detail)],
        ));
    }
    let mut seen = BTreeSet::new();
    let elements = document.walk(NodeId::DOCUMENT);
    for element in &elements {
        let element = *element;
        let tag = document.tag_name(element);
        if tag == "script" {
            out.problems
                .push(Problem::new(ProblemCode::Script, path, &[]));
        } else if tag == "style" || tag == "link" {
            if !resets.contains(&element) {
                out.problems.push(Problem::new(
                    ProblemCode::ExtraStyle,
                    path,
                    &[("element", tag)],
                ));
            }
        } else if !ELEMENTS.contains(&tag) {
            out.problems.push(Problem::new(
                ProblemCode::ElementNotAllowed,
                path,
                &[("element", tag)],
            ));
        } else if tag == "meta" && document.has_attribute(element, "http-equiv") {
            // A refresh navigates and a content security policy changes what the surface may load.
            out.problems.push(Problem::new(
                ProblemCode::ElementNotAllowed,
                path,
                &[("element", "meta http-equiv")],
            ));
        }
        if tag == "video" && !document.has_attribute(element, "poster") {
            out.problems
                .push(Problem::new(ProblemCode::VideoPoster, path, &[]));
        }
        check_attributes(path, &document, element, files, &mut out);
        check_style(path, &document, element, files, &mut out);
        if let Some(element_id) = document.attribute(element, attr::ELEMENT)
            && !seen.insert(element_id)
        {
            out.problems.push(Problem::new(
                ProblemCode::DuplicateElement,
                path,
                &[("id", element_id), ("where", "itself")],
            ));
        }
    }

    if let Some(id) = id {
        let root_attribute = match kind {
            DocumentKind::Slide => attr::SLIDE,
            DocumentKind::Layout => attr::LAYOUT_ROOT,
        };
        let root = elements
            .iter()
            .copied()
            .find(|element| document.tag_name(*element) == "main");
        match root.filter(|root| document.attribute(*root, root_attribute) == Some(id)) {
            Some(root) => {
                out.root_layout = Some(document.attribute(root, attr::LAYOUT).map(str::to_owned));
                out.shows_layout = document.attribute(root, attr::SHOW_LAYOUT) != Some("false");
            }
            None => {
                let detail = format!("its <main> is not {root_attribute}=\"{id}\"");
                out.problems.push(Problem::new(
                    ProblemCode::DocumentRoot,
                    path,
                    &[("detail", &detail)],
                ));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_url_schemes_javascript_urls_and_handlers() {
        assert_eq!(url_scheme("HTTPS://x"), Some("HTTPS"));
        assert_eq!(url_scheme("a+b.c-d:x"), Some("a+b.c-d"));
        assert_eq!(url_scheme("1a:x"), None);
        assert_eq!(url_scheme("../a:b"), None);
        assert_eq!(url_scheme("#a:b"), None);
        assert!(is_javascript_url(" \u{a0}JavaScript:alert(1)"));
        assert!(!is_javascript_url("java script:"));
        assert!(is_event_handler("onClick") && is_event_handler("ON") && !is_event_handler("o"));
    }
}
