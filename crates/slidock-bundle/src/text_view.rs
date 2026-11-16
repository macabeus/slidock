//! The deck page's text view: `<article id="sd-text">`, the text of every shown slide as a reader and
//! an indexer see it below the stage, in the page's light DOM and before every large part of the file.
//!
//! One `<section id="slide-<n>">` per shown slide, in show order, headed by its a11y title (or `Slide <n>`), holding
//! the blocks of its composed slide (the slide's own elements, then its layout's; the background left out), fully
//! built:
//!
//! - skipped: `hidden` elements, `aria-hidden="true"` subtrees (bullet labels), SVG geometry, templates, media,
//!   slidock's own labels (`data-sd-bg`, `data-sd-marker`, `data-sd-failed`) and slide-number fields;
//! - each `<p>` and `<li>` with text is a block; lists keep `<ul>`/`<ol>`/`<li>`, and an item of a numbered list
//!   (`<ol>`) leads with the label its slide shows (`1.`, `b)`, `iv.`), whatever the items left out before it, so the
//!   view numbers no item the slide does not; a table keeps its rows and cells' text; `<br>` stays; other inline
//!   markup is flattened to text;
//! - an `<img alt>` with text, or a `role="img"` element's `aria-label`, is the block `Image: <alt>`;
//! - inline `<a href>` (http, https, mailto) stays; `data-link="slide:<id>"` links to that slide's section (plain text
//!   for a hidden one); `action:` links are unwrapped; an element linking to a URL or a slide links its blocks, or
//!   prints one link (its alt text, else the URL, else the target's heading) when it has none;
//! - the first run of consecutive blocks whose text is the heading (the title's paragraphs) is left out unless it
//!   holds a link, and so is a block the previous shown slide also shows (a multiset of collapsed text and hrefs), so
//!   a run of builds shows each block once. A slide left with nothing to say under the previous section's heading
//!   becomes an anchor at that section's end.
//!
//! Speaker notes join a section only when asked (`<details open>`). Every text and attribute is escaped.

use std::collections::BTreeMap;

use slidock_format::ComposedSlide;
use slidock_format::dom::{Document, NodeData, NodeId};
use slidock_format::escape::{escape_attribute, escape_text};
use slidock_format::markers::{BACKGROUND, BULLET, FAILED, FOREIGN_MARKER, attr};
use slidock_html::deck::{Deck, DeckSlide};

/// The text view's element id.
pub const TEXT_VIEW_ID: &str = "sd-text";

/// What a no-JavaScript visitor reads above the text.
pub const NOSCRIPT: &str = "The slides need JavaScript to play; their text follows.";

/// Elements whose content is never text a slide shows.
const SKIPPED_TAGS: &[&str] = &[
    "svg", "template", "script", "style", "video", "audio", "source", "track", "noscript", "math",
];

/// The URL schemes a link may keep (the lint's).
const LINK_SCHEMES: &[&str] = &["http", "https", "mailto"];

/// `<article id="sd-text">` for `deck` (the bundled deck JSON) and its composed slides, in `deck.slides` order. With
/// `notes` (the same slides, as compiled: the bundled deck JSON may carry no notes), each section prints its slide's.
pub fn text_view(
    deck: &Deck,
    composed: &[ComposedSlide],
    title: &str,
    notes: Option<&[DeckSlide]>,
) -> String {
    let mut numbers = BTreeMap::new();
    let mut count = 0u32;
    for slide in &deck.slides {
        if !slide.hidden {
            count += 1;
            numbers.insert(slide.id.as_str(), slide.number.unwrap_or(count));
        }
    }
    let headings: BTreeMap<&str, String> = deck
        .slides
        .iter()
        .filter_map(|slide| {
            let number = numbers.get(slide.id.as_str())?;
            Some((slide.id.as_str(), heading(&slide.title, *number)))
        })
        .collect();
    let links = Links {
        numbers: &numbers,
        headings: &headings,
    };

    let mut out = String::new();
    out.push_str(&format!(
        "<article id=\"{TEXT_VIEW_ID}\" aria-label=\"Slides as text\">\n<header><h1>{}</h1><noscript><p>{NOSCRIPT}</p></noscript></header>\n",
        escape_text(title)
    ));
    let mut sections: Vec<Section> = Vec::new();
    let mut previous: Vec<String> = Vec::new();
    for (index, (slide, composed)) in deck.slides.iter().zip(composed).enumerate() {
        let Some(number) = numbers.get(slide.id.as_str()).copied() else {
            continue;
        };
        let mut blocks = Vec::new();
        let document = composed.document();
        let mut walk = Walk {
            document,
            links: &links,
            out: &mut blocks,
        };
        for element in composed.reading_order() {
            walk.element(element, None);
        }
        let heading = headings
            .get(slide.id.as_str())
            .cloned()
            .unwrap_or_else(|| heading("", number));
        let title_text = collapse(&slide.title);
        let keys: Vec<String> = blocks.iter().map(Block::key).collect();
        let mut seen = previous;
        let title_run = title_run(&blocks, &title_text);
        let kept: Vec<&Block> = blocks
            .iter()
            .enumerate()
            .filter(|(at, _)| !title_run.contains(at))
            .map(|(_, block)| block)
            .filter(|block| {
                if let Some(at) = seen.iter().position(|key| *key == block.key()) {
                    seen.swap_remove(at);
                    return false;
                }
                true
            })
            .collect();
        previous = keys;
        let notes_html = notes
            .and_then(|slides| slides.get(index))
            .map(|slide| notes_details(&slide.notes))
            .unwrap_or_default();
        let anchor = format!("slide-{number}");
        if kept.is_empty()
            && notes_html.is_empty()
            && let Some(last) = sections.last_mut()
            && last.heading == heading
        {
            last.anchors
                .push_str(&format!("<span id=\"{anchor}\"></span>"));
            continue;
        }
        let mut body = format!(
            "<section id=\"{anchor}\"><h2><a href=\"#{anchor}\">{}</a></h2>\n",
            escape_text(&heading)
        );
        write_blocks(&kept, &mut body);
        body.push_str(&notes_html);
        sections.push(Section {
            heading,
            body,
            anchors: String::new(),
        });
    }
    for section in sections {
        out.push_str(&section.body);
        out.push_str(&section.anchors);
        out.push_str("</section>\n");
    }
    out.push_str("</article>");
    out
}

/// The blocks that say the slide's title (its title element's paragraphs, which the a11y title joins with spaces):
/// the first run of consecutive blocks whose texts, joined with spaces, are `title`, none of them holding a link; an
/// empty range when there is none.
fn title_run(blocks: &[Block], title: &str) -> std::ops::Range<usize> {
    if title.is_empty() {
        return 0..0;
    }
    for start in 0..blocks.len() {
        let mut joined = String::new();
        for (end, block) in blocks.iter().enumerate().skip(start) {
            if !block.hrefs.is_empty() {
                break;
            }
            if !joined.is_empty() {
                joined.push(' ');
            }
            joined.push_str(&block.text);
            if joined == title {
                return start..end + 1;
            }
            if !title.starts_with(joined.as_str()) {
                break;
            }
        }
    }
    0..0
}

/// A section's heading: the slide's title, or `Slide <n>`.
fn heading(title: &str, number: u32) -> String {
    let title = collapse(title);
    if title.is_empty() {
        format!("Slide {number}")
    } else {
        title
    }
}

struct Section {
    heading: String,
    body: String,
    /// The anchors of the slides folded into it.
    anchors: String,
}

/// The kind of a block, as the view writes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Paragraph,
    /// An item of the list `list` (`<ol>` when `ordered`).
    Item {
        list: NodeId,
        ordered: bool,
    },
    Table,
}

struct Block {
    kind: Kind,
    /// Its escaped markup inside its own element.
    html: String,
    /// Its text, whitespace collapsed.
    text: String,
    /// The links it holds, in order.
    hrefs: Vec<String>,
    /// The element link that holds it.
    link: Option<String>,
}

impl Block {
    /// What a repeat is matched by: the collapsed text and the links.
    fn key(&self) -> String {
        let mut key = self.text.clone();
        for href in &self.hrefs {
            key.push('\u{1}');
            key.push_str(href);
        }
        key
    }
}

/// Writes the kept blocks, consecutive items of one list in one list element.
fn write_blocks(blocks: &[&Block], out: &mut String) {
    let mut open: Option<(NodeId, bool)> = None;
    for block in blocks {
        let list = match block.kind {
            Kind::Item { list, ordered } => Some((list, ordered)),
            _ => None,
        };
        if open.is_some() && open != list {
            close_list(open.take(), out);
        }
        let linked = |html: &str| match &block.link {
            Some(href) => format!("<a href=\"{}\">{html}</a>", escape_attribute(href)),
            None => html.to_owned(),
        };
        match block.kind {
            Kind::Paragraph => out.push_str(&format!("<p>{}</p>\n", linked(&block.html))),
            Kind::Table => out.push_str(&format!(
                "{}\n",
                linked(&format!("<table>{}</table>", block.html))
            )),
            Kind::Item { ordered, .. } => {
                if open.is_none() {
                    out.push_str(if ordered { "<ol>\n" } else { "<ul>\n" });
                    open = list;
                }
                out.push_str(&format!("<li>{}</li>\n", linked(&block.html)));
            }
        }
    }
    close_list(open, out);
}

fn close_list(open: Option<(NodeId, bool)>, out: &mut String) {
    if let Some((_, ordered)) = open {
        out.push_str(if ordered { "</ol>\n" } else { "</ul>\n" });
    }
}

/// The notes as an open disclosure: one paragraph per non-blank note paragraph, line breaks as `<br>`.
fn notes_details(notes: &[String]) -> String {
    let paragraphs: Vec<String> = notes
        .iter()
        .filter(|paragraph| !paragraph.trim().is_empty())
        .map(|paragraph| {
            let lines: Vec<String> = paragraph.split('\n').map(escape_text).collect();
            format!("<p>{}</p>", lines.join("<br>"))
        })
        .collect();
    if paragraphs.is_empty() {
        return String::new();
    }
    format!(
        "<details open><summary>Speaker notes</summary>{}</details>\n",
        paragraphs.concat()
    )
}

/// Collapses whitespace runs to one space and trims.
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Where the deck's slide links lead.
struct Links<'a> {
    /// The shown number of each shown slide, by id.
    numbers: &'a BTreeMap<&'a str, u32>,
    headings: &'a BTreeMap<&'a str, String>,
}

/// Where a link points in the view.
struct Target {
    href: String,
    /// The text that names it when nothing else does.
    label: String,
}

impl Links<'_> {
    /// The view's link for an `href` (a URL the lint lets through) or a `data-link` value; `None` for an action, a
    /// hidden or unknown slide, or a URL of any other scheme.
    fn resolve(&self, href: Option<&str>, data_link: Option<&str>) -> Option<Target> {
        if let Some(url) = href {
            return safe_url(url).map(|url| Target {
                href: url.to_owned(),
                label: url.to_owned(),
            });
        }
        let value = data_link?;
        if let Some(url) = value.strip_prefix("url:") {
            return safe_url(url).map(|url| Target {
                href: url.to_owned(),
                label: url.to_owned(),
            });
        }
        let id = value.strip_prefix("slide:")?;
        let number = self.numbers.get(id)?;
        Some(Target {
            href: format!("#slide-{number}"),
            label: self.headings.get(id).cloned().unwrap_or_default(),
        })
    }
}

/// `url`, trimmed, when its scheme is one a link may keep.
fn safe_url(url: &str) -> Option<&str> {
    let url = url.trim();
    let (scheme, _) = url.split_once(':')?;
    LINK_SCHEMES
        .iter()
        .any(|allowed| scheme.eq_ignore_ascii_case(allowed))
        .then_some(url)
}

/// Inline content on its way into a block.
#[derive(Default)]
struct Inline {
    html: String,
    text: String,
    hrefs: Vec<String>,
}

impl Inline {
    fn text(&mut self, text: &str) {
        self.html.push_str(&escape_text(text));
        self.text.push_str(text);
    }

    fn line_break(&mut self) {
        self.html.push_str("<br>");
        self.text.push(' ');
    }
}

struct Walk<'d, 'l> {
    document: &'d Document,
    links: &'l Links<'l>,
    out: &'d mut Vec<Block>,
}

impl Walk<'_, '_> {
    /// Whether an element and its subtree say nothing in the view.
    fn skipped(&self, element: NodeId) -> bool {
        let document = self.document;
        let tag = document.tag_name(element);
        SKIPPED_TAGS.contains(&tag)
            || document.has_attribute(element, "hidden")
            || document
                .attribute(element, "aria-hidden")
                .is_some_and(|value| value.trim().eq_ignore_ascii_case("true"))
            || document.has_attribute(element, BACKGROUND)
            || document.has_attribute(element, FOREIGN_MARKER)
            || document.has_attribute(element, FAILED)
            || document.attribute(element, attr::FIELD) == Some(attr::SLIDE_NUMBER_FIELD)
    }

    /// The blocks of `element` and its subtree; `link` is the href of an enclosing element link.
    fn element(&mut self, element: NodeId, link: Option<&str>) {
        if self.skipped(element) {
            return;
        }
        let document = self.document;
        let tag = document.tag_name(element);
        if tag != "a"
            && link.is_none()
            && let Some(target) = self
                .links
                .resolve(None, document.attribute(element, "data-link"))
        {
            let start = self.out.len();
            self.contents(element, Some(&target.href));
            if self.out.len() == start {
                let label = self
                    .alt(element)
                    .map(|alt| format!("Image: {alt}"))
                    .unwrap_or(target.label);
                let mut inline = Inline::default();
                inline.text(&label);
                self.push(Kind::Paragraph, inline, Some(&target.href));
            }
            return;
        }
        self.contents(element, link);
    }

    /// An element's own blocks and its children's.
    fn contents(&mut self, element: NodeId, link: Option<&str>) {
        let document = self.document;
        match document.tag_name(element) {
            "p" | "li" => {
                let kind = match document.parent(element) {
                    Some(list) if document.tag_name(element) == "li" => Kind::Item {
                        list,
                        ordered: document.tag_name(list) == "ol",
                    },
                    _ => Kind::Paragraph,
                };
                let mut inline = Inline::default();
                if let Kind::Item { ordered: true, .. } = kind
                    && let Some(label) = self.label(element)
                {
                    inline.text(&label);
                    inline.text(" ");
                }
                self.inline(element, link.is_some(), &mut inline);
                self.push(kind, inline, link);
            }
            "table" => self.table(element, link),
            "img" => {
                if let Some(alt) = self.alt(element) {
                    let mut inline = Inline::default();
                    inline.text(&format!("Image: {alt}"));
                    self.push(Kind::Paragraph, inline, link);
                }
            }
            _ => {
                if document.attribute(element, "role") == Some("img")
                    && let Some(label) = self.alt(element)
                {
                    let mut inline = Inline::default();
                    inline.text(&format!("Image: {label}"));
                    self.push(Kind::Paragraph, inline, link);
                }
                for child in document.child_elements(element) {
                    self.element(child, link);
                }
            }
        }
    }

    /// An image's alt text, or a `role="img"` element's label, when it has text.
    fn alt(&self, element: NodeId) -> Option<String> {
        let document = self.document;
        let alt = if document.tag_name(element) == "img" {
            document.attribute(element, "alt")
        } else if document.attribute(element, "role") == Some("img") {
            document.attribute(element, "aria-label")
        } else {
            None
        }?;
        let alt = collapse(alt);
        (!alt.is_empty()).then_some(alt)
    }

    /// The label a list item shows (its `data-sd-bullet` child's text, which is `aria-hidden`), when it has one.
    fn label(&self, item: NodeId) -> Option<String> {
        let document = self.document;
        let label = document
            .child_elements(item)
            .into_iter()
            .find(|child| document.has_attribute(*child, BULLET))?;
        let mut text = Inline::default();
        self.inline(label, true, &mut text);
        let text = collapse(&text.text);
        (!text.is_empty()).then_some(text)
    }

    /// Adds a block when it has text; `link` is the element link that holds it.
    fn push(&mut self, kind: Kind, inline: Inline, link: Option<&str>) {
        let text = collapse(&inline.text);
        if text.is_empty() {
            return;
        }
        let hrefs = match link {
            Some(href) => vec![href.to_owned()],
            None => inline.hrefs,
        };
        self.out.push(Block {
            kind,
            html: inline.html,
            text,
            hrefs,
            link: link.map(str::to_owned),
        });
    }

    /// A table's rows and cells' text, as one block.
    fn table(&mut self, table: NodeId, link: Option<&str>) {
        let document = self.document;
        let mut rows = String::new();
        let mut text = String::new();
        let mut hrefs = Vec::new();
        for row in document
            .walk(table)
            .into_iter()
            .filter(|row| document.tag_name(*row) == "tr" && !self.skipped(*row))
        {
            rows.push_str("<tr>");
            for cell in document.child_elements(row) {
                let tag = document.tag_name(cell);
                if !matches!(tag, "td" | "th") {
                    continue;
                }
                let mut inline = Inline::default();
                if !self.skipped(cell) {
                    self.cell(cell, link.is_some(), &mut inline);
                }
                let span = |name: &str| {
                    document
                        .attribute(cell, name)
                        .map(|value| format!(" {name}=\"{}\"", escape_attribute(value)))
                        .unwrap_or_default()
                };
                rows.push_str(&format!(
                    "<{tag}{}{}>{}</{tag}>",
                    span("colspan"),
                    span("rowspan"),
                    inline.html
                ));
                text.push(' ');
                text.push_str(&inline.text);
                hrefs.append(&mut inline.hrefs);
            }
            rows.push_str("</tr>");
        }
        let inline = Inline {
            html: rows,
            text,
            hrefs,
        };
        self.push(Kind::Table, inline, link);
    }

    /// A cell's text: its paragraphs' inline content, one line each.
    fn cell(&self, cell: NodeId, linked: bool, inline: &mut Inline) {
        let document = self.document;
        let mut first = true;
        for child in document.children(cell) {
            match document.data(*child) {
                Some(NodeData::Text(text)) => inline.text(text),
                Some(NodeData::Element { .. }) if !self.skipped(*child) => {
                    let mut part = Inline::default();
                    self.inline(*child, linked, &mut part);
                    if collapse(&part.text).is_empty() {
                        continue;
                    }
                    if !first {
                        inline.line_break();
                    }
                    first = false;
                    inline.html.push_str(&part.html);
                    inline.text.push_str(&part.text);
                    inline.hrefs.append(&mut part.hrefs);
                }
                _ => {}
            }
        }
    }

    /// The inline content of `element`'s children: text, `<br>` and links; everything else flattened. `linked`: an
    /// enclosing element link already holds it, so no link nests inside.
    fn inline(&self, element: NodeId, linked: bool, out: &mut Inline) {
        let document = self.document;
        for child in document.children(element) {
            match document.data(*child) {
                Some(NodeData::Text(text)) => out.text(text),
                Some(NodeData::Element { .. }) => {
                    if self.skipped(*child) {
                        continue;
                    }
                    match document.tag_name(*child) {
                        "br" => out.line_break(),
                        "a" if !linked => {
                            let target = self.links.resolve(
                                document.attribute(*child, "href"),
                                document.attribute(*child, "data-link"),
                            );
                            match target {
                                Some(target) => {
                                    let mut inner = Inline::default();
                                    self.inline(*child, true, &mut inner);
                                    out.html.push_str(&format!(
                                        "<a href=\"{}\">{}</a>",
                                        escape_attribute(&target.href),
                                        inner.html
                                    ));
                                    out.text.push_str(&inner.text);
                                    out.hrefs.push(target.href);
                                }
                                None => self.inline(*child, linked, out),
                            }
                        }
                        _ => self.inline(*child, linked, out),
                    }
                }
                _ => {}
            }
        }
    }
}
