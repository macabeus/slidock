//! The bundle on a small deck written in the backend's shell: a slide on a layout with a picture and a video, and a
//! hidden slide that hides its layout, with media large enough to span several chunks.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "test code"
)]

use std::collections::{BTreeMap, BTreeSet};

use base64::Engine as _;
use sha2::Digest as _;
use slidock_bundle::{
    BundleError, BundleOptions, BundleReport, BundleSink, CHUNK_BYTES, CompiledDeck, DeckMedia,
    INDEX_PATH, LICENSES_PATH, Packaging, ServedFace, ShippedFace, bundle, embedded_player,
    lint_templates, player, slide_templates,
};
use slidock_fonts::bind::{FaceDescriptor, WeightRange};
use slidock_fonts::{FontDisplay, FontFaceRule, FontFormat, FontOrigin, FsType, LicenceEntry};
use slidock_format::dom::{Document, NodeId};
use slidock_format::manifest::{
    Generator, Manifest, ManifestLayout, ManifestSize, ManifestSlide, ManifestSource,
    ManifestTheme, SchemeColors, SizeUnit,
};
use slidock_format::paths::{MediaExtension, media_path};
use slidock_format::{RESET_STYLE, serialize_manifest};
use slidock_html::deck::{
    Deck, DeckFont, DeckFormat, DeckMedia as DeckMediaEntry, DeckSize, DeckSlide, DeckSource,
    DiagnosticSummary, FontSource,
};
use slidock_html::{Documents, MediaAction, MediaPlanRef};
use slidock_io::zip::{ZipArchive, ZipLimits};
use slidock_io::{BytesReader, RangeReader};
use slidock_ir::fonts::FaceStyle;
use slidock_ir::ids::Sha256;
use slidock_ir::media::Mime;

const BOX: &str = "position:relative;width:720pt;height:405pt;overflow:hidden";
const FULL: &str = "position:absolute;left:0pt;top:0pt;width:720pt;height:405pt";

/// The media bytes, by kind.
fn photo() -> Vec<u8> {
    b"\x89PNG\r\n\x1a\nphoto".to_vec()
}
fn clip() -> Vec<u8> {
    (0..CHUNK_BYTES * 5 / 2).map(|i| (i % 251) as u8).collect()
}
fn poster() -> Vec<u8> {
    b"\xff\xd8\xffposter".to_vec()
}

fn path_of(bytes: &[u8], ext: MediaExtension) -> String {
    media_path(&Sha256::of(bytes).to_hex(), ext).unwrap()
}

fn document_html(title: &str, main: &str, children: &[String]) -> String {
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

fn manifest(title: &str) -> Manifest {
    let slide = |id: &str, file: &str, title: &str, hidden| ManifestSlide {
        id: id.into(),
        file: file.into(),
        title: title.into(),
        hidden,
        layout: Some("ir-l1".into()),
        notes: None,
        transition: None,
        failed: None,
    };
    Manifest {
        title: title.into(),
        size: ManifestSize {
            w: 720.0,
            h: 405.0,
            unit: SizeUnit::Pt,
        },
        slides: vec![
            slide("ir-s1", "slides/ir-s1.html", "One", false),
            slide("ir-S2", "slides/ir-s2-da7f247d.html", "", true),
        ],
        layouts: vec![ManifestLayout {
            id: "ir-l1".into(),
            file: "layouts/ir-l1.html".into(),
            name: Some("Title".into()),
            theme: Some("ir-t1".into()),
            failed: None,
        }],
        themes: BTreeMap::from([(
            "ir-t1".to_owned(),
            ManifestTheme {
                name: Some("Plain".into()),
                colors: SchemeColors::uniform("#888888"),
            },
        )]),
        source: Some(ManifestSource {
            format: "ir".into(),
            name: "sample.presentation-ir.json".into(),
            sha256: "a".repeat(64),
            bytes: 10,
        }),
        generator: Some(Generator {
            name: "slidock".into(),
            version: "0.1.0".into(),
        }),
    }
}

fn documents(title: &str) -> Documents {
    let photo = path_of(&photo(), MediaExtension::Png);
    let clip = path_of(&clip(), MediaExtension::Mp4);
    let poster = path_of(&poster(), MediaExtension::Jpg);
    let layout = document_html(
        "Title",
        "data-layout-root=\"ir-l1\"",
        &[
            format!("<div data-sd-bg style=\"{FULL};background:#202020\"></div>"),
            "<div data-el=\"ir-l1.band\" data-kind=\"shape\" style=\"position:absolute;left:0pt;top:380pt;width:720pt;height:25pt;background:#4472c4\"></div>".into(),
            "<div data-el=\"ir-l1.num\" data-kind=\"shape\" style=\"position:absolute;left:660pt;top:380pt;width:50pt;height:20pt\"><div data-sd-text style=\"position:absolute;left:0pt;top:0pt;width:100%;height:100%\"><p style=\"line-height:1.2\"><span data-field=\"slideNumber\" style=\"font-size:12pt\">‹#›</span></p></div></div>".into(),
        ],
    );
    let one = document_html(
        "One",
        "data-slide=\"ir-s1\" data-layout=\"ir-l1\"",
        &[
            "<div data-el=\"ir-s1.title\" data-kind=\"shape\" style=\"position:absolute;left:40pt;top:40pt;width:640pt;height:80pt\"><div data-sd-text style=\"position:absolute;left:0pt;top:0pt;width:100%;height:100%;font-family:&quot;Liberation Sans&quot;,sans-serif\"><p style=\"line-height:1.2\"><span style=\"font-size:40pt;color:var(--sd-color-tx1,#ffffff)\">One &lt;/script&gt;</span></p></div></div>".into(),
            format!("<div data-el=\"ir-s1.photo\" data-kind=\"picture\" style=\"position:absolute;left:40pt;top:140pt;width:200pt;height:150pt\"><img src=\"../{photo}\" alt=\"\" style=\"position:absolute;left:0%;top:0%;width:100%;height:100%;display:block\"></div>"),
            format!("<div data-el=\"ir-s1.clip\" data-kind=\"media\" style=\"position:absolute;left:300pt;top:140pt;width:320pt;height:180pt\"><video src=\"../{clip}\" poster=\"../{poster}\" preload=\"none\" playsinline muted style=\"position:absolute;left:0pt;top:0pt;width:100%;height:100%;object-fit:fill\"></video></div>"),
        ],
    );
    let two = document_html(
        "",
        "data-slide=\"ir-S2\" data-layout=\"ir-l1\" data-show-layout=\"false\"",
        &[
            format!("<div data-sd-bg style=\"{FULL};background-image:url(&quot;../{photo}&quot;);background-size:100% 100%\"></div>"),
            "<div data-el=\"ir-S2.link\" data-kind=\"shape\" style=\"position:absolute;left:10pt;top:10pt;width:100pt;height:20pt\"><div data-sd-text style=\"position:absolute;left:0pt;top:0pt;width:100%;height:100%\"><p><a href=\"https://example.com/\" style=\"color:inherit\"><span>Site</span></a></p></div></div>".into(),
        ],
    );
    let plan = |bytes: &[u8], mime: &str| MediaPlanRef {
        sha256: Sha256::of(bytes),
        action: MediaAction::Copy,
        mime: Mime::new(mime).unwrap(),
        pixels: None,
    };
    Documents {
        text: BTreeMap::from([
            (
                "presentation.json".to_owned(),
                serialize_manifest(&manifest(title)).unwrap(),
            ),
            ("layouts/ir-l1.html".to_owned(), layout),
            ("slides/ir-s1.html".to_owned(), one),
            ("slides/ir-s2-da7f247d.html".to_owned(), two),
        ]),
        media: BTreeMap::from([
            (photo, plan(&self::photo(), "image/png")),
            (clip, plan(&self::clip(), "video/mp4")),
            (poster, plan(&self::poster(), "image/jpeg")),
        ]),
        faces: BTreeMap::new(),
    }
}

fn deck_font(family: &str, source: FontSource) -> DeckFont {
    DeckFont {
        family: family.into(),
        weight: 400,
        style: FaceStyle::Normal,
        source,
        licence: (source != FontSource::System).then(|| format!("Copyright {family} authors")),
    }
}

fn deck(title: &str, documents: &Documents) -> Deck {
    let slide = |id: &str, hidden: bool, number, title: &str| DeckSlide {
        id: id.into(),
        layout: Some("ir-l1".into()),
        hidden,
        number,
        title: title.into(),
        notes: Vec::new(),
        failed: None,
        timeline: Vec::new(),
        transition: None,
        match_keys: BTreeMap::new(),
        fonts: vec![0],
    };
    Deck {
        format: DeckFormat::V1,
        player: None,
        id: String::new(),
        title: title.into(),
        lang: Some("en".into()),
        size: DeckSize { w: 720.0, h: 405.0 },
        source: DeckSource {
            format: "ir".into(),
            name: "sample.presentation-ir.json".into(),
            sha256: "a".repeat(64),
            bytes: 10,
        },
        slides: vec![
            slide("ir-s1", false, Some(1), "One"),
            slide("ir-S2", true, None, ""),
        ],
        media: documents
            .media
            .iter()
            .map(|(path, plan)| DeckMediaEntry {
                path: path.clone(),
                mime: plan.mime.as_str().to_owned(),
                sha256: plan.sha256.to_hex(),
                bytes: None,
            })
            .collect(),
        fonts: vec![
            deck_font("Inter", FontSource::Google),
            deck_font("Liberation Sans", FontSource::System),
        ],
        diagnostics: DiagnosticSummary::default(),
    }
}

/// The media of the fixture, written in 64 KiB pieces.
struct Media(BTreeMap<String, Vec<u8>>);

impl Media {
    fn new() -> Media {
        Media(BTreeMap::from([
            (path_of(&photo(), MediaExtension::Png), photo()),
            (path_of(&clip(), MediaExtension::Mp4), clip()),
            (path_of(&poster(), MediaExtension::Jpg), poster()),
        ]))
    }
}

impl DeckMedia for Media {
    fn write_media(&self, path: &str, out: &mut dyn std::io::Write) -> Result<(), BundleError> {
        let bytes = self.0.get(path).ok_or_else(|| BundleError::Media {
            path: path.to_owned(),
            message: "unknown".into(),
        })?;
        for piece in bytes.chunks(64 * 1024) {
            out.write_all(piece).map_err(|error| BundleError::Media {
                path: path.to_owned(),
                message: error.to_string(),
            })?;
        }
        Ok(())
    }
}

/// Every chunk a bundle hands over, with its path.
#[derive(Default)]
struct Recorder(Vec<(String, Vec<u8>)>);

impl BundleSink for Recorder {
    fn write(&mut self, path: &str, bytes: &[u8]) -> Result<(), BundleError> {
        self.0.push((path.to_owned(), bytes.to_vec()));
        Ok(())
    }
}

impl Recorder {
    fn bytes(&self) -> Vec<u8> {
        self.0.iter().flat_map(|(_, bytes)| bytes.clone()).collect()
    }
}

const TITLE: &str = "Sample";

fn run_with(
    title: &str,
    options: &BundleOptions,
    fonts: Vec<ShippedFace>,
) -> (BundleReport, Recorder) {
    let documents = documents(title);
    let media = Media::new();
    let compiled = CompiledDeck {
        deck: deck(title, &documents),
        documents,
        media: &media,
        fonts,
    };
    let mut recorder = Recorder::default();
    let report = bundle(&compiled, options, &mut recorder).unwrap();
    (report, recorder)
}

fn run(options: &BundleOptions) -> (BundleReport, Recorder) {
    run_with(TITLE, options, Vec::new())
}

fn files_options() -> BundleOptions {
    BundleOptions {
        packaging: Packaging::Files,
        ..BundleOptions::default()
    }
}

/// The files of a `Packaging::Files` bundle, by path.
fn files_of(recorder: &Recorder) -> BTreeMap<String, Vec<u8>> {
    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for (path, bytes) in &recorder.0 {
        files.entry(path.clone()).or_default().extend(bytes);
    }
    files
}

fn index_of(recorder: &Recorder) -> String {
    String::from_utf8(files_of(recorder)[INDEX_PATH].clone()).unwrap()
}

/// The `<script>` elements anywhere in a page, template content included, with their text.
fn scripts(page: &Document) -> Vec<(NodeId, String)> {
    page.walk(NodeId::DOCUMENT)
        .into_iter()
        .filter(|e| page.tag_name(*e) == "script")
        .map(|e| (e, page.text_content(e)))
        .collect()
}

fn csp_of(page: &Document) -> String {
    let meta = page
        .walk(NodeId::DOCUMENT)
        .into_iter()
        .find(|e| page.attribute(*e, "http-equiv") == Some("Content-Security-Policy"))
        .unwrap();
    page.attribute(meta, "content").unwrap().to_owned()
}

fn deck_json_of(page: &Document) -> Deck {
    let (_, json) = scripts(page)
        .into_iter()
        .find(|(e, _)| page.attribute(*e, "id") == Some("sd-deck"))
        .unwrap();
    Deck::from_json(&json).unwrap()
}

/// The slide templates of a page: their ids and content.
fn templates_of(page: &Document) -> Vec<(String, NodeId)> {
    page.walk(NodeId::DOCUMENT)
        .into_iter()
        .filter(|e| page.tag_name(*e) == "template")
        .map(|e| {
            (
                page.attribute(e, "data-sd-slide").unwrap().to_owned(),
                page.template_content(e).unwrap(),
            )
        })
        .collect()
}

#[test]
fn writes_the_same_bytes_twice_as_a_zip_and_as_files() {
    let (first, a) = run(&BundleOptions::default());
    let (second, b) = run(&BundleOptions::default());
    assert_eq!(a.bytes(), b.bytes());
    assert_eq!(first, second);
    let (_, files_a) = run(&files_options());
    let (_, files_b) = run(&files_options());
    assert_eq!(files_a.bytes(), files_b.bytes());
}

#[test]
fn writes_index_then_sorted_media_then_licenses_into_a_zip_the_reader_opens() {
    let (report, recorder) = run(&BundleOptions::default());
    let bytes = recorder.bytes();
    assert_eq!(report.bytes, bytes.len() as u64);
    assert!(!report.single_file);
    let archive = ZipArchive::open(BytesReader::new(bytes), ZipLimits::default()).unwrap();
    let names: Vec<&str> = archive.entries().iter().map(|e| e.name.as_str()).collect();
    let media = Media::new();
    let mut expected = vec![INDEX_PATH];
    expected.extend(media.0.keys().map(String::as_str));
    expected.push(LICENSES_PATH);
    assert_eq!(names, expected);
    let reported: Vec<&str> = report.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(reported, expected);

    let (_, files) = run(&files_options());
    let files = files_of(&files);
    for (entry, file) in archive.entries().iter().zip(&report.files) {
        let reader = archive.open_entry(entry).unwrap();
        let content = reader.read_at(0, reader.len() as usize).unwrap();
        assert_eq!(
            content.as_ref(),
            files[&entry.name].as_slice(),
            "{}",
            entry.name
        );
        assert_eq!(file.bytes, content.len() as u64, "{}", entry.name);
        if let Some(source) = media.0.get(&entry.name) {
            assert_eq!(entry.method, 0, "{}: stored", entry.name);
            assert_eq!(content.as_ref(), source.as_slice());
        } else {
            assert_eq!(entry.method, 8, "{}: deflated", entry.name);
        }
    }
}

#[test]
fn passes_unzip_t() {
    let (_, recorder) = run(&BundleOptions::default());
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let path = dir.join("slidock-bundle-unzip-t.zip");
    std::fs::write(&path, recorder.bytes()).unwrap();
    let output = match std::process::Command::new("unzip")
        .arg("-t")
        .arg(&path)
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("skipped: `unzip` is not installed, so the zip is not tested with it");
            return;
        }
        Err(error) => panic!("unzip: {error}"),
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}");
    assert!(stdout.contains("No errors detected"), "{stdout}");
    assert_eq!(stdout.matches("OK").count(), 5, "{stdout}");
}

#[test]
fn hands_the_sink_chunks_of_at_most_a_mebibyte_each_labelled_with_its_entry() {
    let (_, recorder) = run(&BundleOptions::default());
    assert!(
        recorder
            .0
            .iter()
            .all(|(_, bytes)| bytes.len() <= CHUNK_BYTES)
    );
    let clip = path_of(&clip(), MediaExtension::Mp4);
    let clip_chunks = recorder.0.iter().filter(|(path, _)| *path == clip).count();
    assert_eq!(clip_chunks, 3, "2.5 MiB plus its header and descriptor");
    let mut order: Vec<&str> = recorder.0.iter().map(|(path, _)| path.as_str()).collect();
    order.dedup();
    let media = Media::new();
    let mut expected = vec![INDEX_PATH];
    expected.extend(media.0.keys().map(String::as_str));
    expected.extend([LICENSES_PATH, ""]);
    assert_eq!(order, expected);
}

#[test]
fn runs_only_its_own_scripts_by_hash() {
    for (presenter, expected) in [
        (
            true,
            vec![
                player::SCRIPT,
                player::PRESENTER,
                player::BOOT_WITH_PRESENTER,
            ],
        ),
        (false, vec![player::SCRIPT, player::BOOT]),
    ] {
        let (_, recorder) = run(&BundleOptions {
            presenter,
            ..files_options()
        });
        runs_only_these_scripts_by_hash(&index_of(&recorder), &expected);
    }
}

fn runs_only_these_scripts_by_hash(index: &str, expected: &[&str]) {
    let page = Document::parse(index);
    let scripts = scripts(&page);
    assert_eq!(scripts.len(), expected.len() + 1);
    let body = page.body().unwrap();
    assert_eq!(page.parent(scripts[0].0), Some(body));
    assert_eq!(
        page.attribute(scripts[0].0, "type"),
        Some("application/json")
    );
    for ((element, text), expected) in scripts[1..].iter().zip(expected) {
        assert_eq!(page.parent(*element), Some(body));
        assert_eq!(text, expected);
    }
    let hashes: Vec<String> = scripts[1..]
        .iter()
        .map(|(_, text)| {
            let digest = sha2::Sha256::digest(text.as_bytes());
            format!(
                "'sha256-{}'",
                base64::engine::general_purpose::STANDARD.encode(digest)
            )
        })
        .collect();
    let csp = csp_of(&page);
    let script_src = csp
        .split("; ")
        .find_map(|directive| directive.strip_prefix("script-src "))
        .unwrap();
    assert_eq!(script_src, hashes.join(" "));
    assert!(csp.starts_with("default-src 'none'; "));
    for directive in [
        "style-src 'unsafe-inline'",
        "font-src data: blob:",
        "connect-src 'none'",
        "object-src 'none'",
        "base-uri 'none'",
    ] {
        assert!(csp.split("; ").any(|d| d == directive), "{directive}");
    }
}

#[test]
fn records_the_embedded_player_in_the_deck_json() {
    let (_, recorder) = run(&BundleOptions {
        presenter: false,
        ..files_options()
    });
    let without = deck_json_of(&Document::parse(&index_of(&recorder)));
    assert_eq!(without.player, Some(embedded_player(false).unwrap()));
    assert_eq!(without.player.as_ref().unwrap().presenter, None);
    let (_, recorder) = run(&files_options());
    let page = Document::parse(&index_of(&recorder));
    let deck = deck_json_of(&page);
    let stamp = embedded_player(true).unwrap();
    assert_eq!(deck.player.as_ref(), Some(&stamp));
    assert_eq!(stamp.sha256, slidock_canon::sha256_hex(player::SCRIPT));
    assert_eq!(
        stamp.presenter,
        Some(slidock_canon::sha256_hex(player::PRESENTER))
    );
    assert_eq!(without.id, deck.id, "the presenter view is not content");
    let version = include_str!("../player/version.txt");
    assert_eq!(format!("{}\n", stamp.version), version);
    assert_eq!(player::SCRIPT, include_str!("../player/player.iife.js"));
    assert_eq!(
        player::PRESENTER,
        include_str!("../player/presenter.iife.js")
    );
    assert!(player::SCRIPT.contains("Slidock"));
    assert!(player::PRESENTER.contains("SlidockPresenter"));
    // The content id is the deck's, whatever player it ships with.
    let documents = documents(TITLE);
    let mut expected = deck.clone();
    expected.fonts[0].source = FontSource::System;
    expected.fonts[0].licence = None;
    assert_eq!(
        deck.id,
        slidock_html::deck::deck_id(&expected, &documents).unwrap()
    );
}

#[test]
fn holds_each_slide_composed_with_its_layout_in_an_inert_template() {
    let (_, recorder) = run(&files_options());
    let page = Document::parse(&index_of(&recorder));
    let templates = templates_of(&page);
    let ids: Vec<&str> = templates.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, ["ir-s1", "ir-S2"]);
    for (id, content) in &templates {
        let roots = page.child_elements(*content);
        assert_eq!(roots.len(), 1);
        assert_eq!(page.tag_name(roots[0]), "main");
        assert_eq!(page.attribute(roots[0], "data-slide"), Some(id.as_str()));
        assert!(
            page.walk(*content)
                .iter()
                .all(|e| page.tag_name(*e) != "script"),
            "{id}"
        );
    }
    let element_ids = |content: NodeId| -> Vec<String> {
        page.walk(content)
            .into_iter()
            .filter_map(|e| page.attribute(e, "data-el").map(str::to_owned))
            .collect()
    };
    assert_eq!(
        element_ids(templates[0].1),
        [
            "ir-l1.band",
            "ir-l1.num",
            "ir-s1.title",
            "ir-s1.photo",
            "ir-s1.clip"
        ]
    );
    assert_eq!(element_ids(templates[1].1), ["ir-S2.link"]);

    let documents = documents(TITLE);
    let media = Media::new();
    let compiled = CompiledDeck {
        deck: deck(TITLE, &documents),
        documents: documents.clone(),
        media: &media,
        fonts: Vec::new(),
    };
    let composed = slide_templates(&compiled).unwrap();
    let index = index_of(&recorder);
    for template in &composed {
        assert!(index.contains(&format!(
            "<template data-sd-slide=\"{}\">{}</template>\n",
            template.id, template.html
        )));
        for path in media.0.keys() {
            assert!(!template.html.contains(&format!("../{path}")));
        }
    }
    let photo = path_of(&photo(), MediaExtension::Png);
    assert!(composed[0].html.contains(&format!("<img src=\"{photo}\"")));
    assert!(
        composed[1]
            .html
            .contains(&format!("url(&quot;{photo}&quot;)"))
    );
    assert!(
        composed[0]
            .html
            .contains("style=\"font-size:12pt\">1</span>")
    );
    let problems = lint_templates(&documents, &composed).unwrap();
    assert_eq!(problems, []);
}

#[test]
fn the_template_lint_sees_a_script_or_a_foreign_media_reference() {
    let documents = documents(TITLE);
    let media = Media::new();
    let compiled = CompiledDeck {
        deck: deck(TITLE, &documents),
        documents: documents.clone(),
        media: &media,
        fonts: Vec::new(),
    };
    let mut templates = slide_templates(&compiled).unwrap();
    templates[1].html = templates[1]
        .html
        .replace("<span>Site</span>", "<script>alert(1)</script>");
    let photo = path_of(&photo(), MediaExtension::Png);
    templates[0].html = templates[0].html.replace(
        &format!("src=\"{photo}\""),
        "src=\"https://example.com/x.png\"",
    );
    let problems = lint_templates(&documents, &templates).unwrap();
    let codes: BTreeSet<(&str, &str)> = problems
        .iter()
        .map(|p| (p.file.as_str(), p.code.name()))
        .collect();
    assert!(
        codes.contains(&("slides/ir-s2-da7f247d.html", "script")),
        "{codes:?}"
    );
    assert!(
        codes.iter().any(|(file, _)| *file == "slides/ir-s1.html"),
        "{codes:?}"
    );
}

#[test]
fn escapes_what_the_page_interpolates() {
    let title = "</script><!--<script>alert(1)</script>\u{2028}\u{2029}&<b>";
    let options = BundleOptions {
        title: Some(title.to_owned()),
        ..files_options()
    };
    let (_, recorder) = run_with(title, &options, Vec::new());
    let index = index_of(&recorder);
    let page = Document::parse(&index);
    assert_eq!(
        scripts(&page).len(),
        4,
        "the page's own: deck, player, presenter, boot"
    );
    assert_eq!(deck_json_of(&page).title, title);
    let title_element = page
        .walk(NodeId::DOCUMENT)
        .into_iter()
        .find(|e| page.tag_name(*e) == "title")
        .unwrap();
    assert_eq!(page.text_content(title_element), title);
    let json = index
        .split("<script type=\"application/json\" id=\"sd-deck\">")
        .nth(1)
        .unwrap()
        .split("</script>")
        .next()
        .unwrap();
    assert!(!json.contains('<'));
    assert!(!json.contains('\u{2028}') && !json.contains('\u{2029}'));
    assert!(json.contains("\\u2028\\u2029"));
}

#[test]
fn declares_the_decks_language_on_the_page_when_it_has_one() {
    let documents = documents(TITLE);
    let lang_of = |lang: Option<&str>| {
        let mut data = deck(TITLE, &documents);
        data.lang = lang.map(str::to_owned);
        let html = slidock_bundle::page::index_html(&slidock_bundle::page::Page {
            deck: &data,
            font_rules: &[],
            templates: &[],
            description: None,
            site_url: None,
            text_view: None,
            presenter: true,
        })
        .unwrap();
        let page = Document::parse(&html);
        let root = page
            .walk(NodeId::DOCUMENT)
            .into_iter()
            .find(|e| page.tag_name(*e) == "html")
            .unwrap();
        (page.attribute(root, "lang").map(str::to_owned), html)
    };
    let (lang, html) = lang_of(Some("pt-BR"));
    assert_eq!(lang.as_deref(), Some("pt-BR"));
    assert!(html.contains("\n<html lang=\"pt-BR\">\n"));
    let hostile = "en\"><script>alert(1)</script>";
    let (lang, _) = lang_of(Some(hostile));
    assert_eq!(lang.as_deref(), Some(hostile));
    // No language is better than a guessed one.
    let (lang, html) = lang_of(None);
    assert_eq!(lang, None);
    assert!(html.contains("\n<html>\n"));
}

#[test]
fn inlines_the_media_as_data_uris_up_to_the_single_file_threshold() {
    // The photo shows on both slides, so the page holds it twice.
    let total = 2 * photo().len() + clip().len() + poster().len();
    let single = |threshold: usize| BundleOptions {
        single_file: true,
        single_file_threshold: threshold as u64,
        ..BundleOptions::default()
    };
    let (report, recorder) = run(&single(total));
    assert!(report.single_file);
    let archive =
        ZipArchive::open(BytesReader::new(recorder.bytes()), ZipLimits::default()).unwrap();
    let names: Vec<&str> = archive.entries().iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, [INDEX_PATH, LICENSES_PATH]);
    let index =
        String::from_utf8(archive.read(&archive.entries()[0]).unwrap().into_owned()).unwrap();
    let encode = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
    assert!(index.contains(&format!(
        "<img src=\"data:image/png;base64,{}\"",
        encode(&photo())
    )));
    assert!(index.contains(&format!(
        "url(&quot;data:image/png;base64,{}&quot;)",
        encode(&photo())
    )));
    assert!(index.contains(&format!(
        "poster=\"data:image/jpeg;base64,{}\"",
        encode(&poster())
    )));
    assert!(index.contains(&format!(
        "src=\"data:video/mp4;base64,{}\"",
        encode(&clip())
    )));
    assert!(!index.contains("=\"media/") && !index.contains("url(&quot;media/"));
    runs_only_these_scripts_by_hash(
        &index,
        &[
            player::SCRIPT,
            player::PRESENTER,
            player::BOOT_WITH_PRESENTER,
        ],
    );

    let (report, recorder) = run(&single(total - 1));
    assert!(!report.single_file);
    assert_eq!(report.files.len(), 5);
    let (_, plain) = run(&BundleOptions::default());
    assert_eq!(recorder.bytes(), plain.bytes());
}

/// A shipped file of Inter at `weight` from `origin` that draws the deck's Inter 400.
fn inter_file(weight: WeightRange, origin: FontOrigin) -> ShippedFace {
    ShippedFace {
        rules: vec![FontFaceRule {
            family: "Inter".into(),
            descriptor: FaceDescriptor {
                weight,
                style: FaceStyle::Normal,
            },
            unicode_range: None,
            format: FontFormat::Woff2,
            display: FontDisplay::Block,
            bytes: b"wOF2inter".to_vec(),
        }],
        licence: LicenceEntry {
            family: "Inter".into(),
            weight: if weight.min == weight.max {
                weight.min.to_string()
            } else {
                format!("{} {}", weight.min, weight.max)
            },
            style: "normal",
            origin,
            fs_type: FsType(0),
            copyright: Some("Copyright Inter authors".into()),
            licence: None,
            licence_url: None,
            licence_id: None,
        },
        serves: vec![ServedFace {
            family: "Inter".into(),
            weight: 400,
            style: FaceStyle::Normal,
        }],
    }
}

#[test]
fn ships_the_faces_given_with_their_own_descriptors() {
    let inter = inter_file(WeightRange::between(100, 900), FontOrigin::Google);
    let (report, recorder) = run_with(TITLE, &files_options(), vec![inter]);
    assert!(report.fonts_left_out.is_empty());
    assert_eq!(report.fonts_with_notice, None);
    let files = files_of(&recorder);
    let index = String::from_utf8(files[INDEX_PATH].clone()).unwrap();
    // The rule states the file's weight range, not the weight the deck draws.
    let rule = "@font-face{font-family:\"Inter\";src:url(data:font/woff2;base64,d09GMmludGVy) \
                format(\"woff2\");font-weight:100 900;font-style:normal;font-display:block}";
    assert!(index.contains(&format!("<style id=\"sd-fonts\">{rule}</style>")));
    let deck = deck_json_of(&Document::parse(&index));
    assert_eq!(deck.fonts[0], deck_font("Inter", FontSource::Google));
    let licenses = String::from_utf8(files[LICENSES_PATH].clone()).unwrap();
    assert!(licenses.contains("Inter 100 900 normal (Google Fonts)\nCopyright Inter authors\n"));
    assert!(licenses.contains("MIT License"));

    // The toggle leaves out only a face from this computer or the user.
    let leave_out = BundleOptions {
        leave_out: BTreeSet::from(["Inter".to_owned()]),
        ..files_options()
    };
    let (report, recorder) = run_with(
        TITLE,
        &leave_out,
        vec![inter_file(WeightRange::point(400), FontOrigin::Google)],
    );
    assert!(report.fonts_left_out.is_empty());
    assert!(index_of(&recorder).contains("font-weight:400;"));
}

#[test]
fn leaves_out_a_family_from_this_computer_the_host_toggles() {
    let local = inter_file(WeightRange::point(400), FontOrigin::Local);
    let (report, recorder) = run_with(TITLE, &files_options(), vec![local.clone()]);
    assert_eq!(report.fonts_with_notice, Some(vec!["Inter".to_owned()]));
    let deck = deck_json_of(&Document::parse(&index_of(&recorder)));
    assert_eq!(deck.fonts[0].source, FontSource::Local);
    assert_eq!(
        deck.fonts[0].licence.as_deref(),
        Some("Copyright Inter authors")
    );

    let leave_out = BundleOptions {
        leave_out: BTreeSet::from(["inter".to_owned()]),
        ..files_options()
    };
    let (report, recorder) = run_with(TITLE, &leave_out, vec![local]);
    assert_eq!(report.fonts_left_out, ["Inter"]);
    assert_eq!(report.fonts_with_notice, None);
    let files = files_of(&recorder);
    let index = String::from_utf8(files[INDEX_PATH].clone()).unwrap();
    assert!(index.contains("<style id=\"sd-fonts\"></style>"));
    let deck = deck_json_of(&Document::parse(&index));
    assert_eq!(deck.fonts[0].source, FontSource::System);
    assert_eq!(deck.fonts[0].licence, None);
    let licenses = String::from_utf8(files[LICENSES_PATH].clone()).unwrap();
    assert!(licenses.contains("No font files ship with this presentation."));
}

#[test]
fn leaves_out_a_restricted_face_unless_the_host_includes_its_family() {
    let restricted = |origin| {
        let mut file = inter_file(WeightRange::point(400), origin);
        file.licence.fs_type = FsType(0x0002);
        file
    };
    let local = restricted(FontOrigin::Local);
    let (report, recorder) = run_with(TITLE, &files_options(), vec![local.clone()]);
    assert_eq!(report.fonts_left_out, ["Inter"]);
    assert_eq!(report.fonts_with_notice, None);
    let index = index_of(&recorder);
    assert!(index.contains("<style id=\"sd-fonts\"></style>"));
    assert_eq!(
        deck_json_of(&Document::parse(&index)).fonts[0].source,
        FontSource::System
    );

    // Included, it ships with the notice; left out too, it stays out.
    let include = BundleOptions {
        include: BTreeSet::from(["inter".to_owned()]),
        ..files_options()
    };
    let (report, recorder) = run_with(TITLE, &include, vec![local.clone()]);
    assert!(report.fonts_left_out.is_empty());
    assert_eq!(report.fonts_with_notice, Some(vec!["Inter".to_owned()]));
    let deck = deck_json_of(&Document::parse(&index_of(&recorder)));
    assert_eq!(deck.fonts[0].source, FontSource::Local);
    let both = BundleOptions {
        leave_out: BTreeSet::from(["Inter".to_owned()]),
        ..include.clone()
    };
    let (report, _) = run_with(TITLE, &both, vec![local]);
    assert_eq!(report.fonts_left_out, ["Inter"]);

    // A restricted face from another source offers no toggle.
    let (report, _) = run_with(TITLE, &include, vec![restricted(FontOrigin::Embedded)]);
    assert_eq!(report.fonts_left_out, ["Inter"]);
}

/// Refuses its `n`th write.
struct RefuseAt(usize);

impl BundleSink for RefuseAt {
    fn write(&mut self, _: &str, _: &[u8]) -> Result<(), BundleError> {
        if self.0 == 0 {
            return Err(BundleError::Cancelled);
        }
        self.0 -= 1;
        Ok(())
    }
}

#[test]
fn stops_with_the_sinks_refusal_even_inside_a_medium() {
    let documents = documents(TITLE);
    let media = Media::new();
    let compiled = CompiledDeck {
        deck: deck(TITLE, &documents),
        documents,
        media: &media,
        fonts: Vec::new(),
    };
    let (_, recorder) = run(&BundleOptions::default());
    for refuse_at in 0..recorder.0.len() {
        assert_eq!(
            bundle(
                &compiled,
                &BundleOptions::default(),
                &mut RefuseAt(refuse_at)
            ),
            Err(BundleError::Cancelled),
            "{refuse_at}"
        );
    }
    assert!(
        bundle(
            &compiled,
            &BundleOptions::default(),
            &mut RefuseAt(recorder.0.len())
        )
        .is_ok()
    );
}

#[test]
fn reports_a_medium_the_host_cannot_give() {
    let documents = documents(TITLE);
    let mut media = Media::new();
    let photo = path_of(&photo(), MediaExtension::Png);
    media.0.remove(&photo);
    let compiled = CompiledDeck {
        deck: deck(TITLE, &documents),
        documents,
        media: &media,
        fonts: Vec::new(),
    };
    for options in [
        BundleOptions::default(),
        BundleOptions {
            single_file: true,
            ..BundleOptions::default()
        },
    ] {
        assert_eq!(
            bundle(&compiled, &options, &mut Recorder::default()),
            Err(BundleError::Media {
                path: photo.clone(),
                message: "unknown".into()
            })
        );
    }
}
