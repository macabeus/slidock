//! An Open Packaging Conventions package: parts in a zip, each with its relationships. Targets resolve inside the
//! package (a relationship whose `..` climbs past the root is dropped), external targets are kept as link data and
//! never fetched, and every XML part is parsed under the shared hostile-input limits, its nodes drawn from one budget
//! for the whole package.

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

use indexmap::IndexMap;
use slidock_core_api::FrontendError;
use slidock_core_api::lowering::LowerError;
use slidock_io::{RangeReader, XmlBudget, XmlLimits, ZipArchive, ZipEntry, ZipLimits};
use slidock_ir::diagnostics::{DiagnosticSite, Stage, codes, make_diagnostic};

use crate::EMITTER;
use crate::xml::{Element, Ns, tree};

/// A part the lowering needs that is missing or cannot be read; it fails the container that needs it.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{part}: {reason}")]
pub(crate) struct PartError {
    pub part: String,
    pub reason: String,
}

impl PartError {
    pub fn new(part: impl Into<String>, reason: impl Into<String>) -> Self {
        PartError {
            part: part.into(),
            reason: reason.into(),
        }
    }
}

impl From<PartError> for LowerError {
    fn from(error: PartError) -> Self {
        LowerError::Part {
            part: error.part,
            reason: error.reason,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Relationship {
    pub id: String,
    /// The last segment of the relationship type URI: `slideLayout`, `image`, `hyperlink`, …
    pub kind: String,
    /// A part path inside the package for internal targets; the URI as written for external ones.
    pub target: String,
    pub external: bool,
}

/// A part's relationships by id, in written order.
pub(crate) type Rels = IndexMap<String, Relationship>;

/// The whole input refused: not a zip, or a zip without a presentation.
pub(crate) fn refusal(name: &str, reason: &str) -> FrontendError {
    FrontendError::refused(make_diagnostic(
        codes::PX2001,
        Stage::Frontend,
        EMITTER,
        DiagnosticSite::new()
            .data("name", name)
            .data("reason", reason),
    ))
}

fn dirname(part: &str) -> &str {
    part.rfind('/')
        .map_or("", |at| part.get(..at).unwrap_or(""))
}

fn basename(part: &str) -> &str {
    part.rfind('/')
        .map_or(part, |at| part.get(at + 1..).unwrap_or(""))
}

/// The relationships part of a part: `ppt/slides/_rels/slide1.xml.rels`; the package's own is `_rels/.rels`.
pub(crate) fn rels_part(part: &str) -> String {
    let dir = dirname(part);
    let dir = if dir.is_empty() {
        String::new()
    } else {
        format!("{dir}/")
    };
    format!("{dir}_rels/{}.rels", basename(part))
}

/// `decodeURIComponent`: every `%XX` escape decoded, the result UTF-8; `None` for a malformed escape or sequence.
fn decode_uri_component(text: &str) -> Option<Cow<'_, str>> {
    if !text.contains('%') {
        return Some(Cow::Borrowed(text));
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while let Some(&byte) = bytes.get(at) {
        if byte == b'%' {
            let hex = bytes
                .get(at + 1..at + 3)
                .filter(|hex| hex.iter().all(u8::is_ascii_hexdigit))?;
            let hex = std::str::from_utf8(hex).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            at += 3;
        } else {
            out.push(byte);
            at += 1;
        }
    }
    String::from_utf8(out).ok().map(Cow::Owned)
}

/// A relative or absolute target against its source part, or `None` when it leaves the package.
pub(crate) fn resolve_target(source: &str, target: &str) -> Option<String> {
    let absolute = target.starts_with('/');
    let mut segments: Vec<Cow<'_, str>> = if absolute {
        Vec::new()
    } else {
        dirname(source)
            .split('/')
            .filter(|segment| !segment.is_empty())
            .map(Cow::Borrowed)
            .collect()
    };
    let path = target.split(['?', '#']).next().unwrap_or("");
    for raw in path.split('/') {
        let segment = decode_uri_component(raw).unwrap_or(Cow::Borrowed(raw));
        match &*segment {
            "" | "." => {}
            ".." => {
                segments.pop()?;
            }
            _ => segments.push(segment),
        }
    }
    (!segments.is_empty()).then(|| segments.join("/"))
}

/// A zip of parts, read under its limits.
pub(crate) struct Package<'r> {
    archive: ZipArchive<&'r dyn RangeReader>,
    xml_limits: XmlLimits,
    /// The XML node budget shared by every part this package parses.
    budget: Cell<XmlBudget>,
    rels: RefCell<BTreeMap<String, Result<Rc<Rels>, PartError>>>,
}

impl<'r> Package<'r> {
    /// Opens the zip; an unreadable zip refuses the whole input.
    pub fn open(
        name: &str,
        reader: &'r dyn RangeReader,
        zip: ZipLimits,
        xml: XmlLimits,
    ) -> Result<Self, FrontendError> {
        let archive =
            ZipArchive::open(reader, zip).map_err(|error| refusal(name, &error.message))?;
        Ok(Package {
            archive,
            xml_limits: xml,
            budget: Cell::new(XmlBudget::new(&xml)),
            rels: RefCell::new(BTreeMap::new()),
        })
    }

    pub fn archive(&self) -> &ZipArchive<&'r dyn RangeReader> {
        &self.archive
    }

    pub fn entry(&self, part: &str) -> Option<&ZipEntry> {
        self.archive.get(part)
    }

    pub fn has(&self, part: &str) -> bool {
        self.archive.get(part).is_some()
    }

    /// A part's bytes in memory; a [`PartError`] when it is missing or over a limit.
    pub fn bytes(&self, part: &str) -> Result<Cow<'_, [u8]>, PartError> {
        let entry = self
            .archive
            .get(part)
            .ok_or_else(|| PartError::new(part, "missing"))?;
        self.archive
            .read(entry)
            .map_err(|error| PartError::new(part, error.message))
    }

    /// A part's root element; a [`PartError`] when it is missing, unreadable or malformed.
    pub fn xml(&self, part: &str) -> Result<Rc<Element>, PartError> {
        let bytes = self.bytes(part)?;
        let malformed = |error: slidock_io::XmlError| {
            PartError::new(
                part,
                format!("{}: {}", error.reason.as_str(), error.message),
            )
        };
        let text = slidock_io::decode_xml(&bytes).map_err(malformed)?;
        let mut budget = self.budget.get();
        let parsed = slidock_io::parse_xml(&text, &self.xml_limits, &mut budget);
        self.budget.set(budget);
        Ok(tree(&parsed.map_err(malformed)?))
    }

    /// A part's relationships by id; empty when the part has no relationships part.
    pub fn rels(&self, part: &str) -> Result<Rc<Rels>, PartError> {
        if let Some(cached) = self.rels.borrow().get(part) {
            return cached.clone();
        }
        let read = self.read_rels(part).map(Rc::new);
        self.rels.borrow_mut().insert(part.to_owned(), read.clone());
        read
    }

    fn read_rels(&self, part: &str) -> Result<Rels, PartError> {
        let mut out = Rels::new();
        let source = rels_part(part);
        if !self.has(&source) {
            return Ok(out);
        }
        let root = self.xml(&source)?;
        if !root.is(Ns::Rel, "Relationships") {
            return Err(PartError::new(source, "not a relationships part"));
        }
        for element in root.children(Ns::Rel, "Relationship") {
            let (Some(id), Some(kind), Some(target)) = (
                element.attr("Id").filter(|id| !id.is_empty()),
                element.attr("Type").filter(|kind| !kind.is_empty()),
                element.attr("Target"),
            ) else {
                continue;
            };
            if out.contains_key(id) {
                continue;
            }
            let external = element.attr("TargetMode") == Some("External");
            let resolved = if external {
                Some(target.to_owned())
            } else {
                resolve_target(part, target)
            };
            let Some(target) = resolved else { continue };
            let kind = kind.rsplit('/').next().unwrap_or(kind);
            out.insert(
                id.to_owned(),
                Relationship {
                    id: id.to_owned(),
                    kind: kind.to_owned(),
                    target,
                    external,
                },
            );
        }
        Ok(out)
    }

    /// A relationship by id, internal or external, when its type is `kind` (any type when `None`).
    pub fn target(
        &self,
        part: &str,
        id: Option<&str>,
        kind: Option<&str>,
    ) -> Result<Option<Relationship>, PartError> {
        let Some(id) = id else { return Ok(None) };
        let rels = self.rels(part)?;
        Ok(rels
            .get(id)
            .filter(|rel| kind.is_none_or(|kind| rel.kind == kind))
            .cloned())
    }

    /// The first internal relationship of `kind`.
    pub fn first(&self, part: &str, kind: &str) -> Result<Option<Relationship>, PartError> {
        let rels = self.rels(part)?;
        Ok(rels
            .values()
            .find(|rel| rel.kind == kind && !rel.external)
            .cloned())
    }
}

/// A read made while planning what to lower: an unreadable part yields `None` and fails its container later.
pub(crate) fn unless_unreadable<T>(read: Result<Option<T>, PartError>) -> Option<T> {
    read.ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relationship_targets_inside_the_package_only() {
        let resolve = |target| resolve_target("ppt/slides/slide1.xml", target);
        assert_eq!(
            resolve("../media/image%201.png").as_deref(),
            Some("ppt/media/image 1.png")
        );
        assert_eq!(
            resolve("/ppt/media/a.png").as_deref(),
            Some("ppt/media/a.png")
        );
        assert_eq!(resolve("../../../etc/passwd"), None);
        assert_eq!(
            resolve("./a.xml?x=1#y").as_deref(),
            Some("ppt/slides/a.xml")
        );
        assert_eq!(
            resolve("%E0%A4%A.png").as_deref(),
            Some("ppt/slides/%E0%A4%A.png")
        );
        assert_eq!(
            resolve("%ED%A0%80.png").as_deref(),
            Some("ppt/slides/%ED%A0%80.png")
        );
        assert_eq!(resolve("a%2Fb").as_deref(), Some("ppt/slides/a/b"));
        assert_eq!(resolve_target("x", "/"), None);
        assert_eq!(
            rels_part("ppt/slides/slide1.xml"),
            "ppt/slides/_rels/slide1.xml.rels"
        );
        assert_eq!(rels_part(""), "_rels/.rels");
    }
}
