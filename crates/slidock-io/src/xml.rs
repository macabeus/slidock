//! XML parts (OOXML, ODF) parsed by `roxmltree`. Hostile input is bounded here, once, for every frontend: no DOCTYPE
//! (so no entity expansion beyond the five predefined entities and numeric references), a depth cap, a node cap per
//! part and across a presentation's parts, and caps on attribute count and size.
//!
//! Nodes are counted as a streaming parser counts them: every element, and every run of text between elements, a
//! comment or processing instruction inside the run not breaking it. Namespace declarations count as attributes.
//! Namespace URIs are trimmed of JavaScript whitespace, as saxes trims them.
//!
//! Depth, attribute counts and character references are checked by a scan of the tags before roxmltree parses, so a
//! hostile part costs it no more than linear time. Parts a streaming parser would read and this module refuses: a part
//! whose namespaces in scope cost roxmltree more than 32 lookups per byte, or that binds more than 65,536 distinct
//! namespaces, is refused as `too-many-namespaces`; XML 1.1 parts are read as XML 1.0. Within one start tag, a limit
//! the tag breaks is reported before a malformation inside it.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::ops::Range;

use indexmap::IndexMap;
use roxmltree::{Document, Node, ParsingOptions};
use slidock_canon::{js_trim, utf16};
use thiserror::Error;

use crate::text::decode_utf8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct XmlLimits {
    pub depth: usize,
    /// One part's nodes.
    pub nodes: u64,
    /// Every part of one presentation, together: a 97-slide presentation holds 50,000.
    pub presentation_nodes: u64,
    /// One element's attributes, namespace declarations included.
    pub attributes: usize,
    /// One attribute value's length, in UTF-16 code units.
    pub attribute_bytes: usize,
}

impl Default for XmlLimits {
    fn default() -> Self {
        Self {
            depth: 256,
            nodes: 1_000_000,
            presentation_nodes: 4_000_000,
            attributes: 512,
            attribute_bytes: 1024 * 1024,
        }
    }
}

/// The nodes a presentation's parts may still parse into, shared by every part it parses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct XmlBudget {
    pub nodes: i64,
}

impl XmlBudget {
    pub fn new(limits: &XmlLimits) -> Self {
        Self {
            nodes: i64::try_from(limits.presentation_nodes).unwrap_or(i64::MAX),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum XmlErrorReason {
    Malformed,
    Doctype,
    Encoding,
    TooDeep,
    TooManyNodes,
    TooManyAttributes,
    AttributeTooLarge,
    TooManyNamespaces,
}

impl XmlErrorReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            XmlErrorReason::Malformed => "malformed",
            XmlErrorReason::Doctype => "doctype",
            XmlErrorReason::Encoding => "encoding",
            XmlErrorReason::TooDeep => "too-deep",
            XmlErrorReason::TooManyNodes => "too-many-nodes",
            XmlErrorReason::TooManyAttributes => "too-many-attributes",
            XmlErrorReason::AttributeTooLarge => "attribute-too-large",
            XmlErrorReason::TooManyNamespaces => "too-many-namespaces",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct XmlError {
    pub reason: XmlErrorReason,
    pub message: String,
}

impl XmlError {
    fn new(reason: XmlErrorReason, message: impl Into<String>) -> Self {
        Self {
            reason,
            message: message.into(),
        }
    }
}

/// A part's bytes as text: UTF-16LE after its byte order mark, otherwise UTF-8 (a byte order mark dropped, invalid
/// sequences replaced). UTF-16BE is refused.
pub fn decode_xml(bytes: &[u8]) -> Result<Cow<'_, str>, XmlError> {
    match bytes {
        [0xff, 0xfe, rest @ ..] => {
            let (pairs, odd) = rest.as_chunks::<2>();
            let units = pairs.iter().map(|&pair| u16::from_le_bytes(pair));
            let mut text: String = char::decode_utf16(units)
                .map(|unit| unit.unwrap_or(char::REPLACEMENT_CHARACTER))
                .collect();
            if !odd.is_empty() {
                text.push(char::REPLACEMENT_CHARACTER);
            }
            Ok(Cow::Owned(text))
        }
        [0xfe, 0xff, ..] => Err(XmlError::new(
            XmlErrorReason::Encoding,
            "UTF-16BE parts are not read",
        )),
        _ => Ok(decode_utf8(bytes)),
    }
}

/// The nodes roxmltree may hold past twice a part's limit.
const FLOOD_SLACK: u64 = 1024;

/// The namespace lookups a part may cost roxmltree per byte, past [`NAMESPACE_WORK_FLOOR`]. Real parts cost a few.
const NAMESPACE_WORK_PER_BYTE: u64 = 32;
const NAMESPACE_WORK_FLOOR: u64 = 1 << 20;

/// Parses one part, its nodes drawn from `budget`; an [`XmlError`] for malformed or over-limit input. The budget is
/// charged for the nodes counted before a limit is hit, as a streaming parser would charge them.
pub fn parse_xml<'input>(
    text: &'input str,
    limits: &XmlLimits,
    budget: &mut XmlBudget,
) -> Result<Document<'input>, XmlError> {
    let mut counter = Counter {
        limits,
        budget,
        nodes: 0,
    };
    match scan(text, limits) {
        Scan::Clean => {}
        Scan::Refused(refusal) => return Err(refuse_at(text, refusal, &mut counter)),
        // roxmltree refuses what the scan cannot read.
        Scan::Stopped => {
            return Err(match parse_tree(text, limits) {
                Err(error) => roxmltree_error(error, &mut counter),
                Ok(_) => XmlError::new(XmlErrorReason::Malformed, "The part could not be scanned"),
            });
        }
    }
    let document =
        parse_tree(text, limits).map_err(|error| roxmltree_error(error, &mut counter))?;
    check_limits(&document, &mut counter)?;
    Ok(document)
}

fn parse_tree<'input>(
    text: &'input str,
    limits: &XmlLimits,
) -> Result<Document<'input>, roxmltree::Error> {
    // roxmltree also holds comments, processing instructions and the document node, which are not counted; it stops
    // at twice the part's limit (plus room for the uncounted ones of a small part), to bound its memory.
    let cap = limits.nodes.saturating_mul(2).saturating_add(FLOOD_SLACK);
    let options = ParsingOptions {
        allow_dtd: false,
        nodes_limit: u32::try_from(cap).unwrap_or(u32::MAX),
        ..ParsingOptions::default()
    };
    Document::parse_with_options(text, options)
}

/// The error for what roxmltree refused.
fn roxmltree_error(error: roxmltree::Error, counter: &mut Counter<'_>) -> XmlError {
    match error {
        roxmltree::Error::NodesLimitReached => {
            // Charged as a part one node past its limit, which a flood of elements and text would be.
            for _ in 0..=counter.limits.nodes {
                if let Err(error) = counter.count() {
                    return error;
                }
            }
            counter.part_error()
        }
        roxmltree::Error::NamespacesLimitReached => XmlError::new(
            XmlErrorReason::TooManyNamespaces,
            "More than 65536 distinct namespaces",
        ),
        roxmltree::Error::DtdDetected => {
            XmlError::new(XmlErrorReason::Doctype, "A DOCTYPE is not accepted")
        }
        roxmltree::Error::NoRootNode => {
            XmlError::new(XmlErrorReason::Malformed, "The part has no root element")
        }
        error => XmlError::new(XmlErrorReason::Malformed, error.to_string()),
    }
}

/// The error for a refusal the scan found, unless the part breaks a rule before it: the markup before the refused
/// tag or text is parsed and checked, its open elements closed.
fn refuse_at(text: &str, refused: Refusal<'_>, counter: &mut Counter<'_>) -> XmlError {
    let closers: usize = refused.open.iter().map(|name| name.len() + 3).sum();
    let mut before = String::with_capacity(refused.at + closers);
    before.push_str(text.get(..refused.at).unwrap_or_default());
    for name in refused.open.iter().rev() {
        before.push_str("</");
        before.push_str(name);
        before.push('>');
    }
    match parse_tree(&before, counter.limits) {
        Ok(document) => {
            if let Err(error) = check_limits(&document, counter) {
                return error;
            }
        }
        // The refused tag is the root.
        Err(roxmltree::Error::NoRootNode) if refused.open.is_empty() => {}
        Err(error) => return roxmltree_error(error, counter),
    }
    if refused.counted
        && let Err(error) = counter.count()
    {
        return error;
    }
    refused.error
}

struct Counter<'a> {
    limits: &'a XmlLimits,
    budget: &'a mut XmlBudget,
    nodes: u64,
}

impl Counter<'_> {
    fn part_error(&self) -> XmlError {
        XmlError::new(
            XmlErrorReason::TooManyNodes,
            format!("More than {} XML nodes", self.limits.nodes),
        )
    }

    fn count(&mut self) -> Result<(), XmlError> {
        self.nodes += 1;
        if self.nodes > self.limits.nodes {
            return Err(self.part_error());
        }
        self.budget.nodes -= 1;
        if self.budget.nodes < 0 {
            return Err(XmlError::new(
                XmlErrorReason::TooManyNodes,
                format!(
                    "The presentation holds more than {} XML nodes",
                    self.limits.presentation_nodes
                ),
            ));
        }
        Ok(())
    }
}

/// What a scan of the markup found.
enum Scan<'input> {
    /// Every tag within the limits.
    Clean,
    /// Markup that is not well-formed, which roxmltree refuses.
    Stopped,
    Refused(Refusal<'input>),
}

/// A start tag or text the scan refuses.
struct Refusal<'input> {
    /// Where its markup starts.
    at: usize,
    /// The qualified names of the elements open there, outermost first.
    open: Vec<&'input str>,
    /// Whether a streaming parser counts it as a node before refusing it: a start tag over a limit, not a malformed one.
    counted: bool,
    error: XmlError,
}

/// Checks depth, attribute counts, namespace scope and character references tag by tag, before roxmltree builds the
/// tree: its namespace resolution costs the namespaces in scope at each element, and its duplicate-attribute check
/// the square of an element's attributes. Comments, CDATA sections and processing instructions are skipped.
fn scan<'input>(text: &'input str, limits: &XmlLimits) -> Scan<'input> {
    let length = u64::try_from(text.len()).unwrap_or(u64::MAX);
    let mut scanner = Scanner {
        text,
        limits,
        open: Vec::new(),
        declared: Vec::new(),
        scope: BTreeMap::new(),
        work: 0,
        work_limit: length
            .saturating_mul(NAMESPACE_WORK_PER_BYTE)
            .saturating_add(NAMESPACE_WORK_FLOOR),
        seen_root: false,
    };
    scanner.run()
}

struct Scanner<'input, 'l> {
    text: &'input str,
    limits: &'l XmlLimits,
    /// Each open element's qualified name and the number of prefixes it declares.
    open: Vec<(&'input str, usize)>,
    /// The prefixes the open elements declare, in order; `""` for the default namespace.
    declared: Vec<&'input str>,
    /// The bindings in scope for each prefix.
    scope: BTreeMap<&'input str, usize>,
    /// roxmltree's namespace lookups so far: each element's names searched through the namespaces in its scope, and
    /// for an element declaring any, its parent's copied with a search each.
    work: u64,
    work_limit: u64,
    seen_root: bool,
}

/// What reading one piece of markup gave.
enum Step<'input> {
    Next(usize),
    Stopped,
    Refused(Refusal<'input>),
}

impl<'input> Scanner<'input, '_> {
    fn run(&mut self) -> Scan<'input> {
        let text = self.text;
        let mut at = 0;
        loop {
            let Some(markup) = text.get(at..).and_then(|rest| rest.find('<')) else {
                return Scan::Clean;
            };
            let markup = at + markup;
            if !self.open.is_empty()
                && let Some(error) = text.get(at..markup).and_then(reference_error)
            {
                return Scan::Refused(self.refusal(at, false, error));
            }
            let rest = text.get(markup..).unwrap_or_default();
            let step = if rest.starts_with("<!--") {
                skip_past(text, markup + 4, "-->")
            } else if rest.starts_with("<![CDATA[") {
                skip_past(text, markup + 9, "]]>")
            } else if rest.starts_with("<?") {
                skip_past(text, markup + 2, "?>")
            } else if rest.starts_with("<!") {
                Step::Stopped
            } else if rest.starts_with("</") {
                self.end_tag(markup)
            } else {
                self.start_tag(markup)
            };
            match step {
                Step::Next(next) => at = next,
                Step::Stopped => return Scan::Stopped,
                Step::Refused(refusal) => return Scan::Refused(refusal),
            }
        }
    }

    fn refusal(&self, at: usize, counted: bool, error: XmlError) -> Refusal<'input> {
        Refusal {
            at,
            open: self.open.iter().map(|(name, _)| *name).collect(),
            counted,
            error,
        }
    }

    fn end_tag(&mut self, markup: usize) -> Step<'input> {
        let Some(end) = self.text.get(markup..).and_then(|rest| rest.find('>')) else {
            return Step::Stopped;
        };
        let Some((_, declared)) = self.open.pop() else {
            return Step::Stopped;
        };
        self.unbind(declared);
        Step::Next(markup + end + 1)
    }

    fn start_tag(&mut self, markup: usize) -> Step<'input> {
        let text = self.text;
        let bytes = text.as_bytes();
        let byte = |at: usize| bytes.get(at).copied();
        if self.open.is_empty() && self.seen_root {
            return Step::Stopped;
        }
        let mut at = markup + 1;
        while byte(at).is_some_and(|b| !is_space(b) && b != b'/' && b != b'>') {
            at += 1;
        }
        let Some(name) = text.get(markup + 1..at) else {
            return Step::Stopped;
        };
        let mut attributes = 0;
        let mut declared = 0;
        let mut prefixed = 0;
        let mut reference = None;
        let empty = loop {
            while byte(at).is_some_and(is_space) {
                at += 1;
            }
            match byte(at) {
                Some(b'>') => break false,
                Some(b'/') if byte(at + 1) == Some(b'>') => {
                    at += 1;
                    break true;
                }
                Some(b'/') | None => return Step::Stopped,
                Some(_) => {}
            }
            let qname_start = at;
            while byte(at).is_some_and(|b| b != b'=' && b != b'/' && b != b'>' && !is_space(b)) {
                at += 1;
            }
            let Some(qname) = text.get(qname_start..at) else {
                return Step::Stopped;
            };
            while byte(at).is_some_and(is_space) {
                at += 1;
            }
            if byte(at) != Some(b'=') {
                return Step::Stopped;
            }
            at += 1;
            while byte(at).is_some_and(is_space) {
                at += 1;
            }
            let Some(quote @ (b'"' | b'\'')) = byte(at) else {
                return Step::Stopped;
            };
            let value_start = at + 1;
            let Some(value) = text
                .get(value_start..)
                .and_then(|rest| rest.find(char::from(quote)))
                .and_then(|length| text.get(value_start..value_start + length))
            else {
                return Step::Stopped;
            };
            at = value_start + value.len() + 1;
            attributes += 1;
            if qname == "xmlns" {
                self.declared.push("");
                declared += 1;
            } else if let Some(prefix) = qname.strip_prefix("xmlns:") {
                self.declared.push(prefix);
                declared += 1;
            } else if qname.contains(':') {
                prefixed += 1;
            }
            if reference.is_none() {
                reference = reference_error(value);
            }
        };
        let next = at + 1;
        self.seen_root = true;

        // A streaming parser reads the whole tag before it counts the element and checks its limits.
        if let Some(error) = reference {
            return Step::Refused(self.refusal(markup, false, error));
        }
        let limits = self.limits;
        if self.open.len() >= limits.depth {
            return Step::Refused(self.refusal(
                markup,
                true,
                XmlError::new(
                    XmlErrorReason::TooDeep,
                    format!("XML nested deeper than {}", limits.depth),
                ),
            ));
        }
        if attributes > limits.attributes {
            return Step::Refused(self.refusal(
                markup,
                true,
                XmlError::new(
                    XmlErrorReason::TooManyAttributes,
                    format!(
                        "An element holds more than {} attributes",
                        limits.attributes
                    ),
                ),
            ));
        }
        let inherited = self.scope.len() as u64;
        let first = self.declared.len() - declared;
        for prefix in self.declared.get(first..).unwrap_or_default() {
            *self.scope.entry(prefix).or_default() += 1;
        }
        let in_scope = self.scope.len() as u64;
        let declares = declared as u64;
        let copies = if declared > 0 {
            inherited.saturating_mul(declares.saturating_add(inherited))
        } else {
            0
        };
        let lookups = (1 + prefixed + declares).saturating_mul(in_scope);
        self.work = self.work.saturating_add(copies).saturating_add(lookups);
        if self.work > self.work_limit {
            self.unbind(declared);
            return Step::Refused(self.refusal(
                markup,
                true,
                XmlError::new(
                    XmlErrorReason::TooManyNamespaces,
                    format!(
                        "The namespaces in scope cost more than {} lookups",
                        self.work_limit
                    ),
                ),
            ));
        }
        if empty {
            self.unbind(declared);
        } else {
            self.open.push((name, declared));
        }
        Step::Next(next)
    }

    /// Ends the last `declared` bindings.
    fn unbind(&mut self, declared: usize) {
        for _ in 0..declared {
            let Some(prefix) = self.declared.pop() else {
                return;
            };
            if let Some(bindings) = self.scope.get_mut(prefix) {
                *bindings -= 1;
                if *bindings == 0 {
                    self.scope.remove(prefix);
                }
            }
        }
    }
}

/// The offset past the next `end` from `from`.
fn skip_past<'input>(text: &str, from: usize, end: &str) -> Step<'input> {
    match text.get(from..).and_then(|rest| rest.find(end)) {
        Some(found) => Step::Next(from + found + end.len()),
        None => Step::Stopped,
    }
}

/// An error for the first character reference in `raw` that names no Unicode character (a surrogate, or past
/// U+10FFFF), which roxmltree would read as U+FFFD.
fn reference_error(raw: &str) -> Option<XmlError> {
    let mut rest = raw;
    while let Some(found) = rest.find("&#") {
        rest = rest.get(found + 2..)?;
        let (digits, radix) = match rest.strip_prefix('x') {
            Some(hex) => (hex, 16),
            None => (rest, 10),
        };
        let length = digits
            .find(|c: char| !c.is_digit(radix))
            .unwrap_or(digits.len());
        if length == 0 || !digits.get(length..)?.starts_with(';') {
            continue;
        }
        let value = digits.get(..length)?.chars().fold(0u64, |value, c| {
            value
                .saturating_mul(u64::from(radix))
                .saturating_add(u64::from(c.to_digit(radix).unwrap_or(0)))
        });
        if u32::try_from(value).ok().and_then(char::from_u32).is_none() {
            return Some(XmlError::new(
                XmlErrorReason::Malformed,
                format!(
                    "&#{}; names no character",
                    rest.get(..length + usize::from(radix == 16))?
                ),
            ));
        }
    }
    None
}

/// Every node count and attribute size, and the namespace rules roxmltree leaves out, checked in document order as a
/// streaming parser meets them.
fn check_limits(document: &Document<'_>, counter: &mut Counter<'_>) -> Result<(), XmlError> {
    for node in document.root().descendants() {
        if node.is_element() {
            let (name, attributes) = start_tag(node);
            check_names(name, &attributes)?;
            counter.count()?;
            check_attribute_sizes(&attributes, counter.limits)?;
        } else if node.is_text()
            && node.parent().is_some_and(|parent| parent.is_element())
            && !continues_text(node)
        {
            counter.count()?;
        }
    }
    Ok(())
}

/// Whether a text node continues the text before it, past comments and processing instructions.
fn continues_text(node: Node<'_, '_>) -> bool {
    node.prev_siblings()
        .skip(1)
        .find(|sibling| !sibling.is_comment() && !sibling.is_pi())
        .is_some_and(|sibling| sibling.is_text())
}

fn malformed(message: impl Into<String>) -> XmlError {
    XmlError::new(XmlErrorReason::Malformed, message)
}

/// The rules of XML namespaces that roxmltree does not check: names with an empty prefix or local part, declarations
/// of `xmlns`, of an empty prefixed namespace or of a reserved namespace once trimmed, and attributes that one name
/// once their namespaces are trimmed.
fn check_names(name: &str, attributes: &[WrittenAttribute<'_, '_>]) -> Result<(), XmlError> {
    let names = std::iter::once(name).chain(attributes.iter().map(|attribute| attribute.qname));
    for qname in names {
        if let Some((prefix, local)) = qname.split_once(':')
            && (prefix.is_empty() || local.is_empty() || local.contains(':'))
        {
            return Err(malformed(format!("Malformed name {qname}")));
        }
    }
    for attribute in attributes.iter().filter(|attribute| attribute.uri == XMLNS) {
        let prefix = attribute.qname.strip_prefix("xmlns:").unwrap_or("");
        let uri = js_trim(attribute.value);
        if prefix == "xmlns" || uri == XMLNS {
            return Err(malformed("The xmlns namespace is never declared"));
        }
        if uri == XML && prefix != "xml" {
            return Err(malformed("Only the xml prefix binds the xml namespace"));
        }
        if !prefix.is_empty() && uri.is_empty() {
            return Err(malformed(format!("Prefix {prefix} is declared empty")));
        }
    }
    if attributes.iter().any(|attribute| attribute.trimmed) {
        let namespaced = attributes
            .iter()
            .filter(|attribute| attribute.uri != XMLNS && attribute.qname.contains(':'));
        for (index, attribute) in namespaced.clone().enumerate() {
            if namespaced
                .clone()
                .skip(index + 1)
                .any(|other| (other.uri, other.local) == (attribute.uri, attribute.local))
            {
                return Err(malformed(format!(
                    "Duplicate attribute {{{}}}{}",
                    attribute.uri, attribute.local
                )));
            }
        }
    }
    Ok(())
}

fn check_attribute_sizes(
    attributes: &[WrittenAttribute<'_, '_>],
    limits: &XmlLimits,
) -> Result<(), XmlError> {
    for attribute in attributes {
        if utf16::len(attribute.value) > limits.attribute_bytes {
            return Err(XmlError::new(
                XmlErrorReason::AttributeTooLarge,
                format!(
                    "Attribute {} exceeds {} bytes",
                    attribute.qname, limits.attribute_bytes
                ),
            ));
        }
    }
    Ok(())
}

const XMLNS: &str = "http://www.w3.org/2000/xmlns/";
const XML: &str = "http://www.w3.org/XML/1998/namespace";

/// One attribute of a start tag as written, namespace declarations included.
struct WrittenAttribute<'a, 'input> {
    qname: &'input str,
    /// The value with references expanded and whitespace normalised.
    value: &'a str,
    /// The namespace the attribute's prefix binds, trimmed; [`XMLNS`] for a declaration.
    uri: &'a str,
    /// Whether trimming changed `uri`.
    trimmed: bool,
    local: &'input str,
}

fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n')
}

/// The byte ranges of an element's start tag: its qualified name, and each attribute's qualified name. The document
/// parsed, so the tag is well-formed.
fn scan_start_tag(text: &str, start: usize) -> (Range<usize>, Vec<Range<usize>>) {
    let bytes = text.as_bytes();
    let byte = |at: usize| bytes.get(at).copied();
    let name_start = start + 1;
    let mut at = name_start;
    while byte(at).is_some_and(|b| !is_space(b) && b != b'/' && b != b'>') {
        at += 1;
    }
    let name = name_start..at;
    let mut attributes = Vec::new();
    loop {
        while byte(at).is_some_and(is_space) {
            at += 1;
        }
        match byte(at) {
            None | Some(b'/' | b'>') => break,
            Some(_) => {}
        }
        let qname_start = at;
        while byte(at).is_some_and(|b| b != b'=' && !is_space(b)) {
            at += 1;
        }
        attributes.push(qname_start..at);
        while byte(at).is_some_and(|b| b == b'=' || is_space(b)) {
            at += 1;
        }
        let Some(quote) = byte(at) else { break };
        at += 1;
        while byte(at).is_some_and(|b| b != quote) {
            at += 1;
        }
        at += 1;
    }
    (name, attributes)
}

/// An element's qualified name and its attributes in written order, namespace declarations included.
fn start_tag<'a, 'input>(
    node: Node<'a, 'input>,
) -> (&'input str, Vec<WrittenAttribute<'a, 'input>>) {
    let text = node.document().input_text();
    let (name, qnames) = scan_start_tag(text, node.range().start);
    // roxmltree keeps the other attributes in written order.
    let mut parsed = node.attributes();
    let attributes = qnames
        .into_iter()
        .filter_map(|range| {
            let qname = text.get(range.clone())?;
            if qname == "xmlns" || qname.starts_with("xmlns:") {
                let prefix = qname.strip_prefix("xmlns:");
                let uri = match prefix {
                    // roxmltree binds `xml` without storing its declaration.
                    Some("xml") => Some(XML),
                    Some(prefix) => node.lookup_namespace_uri(Some(prefix)),
                    None => node.default_namespace(),
                };
                return Some(WrittenAttribute {
                    qname,
                    value: uri.unwrap_or(""),
                    uri: XMLNS,
                    trimmed: false,
                    local: prefix.unwrap_or(qname),
                });
            }
            let attribute =
                parsed.find(|attribute| attribute.range_qname().start == range.start)?;
            let raw = attribute.namespace().unwrap_or("");
            let uri = js_trim(raw);
            Some(WrittenAttribute {
                qname,
                value: attribute.value(),
                uri,
                trimmed: uri.len() != raw.len(),
                local: attribute.name(),
            })
        })
        .collect();
    (text.get(name).unwrap_or(""), attributes)
}

/// An element's qualified name as written: `p:sp`.
pub fn qualified_name<'input>(node: Node<'_, 'input>) -> &'input str {
    let text = node.document().input_text();
    let (name, _) = scan_start_tag(text, node.range().start);
    text.get(name).unwrap_or("")
}

/// An element's namespace URI, trimmed; `""` for no namespace.
pub fn namespace_uri<'a>(node: Node<'a, '_>) -> &'a str {
    js_trim(node.tag_name().namespace().unwrap_or(""))
}

/// Whether `node` is the element `local` in namespace `uri` (`""` for none).
pub fn is_element(node: Node<'_, '_>, uri: &str, local: &str) -> bool {
    node.is_element() && namespace_uri(node) == uri && node.tag_name().name() == local
}

/// The element children, without text.
pub fn elements_of<'a, 'input>(
    node: Node<'a, 'input>,
) -> impl Iterator<Item = Node<'a, 'input>> + use<'a, 'input> {
    node.children().filter(Node::is_element)
}

/// The first child element with this namespace and local name.
pub fn child_element<'a, 'input>(
    node: Node<'a, 'input>,
    uri: &str,
    local: &str,
) -> Option<Node<'a, 'input>> {
    node.children().find(|child| is_element(*child, uri, local))
}

/// Every child element with this namespace and local name.
pub fn child_elements<'a, 'input, 'n>(
    node: Node<'a, 'input>,
    uri: &'n str,
    local: &'n str,
) -> impl Iterator<Item = Node<'a, 'input>> + use<'a, 'input, 'n> {
    node.children()
        .filter(move |child| is_element(*child, uri, local))
}

/// The concatenated text of an element and its descendants.
pub fn text_content(node: Node<'_, '_>) -> String {
    node.descendants()
        .filter(Node::is_text)
        .filter_map(|text| text.text())
        .collect()
}

/// An element's attributes by name, namespace declarations included, in written order. Without `prefixes` each is
/// named as written (`r:id`). With `prefixes` (namespace URI, prefix), every namespaced attribute is named by its
/// namespace, whatever prefix the writer bound: `{prefix}:{local}` for a namespace the list holds, `{uri}local`
/// otherwise, so a reader matches attributes by one name and tells namespaces apart. Declarations and unprefixed
/// attributes keep their names.
pub fn attributes(
    node: Node<'_, '_>,
    prefixes: Option<&[(&str, &str)]>,
) -> IndexMap<String, String> {
    start_tag(node)
        .1
        .into_iter()
        .map(|attribute| {
            let name = match prefixes {
                Some(prefixes) if attribute.uri != XMLNS && attribute.qname.contains(':') => {
                    match prefixes.iter().find(|(uri, _)| *uri == attribute.uri) {
                        Some((_, prefix)) => format!("{prefix}:{}", attribute.local),
                        None => format!("{{{}}}{}", attribute.uri, attribute.local),
                    }
                }
                _ => attribute.qname.to_owned(),
            };
            (name, attribute.value.to_owned())
        })
        .collect()
}
