//! An OpenDocument package: a zip whose first entry, `mimetype`, is stored uncompressed and names the document type,
//! with `content.xml` (the pages), `styles.xml` (common styles and master pages), `meta.xml` and the files they
//! reference. Every XML part is parsed under the shared hostile-input limits; references resolve inside the package,
//! and one with a scheme or leaving the root is external and never fetched.

use std::cell::{OnceCell, RefCell};
use std::collections::BTreeMap;

use slidock_canon::utf16;
use slidock_core_api::FrontendError;
use slidock_core_api::frontend::App;
use slidock_io::zip::STORED;
use slidock_io::{RangeReader, XmlBudget, XmlLimits, ZipArchive, ZipEntry, ZipLimits, parse_xml};
use slidock_ir::diagnostics::{DiagnosticCode, DiagnosticSite, Stage, codes, make_diagnostic};

use crate::names::qn;
use crate::odf::{Element, Ns};

pub(crate) const MIMETYPE: &str = "application/vnd.oasis.opendocument.presentation";

const MIMETYPE_ENTRY: &str = "mimetype";
const LOCAL_HEADER_SIZE: usize = 30;

/// A part a container needs that is missing or cannot be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PartError {
    pub part: String,
    pub reason: String,
}

impl PartError {
    pub(crate) fn new(part: &str, reason: impl Into<String>) -> Self {
        PartError {
            part: part.to_owned(),
            reason: reason.into(),
        }
    }

    /// `part: reason`, as the error reads.
    pub(crate) fn message(&self) -> String {
        format!("{}: {}", self.part, self.reason)
    }
}

/// The whole input refused: not a zip (OD9001), or a zip that is not an OpenDocument presentation (OD9002).
pub(crate) fn refusal(name: &str, code: DiagnosticCode, reason: &str) -> FrontendError {
    FrontendError::refused(make_diagnostic(
        code,
        Stage::Frontend,
        "odp",
        DiagnosticSite::new()
            .data("name", name)
            .data("reason", reason),
    ))
}

/// Whether the first bytes are an OpenDocument presentation's: a local header for a stored `mimetype` entry whose
/// bytes are the presentation media type. ODF requires exactly this layout so the type can be read from the head.
pub(crate) fn is_odp_head(head: &[u8]) -> bool {
    let le16 = |at: usize| {
        head.get(at..at + 2)
            .and_then(|b| b.try_into().ok())
            .map(|b: [u8; 2]| usize::from(u16::from_le_bytes(b)))
    };
    // `subarray(start, start + len)`: clipped to the bytes there are.
    let sub = |start: usize, len: usize| {
        let start = start.min(head.len());
        head.get(start..(start + len).min(head.len()))
            .unwrap_or_default()
    };
    if head.len() < LOCAL_HEADER_SIZE || head.get(..4) != Some(&[0x50, 0x4b, 0x03, 0x04][..]) {
        return false;
    }
    if le16(8) != Some(usize::from(STORED)) {
        return false;
    }
    let (Some(name_length), Some(extra_length)) = (le16(26), le16(28)) else {
        return false;
    };
    let name = sub(LOCAL_HEADER_SIZE, name_length);
    let data = sub(
        LOCAL_HEADER_SIZE + name_length + extra_length,
        MIMETYPE.len(),
    );
    decode_utf8(name) == MIMETYPE_ENTRY && decode_utf8(data) == MIMETYPE
}

/// `new TextDecoder().decode(bytes)`: UTF-8, a leading byte order mark dropped, invalid sequences replaced.
fn decode_utf8(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    text.strip_prefix('\u{feff}').unwrap_or(&text).to_owned()
}

/// A reference (`xlink:href`) as a path inside the package, or why it is not one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Reference {
    Internal(String),
    External(String),
}

/// `^[A-Za-z][A-Za-z0-9+.-]*:`.
fn has_scheme(href: &str) -> bool {
    let bytes = href.as_bytes();
    if !bytes.first().is_some_and(u8::is_ascii_alphabetic) {
        return false;
    }
    let rest = bytes.get(1..).unwrap_or_default();
    let scheme = rest
        .iter()
        .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'.' | b'-'))
        .count();
    rest.get(scheme) == Some(&b':')
}

/// `decodeURIComponent`, or `None` for a malformed escape.
pub(crate) fn decode_uri_component(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while let Some(&b) = bytes.get(at) {
        if b != b'%' {
            out.push(b);
            at += 1;
            continue;
        }
        let hex = bytes.get(at + 1..at + 3)?;
        let text = std::str::from_utf8(hex).ok()?;
        if !text.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        out.push(u8::from_str_radix(text, 16).ok()?);
        at += 3;
    }
    // A decoded byte sequence that is not UTF-8 is a URIError.
    String::from_utf8(out).ok()
}

/// Resolves an `xlink:href` from the package root; a scheme, a leading `/` or a `..` past the root is external.
pub(crate) fn resolve_reference(href: &str) -> Reference {
    let external = || Reference::External(utf16::clip(href, 256).to_owned());
    if has_scheme(href) || href.starts_with('/') {
        return external();
    }
    let path = href.split(['?', '#']).next().unwrap_or_default();
    let mut segments: Vec<String> = Vec::new();
    for raw in path.split('/') {
        let segment = decode_uri_component(raw).unwrap_or_else(|| raw.to_owned());
        match segment.as_str() {
            "" | "." => {}
            ".." => {
                if segments.pop().is_none() {
                    return external();
                }
            }
            _ => segments.push(segment),
        }
    }
    if segments.is_empty() {
        Reference::External(href.to_owned())
    } else {
        Reference::Internal(segments.join("/"))
    }
}

/// An opened package: its zip, read under the presentation's limits, and the XML nodes its parts draw from.
pub(crate) struct OdpPackage<'r> {
    pub archive: ZipArchive<&'r dyn RangeReader>,
    xml_limits: XmlLimits,
    xml_budget: RefCell<XmlBudget>,
    /// Media types by path, from `META-INF/manifest.xml`: hints for embedded objects only.
    manifest_types: OnceCell<BTreeMap<String, String>>,
}

impl<'r> OdpPackage<'r> {
    /// Opens the zip and checks its `mimetype`, refusing the whole input on a mismatch.
    pub(crate) fn open(
        name: &str,
        reader: &'r dyn RangeReader,
        zip: ZipLimits,
        xml: XmlLimits,
    ) -> Result<Self, FrontendError> {
        let archive = ZipArchive::open(reader, zip)
            .map_err(|error| refusal(name, codes::OD9001, &error.message))?;
        let first = archive.entries().first().cloned();
        let Some(first) = first.filter(|entry| entry.name == MIMETYPE_ENTRY) else {
            return Err(refusal(
                name,
                codes::OD9002,
                "the first entry is not mimetype",
            ));
        };
        if first.method != STORED {
            return Err(refusal(name, codes::OD9002, "mimetype is compressed"));
        }
        let bytes = archive
            .read(&first)
            .map_err(|error| refusal(name, codes::OD9002, &error.message))?;
        let mimetype = decode_utf8(&bytes);
        if mimetype != MIMETYPE {
            let shown = utf16::clip(&mimetype, 80);
            return Err(refusal(
                name,
                codes::OD9002,
                &format!("mimetype is {shown}"),
            ));
        }
        drop(bytes);
        Ok(OdpPackage {
            archive,
            xml_budget: RefCell::new(XmlBudget::new(&xml)),
            xml_limits: xml,
            manifest_types: OnceCell::new(),
        })
    }

    pub(crate) fn entry(&self, path: &str) -> Option<&ZipEntry> {
        self.archive.get(path)
    }

    /// A part's root element; a [`PartError`] when it is missing, unreadable or malformed.
    pub(crate) fn xml(&self, part: &str) -> Result<Element, PartError> {
        let entry = self
            .archive
            .get(part)
            .ok_or_else(|| PartError::new(part, "missing"))?;
        let bytes = self
            .archive
            .read(entry)
            .map_err(|error| PartError::new(part, error.message))?;
        let xml_error = |error: slidock_io::XmlError| {
            PartError::new(
                part,
                format!("{}: {}", error.reason.as_str(), error.message),
            )
        };
        let text = slidock_io::decode_xml(&bytes).map_err(xml_error)?;
        let mut budget = self.xml_budget.borrow_mut();
        let document = parse_xml(&text, &self.xml_limits, &mut budget).map_err(xml_error)?;
        Ok(Element::from_document(&document))
    }

    /// The media type the manifest declares for a path, when it declares one.
    pub(crate) fn declared_type(&self, path: &str) -> Option<&str> {
        self.manifest_types
            .get_or_init(|| {
                let mut types = BTreeMap::new();
                if let Ok(root) = self.xml("META-INF/manifest.xml") {
                    for entry in root.children_of(Ns::Manifest, "file-entry") {
                        let (Some(full), Some(media_type)) = (
                            entry
                                .attr(qn!("manifest:full-path"))
                                .filter(|s| !s.is_empty()),
                            entry
                                .attr(qn!("manifest:media-type"))
                                .filter(|s| !s.is_empty()),
                        ) else {
                            continue;
                        };
                        let full = full
                            .strip_prefix("./")
                            .or_else(|| full.strip_prefix('/'))
                            .unwrap_or(full);
                        let full = full.strip_suffix('/').unwrap_or(full);
                        types.insert(full.to_owned(), media_type.to_owned());
                    }
                }
                types
            })
            .get(path)
            .map(String::as_str)
    }

    /// `meta:generator` as written, trimmed; `None` when absent or empty.
    pub(crate) fn generator_text(&self) -> Option<String> {
        let root = self.xml("meta.xml").ok()?;
        let generator = root
            .child(Ns::Office, "meta")
            .and_then(|meta| meta.child(Ns::Meta, "generator"))?;
        let text = generator.text_of();
        let text = slidock_canon::js_trim(&text);
        (!text.is_empty()).then(|| text.to_owned())
    }

    /// The app that wrote the document (`meta:generator`).
    pub(crate) fn generator(&self) -> Option<App> {
        let text = self.generator_text()?;
        let text = text.as_str();
        // `LibreOffice/7.6.4.1$Linux_X86_64 LibreOffice_project/…`: the app and its version before the platform.
        let product = text.split_once('/').and_then(|(name, rest)| {
            let version_len = rest.find(['$', ' ']).unwrap_or(rest.len());
            let version = rest.get(..version_len)?;
            let name_ok = !name.is_empty() && !name.contains(['$', ' ']);
            (name_ok && !version.is_empty()).then_some((name, version))
        });
        Some(match product {
            Some((name, version)) => App {
                name: utf16::clip(name, 256).to_owned(),
                version: Some(utf16::clip(version, 64).to_owned()),
            },
            None => App {
                name: utf16::clip(text, 256).to_owned(),
                version: None,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_references_inside_the_package_only() {
        let internal = |path: &str| Reference::Internal(path.to_owned());
        assert_eq!(
            resolve_reference("Pictures/a%20b.png"),
            internal("Pictures/a b.png")
        );
        assert_eq!(resolve_reference("./Object 1"), internal("Object 1"));
        assert_eq!(resolve_reference("a/../b.png?x#y"), internal("b.png"));
        assert_eq!(resolve_reference("bad%zz.png"), internal("bad%zz.png"));
        assert!(matches!(
            resolve_reference("../outside.png"),
            Reference::External(_)
        ));
        assert_eq!(
            resolve_reference("https://example.com/a.png"),
            Reference::External("https://example.com/a.png".to_owned())
        );
        assert!(matches!(
            resolve_reference("file:///etc/passwd"),
            Reference::External(_)
        ));
        assert!(matches!(
            resolve_reference("/etc/passwd"),
            Reference::External(_)
        ));
        assert_eq!(resolve_reference(""), Reference::External(String::new()));
    }

    #[test]
    fn recognises_schemes() {
        assert!(has_scheme("https:x"));
        assert!(has_scheme("a+b.c-d:x"));
        assert!(!has_scheme("1ab:x"));
        assert!(!has_scheme("Pictures/a:b"));
    }
}
