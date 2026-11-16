//! The format's lint over a deck page's templates: a post-condition for tests and the bench. Each template stands
//! for the slide document it was composed from, with that document's head, its media referenced from the document
//! set's `../media/`, and no layout (its layout's elements are part of it now).

use std::collections::BTreeMap;

use slidock_format::dom::{Document, NodeId};
use slidock_format::markers::attr;
use slidock_format::paths::MANIFEST_PATH;
use slidock_format::{PresentationFiles, Problem, lint_presentation, read_manifest};
use slidock_html::Documents;

use crate::{BundleError, SlideTemplate};

fn compose_error(message: impl Into<String>) -> BundleError {
    BundleError::Compose(message.into())
}

/// A media reference the deck page writes (`media/…`, `url("media/…")`) as the document set writes it.
fn document_reference(name: &str, value: &str) -> String {
    if name == "style" {
        value.replace("url(\"media/", "url(\"../media/")
    } else if value.starts_with("media/") {
        format!("../{value}")
    } else {
        value.to_owned()
    }
}

/// The slide document `template` stands for: `slide_html` with its `<main>` replaced by the template's.
fn template_document(slide_html: &str, template: &SlideTemplate) -> Result<String, BundleError> {
    let mut document = Document::parse(slide_html);
    let body = document
        .body()
        .ok_or_else(|| compose_error(format!("{}: no body", template.id)))?;
    let fragment = Document::parse(&template.html);
    let main = fragment
        .body()
        .and_then(|body| fragment.child_elements(body).into_iter().next())
        .filter(|main| fragment.tag_name(*main) == "main")
        .ok_or_else(|| compose_error(format!("template {} holds no <main>", template.id)))?;
    let copy = document
        .import(&fragment, main)
        .ok_or_else(|| compose_error(format!("template {}", template.id)))?;
    if let Some(attrs) = document.attrs_mut(copy) {
        attrs.retain(|a| a.local() != attr::LAYOUT && a.local() != attr::SHOW_LAYOUT);
    }
    for element in std::iter::once(copy).chain(document.walk(copy)) {
        if let Some(attrs) = document.attrs_mut(element) {
            for attribute in attrs.iter_mut() {
                attribute.value = document_reference(attribute.local(), &attribute.value);
            }
        }
    }
    let children: Vec<NodeId> = document
        .children(body)
        .iter()
        .map(|child| {
            if document.tag_name(*child) == "main" {
                copy
            } else {
                *child
            }
        })
        .collect();
    document.set_children(body, children);
    Ok(document.serialize(NodeId::DOCUMENT))
}

/// The lint's problems with `templates`, the deck page's templates of `documents`; see the module comment.
pub fn lint_templates(
    documents: &Documents,
    templates: &[SlideTemplate],
) -> Result<Vec<Problem>, BundleError> {
    let text = |path: &str| {
        documents
            .text
            .get(path)
            .ok_or_else(|| compose_error(format!("{path} is not in the document set")))
    };
    let mut manifest =
        read_manifest(text(MANIFEST_PATH)?).map_err(|error| compose_error(error.to_string()))?;
    manifest.layouts.clear();
    let mut files: PresentationFiles = BTreeMap::new();
    for slide in &mut manifest.slides {
        slide.layout = None;
        let template = templates
            .iter()
            .find(|template| template.id == slide.id)
            .ok_or_else(|| compose_error(format!("no template for slide {}", slide.id)))?;
        let html = template_document(text(&slide.file)?, template)?;
        files.insert(slide.file.clone(), Some(html));
    }
    let manifest = slidock_format::serialize_manifest(&manifest)
        .map_err(|error| compose_error(error.to_string()))?;
    files.insert(MANIFEST_PATH.to_owned(), Some(manifest));
    for path in documents.media.keys() {
        files.insert(path.clone(), None);
    }
    Ok(lint_presentation(&files))
}
