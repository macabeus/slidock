//! The deck page's text view and metadata on a small deck: shown slides on a layout with a footer
//! and a slide-number field, a hidden slide, repeated blocks across a run of builds, links of every kind, pictures with
//! and without alt text, a table, and speaker notes.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

use std::collections::BTreeMap;

use slidock_bundle::page::{Page, index_html};
use slidock_bundle::{
    BundleError, BundleOptions, CompiledDeck, DeckMedia, INDEX_PATH, LICENSES_PATH, NotesMode,
    Packaging, bundle, page_defaults, player,
};
use slidock_format::dom::{Document, NodeId};
use slidock_format::manifest::{
    Manifest, ManifestLayout, ManifestSize, ManifestSlide, ManifestSource, SizeUnit,
};
use slidock_format::{RESET_STYLE, serialize_manifest};
use slidock_html::Documents;
use slidock_html::deck::{Deck, DeckFormat, DeckSize, DeckSlide, DeckSource, DiagnosticSummary};

const BOX: &str = "position:relative;width:720pt;height:405pt;overflow:hidden";

/// A shape holding paragraphs.
fn shape(id: &str, attrs: &str, paragraphs: &str) -> String {
    format!(
        "<div data-el=\"{id}\" data-kind=\"shape\"{attrs} style=\"position:absolute;left:0pt;top:0pt;width:100pt\"><div data-sd-text style=\"position:absolute;left:0pt;top:0pt;width:100%\">{paragraphs}</div></div>"
    )
}

fn document(title: &str, main: &str, children: &[String]) -> String {
    let mut lines = vec![
        "<!doctype html>".to_owned(),
        "<html>".to_owned(),
        "<head>".to_owned(),
        "<meta charset=\"utf-8\">".to_owned(),
        format!("<title>{title}</title>"),
        format!("<style>{RESET_STYLE}</style>"),
        "</head>".to_owned(),
        "<body>".to_owned(),
        format!("<main {main} style=\"{BOX}\">"),
    ];
    lines.extend(children.iter().cloned());
    lines.extend(["</main>", "</body>", "</html>", ""].map(str::to_owned));
    lines.join("\n")
}

/// A slide of the fixture: id, title, hidden, notes, its elements.
struct Slide {
    id: &'static str,
    title: &'static str,
    hidden: bool,
    notes: Vec<String>,
    elements: Vec<String>,
}

fn slide(id: &'static str, title: &'static str, elements: Vec<String>) -> Slide {
    Slide {
        id,
        title,
        hidden: false,
        notes: Vec::new(),
        elements,
    }
}

fn p(text: &str) -> String {
    format!("<p style=\"line-height:1.2\">{text}</p>")
}

fn slides() -> Vec<Slide> {
    let mut first = slide(
        "s1",
        "Welcome",
        vec![
            shape("s1.1", "", &p("Welcome")),
            shape(
                "s1.2",
                "",
                &[
                    p("Own paragraph"),
                    p("Shared line"),
                    p("Dup"),
                    p("Dup"),
                    p("&lt;/article&gt;&lt;script&gt;alert(1)&lt;/script&gt;"),
                    p("<span style=\"font-size:9pt\">Inline <b>flattened</b></span> line<br>after break"),
                ]
                .concat(),
            ),
            shape(
                "s1.3",
                "",
                "<ul style=\"margin:0\"><li><span data-sd-bullet aria-hidden=\"true\">•</span>Point A</li><li></li><li><span data-sd-bullet aria-hidden=\"true\">•</span>Point B</li></ul>",
            ),
            "<img data-el=\"s1.4\" data-kind=\"picture\" alt=\"A cat\" style=\"position:absolute;left:0pt;top:0pt;width:10pt\">".into(),
            "<img data-el=\"s1.5\" data-kind=\"picture\" alt=\"\" style=\"position:absolute;left:0pt;top:0pt;width:10pt\">".into(),
            "<div data-el=\"s1.6\" data-kind=\"shape\" role=\"img\" aria-label=\"A chart\" style=\"position:absolute;left:0pt;top:0pt;width:10pt\"><svg data-sd-geom><text>geometry</text></svg></div>".into(),
            shape("s1.7", " hidden", &p("Hidden element text")),
            shape(
                "s1.8",
                "",
                &[
                    p("<a href=\"https://example.com/a\" style=\"color:inherit\">Example</a> and <a href=\"https://example.com/?q=&quot;x&quot;\">quote</a>"),
                    p("Go <a data-link=\"slide:s4\">to four</a>, <a data-link=\"slide:h\">to hidden</a>, <a data-link=\"action:next\">onwards</a>"),
                ]
                .concat(),
            ),
            shape("s1.9", " data-link=\"url:https://example.com/element\"", &p("Element linked")),
            "<div data-el=\"s1.10\" data-kind=\"picture\" data-link=\"url:https://example.com/picture\" style=\"position:absolute;left:0pt;top:0pt;width:10pt\"><img alt=\"\" style=\"width:100%\"></div>".into(),
            "<div data-el=\"s1.11\" data-kind=\"picture\" data-link=\"slide:s5\" style=\"position:absolute;left:0pt;top:0pt;width:10pt\"><img alt=\"\" style=\"width:100%\"></div>".into(),
            "<div data-el=\"s1.12\" data-kind=\"shape\" data-link=\"action:last\" style=\"position:absolute;left:0pt;top:0pt;width:10pt\"><div data-sd-text>".to_owned() + &p("Action element") + "</div></div>",
            "<table data-el=\"s1.13\" data-kind=\"table\" style=\"position:absolute;left:0pt;top:0pt;width:100pt\"><colgroup><col style=\"width:50pt\"></colgroup><tbody><tr><td>".to_owned() + &p("Cell A") + &p("Cell A2") + "</td><td colspan=\"2\">" + &p("Cell B") + "</td></tr></tbody></table>",
            "<div data-el=\"s1.14\" data-kind=\"shape\" data-sd-marker style=\"position:absolute;left:0pt;top:0pt;width:10pt\">MARKER</div>".into(),
        ],
    );
    first.notes = vec![
        "First note\nsecond line".into(),
        " ".into(),
        "Para 2 <b>".into(),
    ];
    let mut hidden = slide(
        "h",
        "Hidden slide",
        vec![shape("h.1", "", &p("Secret hidden text"))],
    );
    hidden.hidden = true;
    hidden.notes = vec!["Hidden slide note".into()];
    vec![
        first,
        hidden,
        slide(
            "s3",
            "Second",
            vec![shape(
                "s3.1",
                "",
                &[
                    p("Shared line"),
                    p("Only two"),
                    p("Dup"),
                    p("Dup"),
                    p("Dup"),
                ]
                .concat(),
            )],
        ),
        slide("s4", "Second", vec![shape("s4.1", "", &p("Only two"))]),
        slide("s5", "Fourth", vec![shape("s5.1", "", &p("Shared line"))]),
        slide("s6", "", vec![shape("s6.1", "", &p("Untitled body"))]),
    ]
}

fn layout() -> String {
    document(
        "Layout",
        "data-layout-root=\"l1\"",
        &[
            "<div data-sd-bg style=\"position:absolute;left:0pt;top:0pt;width:720pt;height:405pt;background:#202020\"><p>Background text</p></div>".into(),
            shape(
                "l1.foot",
                "",
                &p("Footer text"),
            ),
            shape(
                "l1.num",
                "",
                &p("Page <span data-field=\"slideNumber\">‹#›</span>"),
            ),
        ],
    )
}

struct NoMedia;

impl DeckMedia for NoMedia {
    fn write_media(&self, path: &str, _: &mut dyn std::io::Write) -> Result<(), BundleError> {
        Err(BundleError::Media {
            path: path.to_owned(),
            message: "none".into(),
        })
    }
}

fn compiled(slides: &[Slide]) -> (Deck, Documents) {
    let manifest = Manifest {
        title: "fixture copy".into(),
        size: ManifestSize {
            w: 720.0,
            h: 405.0,
            unit: SizeUnit::Pt,
        },
        slides: slides
            .iter()
            .map(|s| ManifestSlide {
                id: s.id.into(),
                file: format!("slides/{}.html", s.id),
                title: s.title.into(),
                hidden: s.hidden,
                layout: Some("l1".into()),
                notes: None,
                transition: None,
                failed: None,
            })
            .collect(),
        layouts: vec![ManifestLayout {
            id: "l1".into(),
            file: "layouts/l1.html".into(),
            name: Some("Layout".into()),
            theme: None,
            failed: None,
        }],
        themes: BTreeMap::new(),
        source: Some(ManifestSource {
            format: "ir".into(),
            name: "fixture copy.presentation-ir.json".into(),
            sha256: "a".repeat(64),
            bytes: 10,
        }),
        generator: None,
    };
    let mut text = BTreeMap::from([
        (
            "presentation.json".to_owned(),
            serialize_manifest(&manifest).unwrap(),
        ),
        ("layouts/l1.html".to_owned(), layout()),
    ]);
    for s in slides {
        text.insert(
            format!("slides/{}.html", s.id),
            document(
                s.title,
                &format!("data-slide=\"{}\" data-layout=\"l1\"", s.id),
                &s.elements,
            ),
        );
    }
    let mut number = 0;
    let deck = Deck {
        format: DeckFormat::V1,
        player: None,
        id: String::new(),
        title: "fixture copy".into(),
        lang: None,
        size: DeckSize { w: 720.0, h: 405.0 },
        source: DeckSource {
            format: "ir".into(),
            name: "fixture copy.presentation-ir.json".into(),
            sha256: "a".repeat(64),
            bytes: 10,
        },
        slides: slides
            .iter()
            .map(|s| DeckSlide {
                id: s.id.into(),
                layout: Some("l1".into()),
                hidden: s.hidden,
                number: (!s.hidden).then(|| {
                    number += 1;
                    number
                }),
                title: s.title.into(),
                notes: s.notes.clone(),
                failed: None,
                timeline: Vec::new(),
                transition: None,
                match_keys: BTreeMap::new(),
                fonts: Vec::new(),
            })
            .collect(),
        media: Vec::new(),
        fonts: Vec::new(),
        diagnostics: DiagnosticSummary::default(),
    };
    let documents = Documents {
        text,
        media: BTreeMap::new(),
        faces: BTreeMap::new(),
    };
    (deck, documents)
}

/// Every file of a bundle of the fixture with `options`, by path, unpacked from the zip.
fn zip_files(options: &BundleOptions) -> (slidock_bundle::BundleReport, BTreeMap<String, Vec<u8>>) {
    use slidock_io::RangeReader as _;
    let (deck, documents) = compiled(&slides());
    let compiled = CompiledDeck {
        deck,
        documents,
        media: &NoMedia,
        fonts: Vec::new(),
    };
    struct Zip(Vec<u8>);
    impl slidock_bundle::BundleSink for Zip {
        fn write(&mut self, _: &str, bytes: &[u8]) -> Result<(), BundleError> {
            self.0.extend_from_slice(bytes);
            Ok(())
        }
    }
    let mut zip = Zip(Vec::new());
    let options = BundleOptions {
        packaging: Packaging::Zip,
        ..options.clone()
    };
    let report = bundle(&compiled, &options, &mut zip).unwrap();
    let archive = slidock_io::zip::ZipArchive::open(
        slidock_io::BytesReader::new(zip.0),
        slidock_io::zip::ZipLimits::default(),
    )
    .unwrap();
    let files = archive
        .entries()
        .iter()
        .map(|entry| {
            let reader = archive.open_entry(entry).unwrap();
            let bytes = reader.read_at(0, reader.len() as usize).unwrap();
            (entry.name.clone(), bytes.to_vec())
        })
        .collect();
    (report, files)
}

fn page_with(slides: &[Slide], options: &BundleOptions) -> Result<String, BundleError> {
    let (deck, documents) = compiled(slides);
    let compiled = CompiledDeck {
        deck,
        documents,
        media: &NoMedia,
        fonts: Vec::new(),
    };
    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut sink = |path: &str, bytes: &[u8]| {
        files.entry(path.to_owned()).or_default().extend(bytes);
    };
    struct Sink<'a>(&'a mut dyn FnMut(&str, &[u8]));
    impl slidock_bundle::BundleSink for Sink<'_> {
        fn write(&mut self, path: &str, bytes: &[u8]) -> Result<(), BundleError> {
            (self.0)(path, bytes);
            Ok(())
        }
    }
    let options = BundleOptions {
        packaging: Packaging::Files,
        ..options.clone()
    };
    bundle(&compiled, &options, &mut Sink(&mut sink))?;
    Ok(String::from_utf8(files.remove(INDEX_PATH).unwrap()).unwrap())
}

fn page(options: &BundleOptions) -> String {
    page_with(&slides(), options).unwrap()
}

/// The `<article id="sd-text">` of a page, as written.
fn view(page: &str) -> &str {
    let start = page.find("<article id=\"sd-text\"").expect("a text view");
    let end = page[start..].find("</article>").unwrap() + start + "</article>".len();
    &page[start..end]
}

/// The markup of the section `id`, as written.
fn section<'p>(view: &'p str, id: &str) -> &'p str {
    let start = view
        .find(&format!("<section id=\"{id}\">"))
        .unwrap_or_else(|| panic!("no section {id}"));
    let end = view[start..].find("</section>").unwrap() + start;
    &view[start..end]
}

/// The text of a parsed node and its descendants, `<template>` and raw-text elements left out.
fn text_of(document: &Document, node: NodeId) -> String {
    let mut out = String::new();
    for child in document.children(node) {
        match document.data(*child) {
            Some(slidock_format::dom::NodeData::Text(text)) => out.push_str(text),
            Some(slidock_format::dom::NodeData::Element { .. })
                if !matches!(
                    document.tag_name(*child),
                    "noscript" | "template" | "script" | "style"
                ) =>
            {
                out.push(' ');
                out.push_str(&text_of(document, *child));
            }
            _ => {}
        }
    }
    out
}

fn article_text(page: &str) -> String {
    let document = Document::parse(page);
    let article = document
        .walk(NodeId::DOCUMENT)
        .into_iter()
        .find(|e| document.attribute(*e, "id") == Some("sd-text"))
        .unwrap();
    text_of(&document, article)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn holds_the_shown_slides_only_in_show_order() {
    let page = page(&BundleOptions::default());
    let view = view(&page);
    assert!(!view.contains("Secret hidden text"), "{view}");
    assert!(!view.contains("Hidden slide"), "{view}");
    let order: Vec<usize> = ["slide-1", "slide-2", "slide-4", "slide-5"]
        .iter()
        .map(|id| view.find(&format!("<section id=\"{id}\">")).unwrap())
        .collect();
    assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "{order:?}");
    assert!(view.contains("<section id=\"slide-1\"><h2><a href=\"#slide-1\">Welcome</a></h2>"));
    assert!(view.contains("<section id=\"slide-5\"><h2><a href=\"#slide-5\">Slide 5</a></h2>"));
    assert!(view.starts_with(
        "<article id=\"sd-text\" aria-label=\"Slides as text\">\n<header><h1>Welcome</h1><noscript><p>The slides need JavaScript to play; their text follows.</p></noscript></header>\n"
    ));
    let document = Document::parse(&page);
    let tags = |tag: &str| {
        document
            .walk(NodeId::DOCUMENT)
            .into_iter()
            .filter(|e| document.tag_name(*e) == tag)
            .count()
    };
    assert_eq!(tags("h1"), 1);
    assert_eq!(tags("h2"), 4);
}

#[test]
fn reads_the_slides_own_elements_before_its_layouts_and_leaves_out_the_background() {
    let page = page(&BundleOptions::default());
    let first = section(view(&page), "slide-1");
    let own = first.find("Own paragraph").unwrap();
    let footer = first.find("Footer text").unwrap();
    assert!(own < footer, "{first}");
    assert!(!page[..page.find("</article>").unwrap()].contains("Background text"));
}

#[test]
fn leaves_out_slide_number_fields_markers_hidden_elements_and_geometry() {
    let page = page(&BundleOptions::default());
    let first = section(view(&page), "slide-1");
    assert!(first.contains("<p>Page </p>"), "{first}");
    assert!(!first.contains("Page 1"), "{first}");
    for absent in ["Hidden element text", "MARKER", "geometry", "•"] {
        assert!(!first.contains(absent), "{absent}: {first}");
    }
}

#[test]
fn writes_blocks_lists_tables_and_line_breaks() {
    let page = page(&BundleOptions::default());
    let first = section(view(&page), "slide-1");
    // The first block that repeats the heading is the heading.
    assert_eq!(first.matches("Welcome").count(), 1, "{first}");
    assert!(
        first.contains("<p>Inline flattened line<br>after break</p>"),
        "{first}"
    );
    assert!(
        first.contains("<ul>\n<li>Point A</li>\n<li>Point B</li>\n</ul>\n"),
        "{first}"
    );
    assert!(
        first.contains(
            "<table><tr><td>Cell A<br>Cell A2</td><td colspan=\"2\">Cell B</td></tr></table>"
        ),
        "{first}"
    );
}

#[test]
fn names_pictures_by_their_alt_text_only() {
    let page = page(&BundleOptions::default());
    let first = section(view(&page), "slide-1");
    assert!(first.contains("<p>Image: A cat</p>"), "{first}");
    assert!(first.contains("<p>Image: A chart</p>"), "{first}");
    assert_eq!(first.matches("Image:").count(), 2, "{first}");
    assert!(!first.contains("<p></p>"), "{first}");
    assert!(!first.contains("<img"), "{first}");
}

#[test]
fn keeps_urls_maps_slide_links_and_unwraps_actions() {
    let page = page(&BundleOptions::default());
    let first = section(view(&page), "slide-1");
    assert!(
        first.contains("<a href=\"https://example.com/a\">Example</a>"),
        "{first}"
    );
    assert!(
        first.contains("<p>Go <a href=\"#slide-3\">to four</a>, to hidden, onwards</p>"),
        "{first}"
    );
    assert!(
        first.contains("<p><a href=\"https://example.com/element\">Element linked</a></p>"),
        "{first}"
    );
    // A picture link with nothing to read prints its URL, or its target's heading.
    assert!(
        first.contains(
            "<p><a href=\"https://example.com/picture\">https://example.com/picture</a></p>"
        ),
        "{first}"
    );
    assert!(
        first.contains("<p><a href=\"#slide-4\">Fourth</a></p>"),
        "{first}"
    );
    assert!(first.contains("<p>Action element</p>"), "{first}");
    assert!(!first.contains("data-link"), "{first}");
}

#[test]
fn escapes_text_and_attributes() {
    let page = page(&BundleOptions::default());
    let first = section(view(&page), "slide-1");
    assert!(
        first.contains("<p>&lt;/article&gt;&lt;script&gt;alert(1)&lt;/script&gt;</p>"),
        "{first}"
    );
    assert!(
        first.contains("<a href=\"https://example.com/?q=&quot;x&quot;\">quote</a>"),
        "{first}"
    );
    let document = Document::parse(&page);
    let scripts = document
        .walk(NodeId::DOCUMENT)
        .into_iter()
        .filter(|e| document.tag_name(*e) == "script")
        .count();
    assert_eq!(
        scripts, 4,
        "the deck JSON, the player, the presenter view and the boot"
    );
    let hrefs: Vec<&str> = document
        .walk(NodeId::DOCUMENT)
        .into_iter()
        .filter_map(|e| document.attribute(e, "href"))
        .collect();
    assert!(hrefs.contains(&"https://example.com/?q=\"x\""), "{hrefs:?}");
}

#[test]
fn prints_a_block_a_run_of_slides_repeats_once_and_folds_empty_repeats_into_an_anchor() {
    let page = page(&BundleOptions::default());
    let view = view(&page);
    let second = section(view, "slide-2");
    // Shown on slide 1: left out on slide 2. Twice on slide 1, three times on slide 2: once more.
    assert!(!second.contains("Shared line"), "{second}");
    assert_eq!(second.matches("<p>Dup</p>").count(), 1, "{second}");
    assert!(second.contains("<p>Only two</p>"), "{second}");
    assert!(!second.contains("Footer text"), "{second}");
    // Slide 3 repeats slide 2 under its heading: an anchor at the end of slide 2's section.
    assert!(!view.contains("<section id=\"slide-3\">"), "{view}");
    assert!(second.ends_with("<span id=\"slide-3\"></span>"), "{second}");
    // Not on slide 3, so shown again on slide 4.
    assert!(
        section(view, "slide-4").contains("<p>Shared line</p>"),
        "{view}"
    );
    assert_eq!(view.matches("<p>Shared line</p>").count(), 2, "{view}");
}

#[test]
fn writes_notes_only_on_the_page_mode_and_drops_them_with_none() {
    let deck_json = |page: &str| {
        let start = page.find("id=\"sd-deck\">").unwrap() + "id=\"sd-deck\">".len();
        let end = page[start..].find("</script>").unwrap() + start;
        Deck::from_json(&page[start..end]).unwrap()
    };
    let presenter = page(&BundleOptions::default());
    assert!(!view(&presenter).contains("First note"));
    assert!(!view(&presenter).contains("Speaker notes"));
    assert_eq!(deck_json(&presenter).slides[0].notes.len(), 3);

    let on_page = page(&BundleOptions {
        notes: NotesMode::Page,
        ..BundleOptions::default()
    });
    let first = section(view(&on_page), "slide-1");
    assert!(
        first.ends_with(
            "<details open><summary>Speaker notes</summary><p>First note<br>second line</p><p>Para 2 &lt;b&gt;</p></details>\n"
        ),
        "{first}"
    );
    assert!(!view(&on_page).contains("Hidden slide note"));
    assert_eq!(view(&on_page).matches("<details").count(), 1);

    let none = page(&BundleOptions {
        notes: NotesMode::None,
        ..BundleOptions::default()
    });
    assert!(!view(&none).contains("First note"));
    assert!(deck_json(&none).slides.iter().all(|s| s.notes.is_empty()));
    assert!(!none.contains("First note"));
}

/// The presenter view on and off, with each notes mode, as a zip and as a single file: the files written, the page's
/// scripts, the deck JSON's player and notes, and every place the notes' text appears.
#[test]
fn ships_the_presenter_view_and_the_notes_only_where_asked() {
    let deck_json = |page: &str| {
        let start = page.find("id=\"sd-deck\">").unwrap() + "id=\"sd-deck\">".len();
        let end = page[start..].find("</script>").unwrap() + start;
        Deck::from_json(&page[start..end]).unwrap()
    };
    for presenter in [true, false] {
        for notes in [NotesMode::Presenter, NotesMode::Page, NotesMode::None] {
            for single_file in [false, true] {
                let at =
                    format!("presenter {presenter}, notes {notes:?}, single file {single_file}");
                let options = BundleOptions {
                    presenter,
                    notes,
                    single_file,
                    ..BundleOptions::default()
                };
                let (report, files) = zip_files(&options);
                assert_eq!(report.single_file, single_file, "{at}");
                assert_eq!(
                    files.keys().map(String::as_str).collect::<Vec<_>>(),
                    [LICENSES_PATH, INDEX_PATH],
                    "{at}"
                );
                let index = std::str::from_utf8(&files[INDEX_PATH]).unwrap();
                assert_eq!(
                    index.matches("<script").count(),
                    if presenter { 4 } else { 3 },
                    "{at}"
                );
                assert_eq!(index.contains(player::PRESENTER), presenter, "{at}");
                assert_eq!(
                    index.contains(player::BOOT_WITH_PRESENTER),
                    presenter,
                    "{at}"
                );
                assert_eq!(index.contains("SlidockPresenter"), presenter, "{at}");
                let deck = deck_json(index);
                assert_eq!(deck.player.unwrap().presenter.is_some(), presenter, "{at}");

                let in_deck = presenter && notes != NotesMode::None;
                assert_eq!(options.notes_in_deck(), in_deck, "{at}");
                assert_eq!(
                    deck.slides
                        .iter()
                        .map(|s| s.notes.len())
                        .collect::<Vec<_>>(),
                    if in_deck { [3, 1, 0, 0, 0, 0] } else { [0; 6] },
                    "{at}"
                );
                let on_page = notes == NotesMode::Page;
                assert_eq!(view(index).contains("Speaker notes"), on_page, "{at}");
                assert_eq!(view(index).contains("First note"), on_page, "{at}");
                // A hidden slide's notes never reach the view: only the presenter view shows them.
                assert!(!view(index).contains("Hidden slide note"), "{at}");
                let shipped = |text: &str| {
                    files
                        .values()
                        .any(|bytes| String::from_utf8_lossy(bytes).contains(text))
                };
                assert_eq!(shipped("First note"), in_deck || on_page, "{at}");
                assert_eq!(shipped("Hidden slide note"), in_deck, "{at}");
                assert_eq!(shipped("Para 2"), in_deck || on_page, "{at}");
            }
        }
    }
}

#[test]
fn leaves_the_view_out_on_request() {
    let page = page(&BundleOptions {
        text_view: false,
        ..BundleOptions::default()
    });
    assert!(!page.contains("sd-text\" aria-label"));
    assert!(page.contains(
        "<body>\n<noscript><p>This presentation needs JavaScript to play.</p></noscript>\n"
    ));
}

#[test]
fn ends_the_view_before_the_fonts_the_deck_json_and_the_templates() {
    let page = page(&BundleOptions::default());
    let end = page.find("</article>").unwrap();
    for later in [
        "<style id=\"sd-fonts\">",
        "<script type=\"application/json\" id=\"sd-deck\">",
        "<template data-sd-slide=",
        "<script>",
    ] {
        assert!(end < page.find(later).unwrap(), "{later}");
    }
    assert!(page.find("<body>").unwrap() < page.find("<article").unwrap());
    assert!(page.find("</head>").unwrap() < page.find("<style id=\"sd-fonts\">").unwrap());

    // Fonts weigh what they weigh: the view still ends inside the first 2 MiB.
    let (deck, documents) = compiled(&slides());
    let view = slidock_bundle::text_view::text_view(&deck, &[], "x", None);
    let rule = format!(
        "@font-face{{src:url(data:font/woff2;base64,{})}}",
        "A".repeat(2_500_000)
    );
    let rules = [rule];
    let html = index_html(&Page {
        deck: &deck,
        font_rules: &rules,
        templates: &[],
        description: None,
        site_url: None,
        text_view: Some(&view),
        presenter: true,
    })
    .unwrap();
    drop(documents);
    assert!(html.len() > 2_500_000);
    let end = html.find("</article>").unwrap();
    assert!(end < 2_097_152, "{end}");
    assert!(end < html.find("<style id=\"sd-fonts\">").unwrap());
}

#[test]
fn heads_the_page_with_its_description_and_link_preview_tags() {
    let page = page(&BundleOptions::default());
    let head = &page[..page.find("</head>").unwrap()];
    assert!(head.contains("<title>Welcome</title>"), "{head}");
    for tag in [
        "<meta name=\"description\" content=\"Second · Fourth\">",
        "<meta property=\"og:type\" content=\"website\">",
        "<meta property=\"og:title\" content=\"Welcome\">",
        "<meta property=\"og:description\" content=\"Second · Fourth\">",
        "<meta name=\"twitter:card\" content=\"summary\">",
    ] {
        assert!(head.contains(tag), "{tag}: {head}");
    }
    assert!(
        !head.contains("canonical") && !head.contains("og:url"),
        "{head}"
    );
    assert!(
        !head.contains("og:image") && !head.contains("robots"),
        "{head}"
    );
    assert!(head.find("<title>").unwrap() < head.find("<style id=\"sd-chrome\">").unwrap());

    let given = page_with(
        &slides(),
        &BundleOptions {
            title: Some("My \"talk\"".into()),
            description: Some(String::new()),
            site_url: Some("https://example.com/talk/".into()),
            ..BundleOptions::default()
        },
    )
    .unwrap();
    let head = &given[..given.find("</head>").unwrap()];
    assert!(head.contains("<title>My \"talk\"</title>"), "{head}");
    assert!(
        head.contains("<meta property=\"og:title\" content=\"My &quot;talk&quot;\">"),
        "{head}"
    );
    assert!(!head.contains("description"), "{head}");
    assert!(
        head.contains("<link rel=\"canonical\" href=\"https://example.com/talk/\">"),
        "{head}"
    );
    assert!(
        head.contains("<meta property=\"og:url\" content=\"https://example.com/talk/\">"),
        "{head}"
    );
    assert!(view(&given).contains("<h1>My \"talk\"</h1>"));
}

#[test]
fn refuses_a_public_url_that_is_not_absolute_http() {
    for bad in [
        "ftp://example.com/talk",
        "/talk/",
        "https://u:p@example.com/",
        "https://example.com/#x",
    ] {
        let result = page_with(
            &slides(),
            &BundleOptions {
                site_url: Some(bad.into()),
                ..BundleOptions::default()
            },
        );
        assert!(matches!(result, Err(BundleError::SiteUrl(_))), "{bad}");
    }
}

#[test]
fn defaults_the_title_to_the_first_shown_slides_then_the_input_name() {
    let (deck, _) = compiled(&slides());
    let defaults = page_defaults(&deck);
    assert_eq!(defaults.title, "Welcome");
    assert_eq!(defaults.description.as_deref(), Some("Second · Fourth"));

    let mut hidden_first = slides();
    hidden_first.swap(0, 1);
    let (deck, _) = compiled(&hidden_first);
    assert_eq!(page_defaults(&deck).title, "Welcome");

    let mut untitled = slides();
    untitled.remove(0);
    untitled.rotate_left(4);
    let (deck, _) = compiled(&untitled);
    assert_eq!(deck.slides.iter().find(|s| !s.hidden).unwrap().title, "");
    assert_eq!(page_defaults(&deck).title, "fixture copy");
}

#[test]
fn cuts_the_default_description_at_the_last_whole_title() {
    let (mut deck, _) = compiled(&slides());
    let long = "x".repeat(70);
    for (i, slide) in deck.slides.iter_mut().enumerate() {
        slide.title = format!("{long}{i}");
    }
    let description = page_defaults(&deck).description.unwrap();
    // Slide 0 is the title, slide 1 is hidden: slides 2 and 3 fit (71 + 3 + 71 = 145), slide 4 does not.
    assert_eq!(description, format!("{long}2 · {long}3"));
    assert!(description.chars().count() <= 160);
}

#[test]
fn reads_as_text_what_the_slides_show() {
    let text = article_text(&page(&BundleOptions::default()));
    assert!(text.starts_with("Welcome Welcome"), "{text}");
    assert!(text.contains("Untitled body"), "{text}");
}

/// A numbered list's item, as the HTML backend prints it: its label as an `aria-hidden` text before its runs.
fn numbered(label: &str, text: &str) -> String {
    format!(
        "<li style=\"padding-left:36pt\"><span data-sd-bullet aria-hidden=\"true\" style=\"display:inline-block\">{label}</span>{text}</li>"
    )
}

#[test]
fn numbers_a_numbered_lists_items_with_the_labels_their_slide_shows() {
    let list = |items: &[(&str, &str)]| {
        let items: String = items
            .iter()
            .map(|(label, text)| numbered(label, text))
            .collect();
        format!("<ol style=\"margin:0;padding:0;list-style:none\">{items}</ol>")
    };
    let slides = vec![
        slide(
            "a",
            "Steps",
            vec![shape(
                "a.1",
                "",
                &list(&[("1.", "Alpha step"), ("2.", "Beta step")]),
            )],
        ),
        // The next build: the first two items repeat, so only the third prints, with its own number.
        slide(
            "b",
            "Steps",
            vec![shape(
                "b.1",
                "",
                &list(&[
                    ("1.", "Alpha step"),
                    ("2.", "Beta step"),
                    ("3.", "Gamma step"),
                ]),
            )],
        ),
        // Deeper items keep their own labels in the one list.
        slide(
            "c",
            "Nested",
            vec![shape(
                "c.1",
                "",
                &list(&[
                    ("1.", "First"),
                    ("a.", "Sub one"),
                    ("b.", "Sub two"),
                    ("2.", "Second"),
                ]),
            )],
        ),
    ];
    let page = page_with(&slides, &BundleOptions::default()).unwrap();
    let view = view(&page);
    assert!(
        section(view, "slide-2").contains("<ol>\n<li>3. Gamma step</li>\n</ol>\n"),
        "{view}"
    );
    assert!(
        section(view, "slide-3").contains(
            "<ol>\n<li>1. First</li>\n<li>a. Sub one</li>\n<li>b. Sub two</li>\n<li>2. Second</li>\n</ol>\n"
        ),
        "{view}"
    );
    assert!(!view.contains("start="), "{view}");
    // The view's own markers would number the items again.
    let head = &page[..page.find("</head>").unwrap()];
    assert!(head.contains("#sd-text ol{list-style:none}"), "{head}");
    // A bulleted list keeps the view's own markers, not the slide's glyphs.
    let first = section(view, "slide-1");
    assert!(first.contains("<li>1. Alpha step</li>"), "{first}");
    let welcome = section(view_of_default(), "slide-1");
    assert!(welcome.contains("<li>Point A</li>"), "{welcome}");
}

fn view_of_default() -> &'static str {
    let page: &'static str = Box::leak(page(&BundleOptions::default()).into_boxed_str());
    view(page)
}

#[test]
fn keeps_a_title_block_that_holds_a_link() {
    let slides = vec![slide(
        "a",
        "Wrap me",
        vec![shape(
            "a.1",
            " data-link=\"url:mailto:a@b.c\"",
            &p("Wrap me"),
        )],
    )];
    let page = page_with(&slides, &BundleOptions::default()).unwrap();
    let first = section(view(&page), "slide-1");
    assert!(
        first.contains("<p><a href=\"mailto:a@b.c\">Wrap me</a></p>"),
        "{first}"
    );
}

#[test]
fn leaves_out_the_paragraphs_that_make_up_the_title() {
    // A title from a text box of two paragraphs (the a11y title joins them with a space).
    let slides = vec![slide(
        "a",
        "Shared line Third only",
        vec![
            shape("a.1", "", &[p("Shared line"), p("Third only")].concat()),
            shape("a.2", "", &p("Body")),
        ],
    )];
    let page = page_with(&slides, &BundleOptions::default()).unwrap();
    let first = section(view(&page), "slide-1");
    assert!(
        first.starts_with(
            "<section id=\"slide-1\"><h2><a href=\"#slide-1\">Shared line Third only</a></h2>\n<p>Body</p>\n"
        ),
        "{first}"
    );
    assert_eq!(first.matches("Third only").count(), 1, "{first}");
}

#[test]
fn takes_a_blank_title_or_description_as_none() {
    let page = page_with(
        &slides(),
        &BundleOptions {
            title: Some(" \n\t ".into()),
            description: Some("   ".into()),
            ..BundleOptions::default()
        },
    )
    .unwrap();
    let head = &page[..page.find("</head>").unwrap()];
    assert!(head.contains("<title>Welcome</title>"), "{head}");
    assert!(
        head.contains("<meta property=\"og:title\" content=\"Welcome\">"),
        "{head}"
    );
    assert!(!head.contains("description"), "{head}");
    assert!(view(&page).contains("<h1>Welcome</h1>"));

    let page = page_with(
        &slides(),
        &BundleOptions {
            title: Some("  My talk ".into()),
            description: Some(" A talk\n".into()),
            ..BundleOptions::default()
        },
    )
    .unwrap();
    let head = &page[..page.find("</head>").unwrap()];
    assert!(head.contains("<title>My talk</title>"), "{head}");
    assert!(
        head.contains("<meta name=\"description\" content=\"A talk\">"),
        "{head}"
    );
}

#[test]
fn scrolls_to_the_view_in_a_browser_without_has() {
    // Such a browser drops every rule whose selector it cannot read: what is left must not hide the page's overflow.
    let chrome = |page: &str| {
        let start =
            page.find("<style id=\"sd-chrome\">").unwrap() + "<style id=\"sd-chrome\">".len();
        let end = page[start..].find("</style>").unwrap() + start;
        page[start..end].to_owned()
    };
    let with_view = chrome(&page(&BundleOptions::default()));
    let read: Vec<&str> = with_view
        .split_inclusive('}')
        .filter(|rule| !rule.contains(":has("))
        .collect();
    assert!(
        read.iter().all(|rule| !rule.contains("overflow:hidden")),
        "{read:?}"
    );
    // A browser with it hides the overflow of a body without the view (a host that replaced the body's content).
    assert!(
        with_view.contains("body:not(:has(>#sd-text)){overflow:hidden}"),
        "{with_view}"
    );
    // A page without a view has nothing to scroll in any browser.
    let without = chrome(&page(&BundleOptions {
        text_view: false,
        ..BundleOptions::default()
    }));
    assert!(without.contains("body{overflow:hidden}"), "{without}");
    // The view is the page without scripts.
    let page = page(&BundleOptions::default());
    let head = &page[..page.find("</head>").unwrap()];
    assert!(
        head.contains("<noscript><style>#sd-text{margin-top:0}</style></noscript>"),
        "{head}"
    );
}
