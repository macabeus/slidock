//! The lint, with the `sd` markers.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod common;

use common::{
    LAYOUT, PHOTO, SLIDE, SLIDE_TWO, document_html, sample_files, sample_manifest, slide_one_html,
    text,
};
use slidock_format::{
    PresentationFiles, Problem, ProblemCode, ProblemSeverity, lint_presentation, serialize_manifest,
};

fn lint_with(change: impl FnOnce(&mut PresentationFiles)) -> Vec<Problem> {
    let mut files = sample_files();
    change(&mut files);
    lint_presentation(&files)
}

/// The problems a change brings, as (code, file).
fn found(change: impl FnOnce(&mut PresentationFiles)) -> Vec<(&'static str, String)> {
    lint_with(change)
        .into_iter()
        .map(|problem| (problem.code.name(), problem.file))
        .collect()
}

fn pairs(expected: &[(&'static str, &str)]) -> Vec<(&'static str, String)> {
    expected
        .iter()
        .map(|(code, file)| (*code, (*file).to_owned()))
        .collect()
}

/// Replaces text in a document of the set.
fn replace_in(path: &'static str, from: String, to: String) -> impl FnOnce(&mut PresentationFiles) {
    move |files| {
        let html = text(files, path);
        assert!(html.contains(&from), "{path} has no {from}");
        let changed = html.replacen(&from, &to, 1);
        files.insert(path.to_owned(), Some(changed));
    }
}

/// Replaces text in the first slide's document.
fn in_slide(from: &str, to: &str) -> impl FnOnce(&mut PresentationFiles) {
    replace_in(SLIDE, from.to_owned(), to.to_owned())
}

const TITLE_DIV: &str = "<div data-el=\"ir-s1.title\" data-kind=\"shape\" style=\"position:absolute;left:40pt;top:40pt;";

#[test]
fn finds_nothing_in_a_clean_file_set() {
    assert_eq!(lint_presentation(&sample_files()), []);
}

#[test]
fn reports_a_missing_or_unreadable_manifest_and_still_checks_the_documents() {
    assert_eq!(
        found(|files| {
            files.remove("presentation.json");
        }),
        pairs(&[("manifest-missing", "presentation.json")])
    );
    assert_eq!(
        found(|files| {
            files.insert("presentation.json".into(), Some("{".into()));
        }),
        pairs(&[("manifest-invalid", "presentation.json")])
    );
    let unsafe_ = found(|files| {
        files.remove("presentation.json");
        in_slide("</main>", "<script>x()</script></main>")(files);
    });
    assert!(
        unsafe_.contains(&("script", SLIDE.to_owned())),
        "{unsafe_:?}"
    );
}

#[test]
fn reports_duplicate_ids_and_too_many_slides_in_the_manifest() {
    let mut duplicated = sample_manifest();
    duplicated.layouts[0].id = "ir-s1".into();
    duplicated.layouts[0].file = "layouts/ir-s1.html".into();
    for slide in &mut duplicated.slides {
        slide.layout = None;
    }
    let problems = found(|files| {
        files.insert(
            "presentation.json".into(),
            Some(serialize_manifest(&duplicated).unwrap()),
        );
        let layout = text(files, LAYOUT).replacen("ir-l1\"", "ir-s1\"", 1);
        files.insert("layouts/ir-s1.html".into(), Some(layout));
        files.remove(LAYOUT);
        for path in [SLIDE, SLIDE_TWO] {
            let slide = text(files, path).replacen(" data-layout=\"ir-l1\"", "", 1);
            files.insert(path.into(), Some(slide));
        }
    });
    assert!(
        problems.contains(&("duplicate-id", "presentation.json".to_owned())),
        "{problems:?}"
    );

    let mut many = sample_manifest();
    let template = many.slides[0].clone();
    many.slides = (0..301)
        .map(|n| slidock_format::manifest::ManifestSlide {
            id: format!("ir-n{n}"),
            file: format!("slides/ir-n{n}.html"),
            title: String::new(),
            hidden: false,
            layout: None,
            ..template.clone()
        })
        .collect();
    let problems = lint_with(|files| {
        files.insert(
            "presentation.json".into(),
            Some(serialize_manifest(&many).unwrap()),
        );
    });
    let message = problems
        .iter()
        .find(|p| p.code == ProblemCode::TooManySlides)
        .map(|p| p.message.as_str());
    assert_eq!(
        message,
        Some("The presentation has 301 slides; at most 300 are supported")
    );
    assert_eq!(
        problems
            .iter()
            .filter(|p| p.code == ProblemCode::TooManySlides)
            .count(),
        1
    );
    assert_eq!(
        problems
            .iter()
            .filter(|p| p.code == ProblemCode::FileMissing)
            .count(),
        301
    );
}

#[test]
fn reports_listed_files_that_are_missing_and_files_nothing_lists_or_references() {
    assert_eq!(
        found(|files| {
            files.remove(LAYOUT);
        }),
        pairs(&[
            ("file-missing", LAYOUT),
            ("layout-missing", SLIDE),
            ("layout-missing", SLIDE_TWO),
        ])
    );
    let extra = lint_with(|files| {
        files.insert("slides/ir-s9.html".into(), Some(slide_one_html()));
        files.insert(format!("media/{}.png", "d4".repeat(32)), None);
        files.insert("notes.txt".into(), Some("x".into()));
    });
    let extra: Vec<(&str, ProblemSeverity, &str)> = extra
        .iter()
        .map(|p| (p.code.name(), p.severity, p.file.as_str()))
        .collect();
    assert_eq!(
        extra,
        [
            (
                "orphan-file",
                ProblemSeverity::Warning,
                format!("media/{}.png", "d4".repeat(32)).as_str()
            ),
            ("orphan-file", ProblemSeverity::Warning, "notes.txt"),
            (
                "unlisted-document",
                ProblemSeverity::Warning,
                "slides/ir-s9.html"
            ),
        ]
    );
}

#[test]
fn requires_each_document_root_to_match_its_manifest_entry() {
    assert_eq!(
        found(in_slide("data-slide=\"ir-s1\"", "data-slide=\"ir-s7\"")),
        pairs(&[("document-root", SLIDE)])
    );
    assert_eq!(
        found(in_slide(" data-layout=\"ir-l1\"", "")),
        pairs(&[("document-root", SLIDE)])
    );
    assert_eq!(
        found(replace_in(
            LAYOUT,
            "data-layout-root".into(),
            "data-x".into()
        )),
        pairs(&[("document-root", LAYOUT)])
    );
    let messages: Vec<String> = lint_with(in_slide(" data-layout=\"ir-l1\"", ""))
        .into_iter()
        .map(|p| p.message)
        .collect();
    assert_eq!(
        messages,
        ["slides/ir-s1.html: its data-layout (none) differs from the manifest's (ir-l1)"]
    );
    // The root is the document's first <main>, even when a later one carries the entry's attribute.
    let messages: Vec<String> = lint_with(in_slide("<body>\n", "<body>\n<main></main>\n"))
        .into_iter()
        .map(|p| p.message)
        .collect();
    assert_eq!(
        messages,
        ["slides/ir-s1.html: its <main> is not data-slide=\"ir-s1\""]
    );
}

#[test]
fn refuses_scripts_event_handlers_javascript_urls_and_other_link_schemes() {
    assert_eq!(
        found(in_slide("</main>", "<script>alert(1)</script></main>")),
        pairs(&[("script", SLIDE)])
    );
    assert_eq!(
        found(in_slide(
            "<div data-el=\"ir-s1.title\"",
            "<div onclick=\"x()\" data-el=\"ir-s1.title\""
        )),
        pairs(&[("event-handler", SLIDE)])
    );
    assert_eq!(
        found(in_slide(
            "</main>",
            "<a href=\" JavaScript:alert(1)\">x</a></main>"
        )),
        pairs(&[("javascript-url", SLIDE)])
    );
    assert_eq!(
        found(in_slide(
            "</main>",
            "<a href=\"file:///etc/passwd\">x</a></main>"
        )),
        pairs(&[("link-scheme", SLIDE)])
    );
    assert_eq!(
        found(in_slide(
            "</main>",
            "<a href=\"https://a.example\" ping=\"https://b.example/p\">x</a></main>"
        )),
        pairs(&[("remote-url", SLIDE)])
    );
    assert_eq!(
        found(in_slide(
            "</main>",
            "<a href=\"//x.example\">x</a><a href=\"../outside\">y</a></main>"
        )),
        pairs(&[("link-scheme", SLIDE), ("link-scheme", SLIDE)])
    );
    assert_eq!(
        found(in_slide(
            "</main>",
            "<a href=\"mailto:a@b.c\">x</a><a href=\"#top\">y</a></main>"
        )),
        []
    );
    // An SVG script is a script too.
    assert_eq!(
        found(in_slide(
            "</main>",
            "<svg><script>x()</script></svg></main>"
        )),
        pairs(&[("script", SLIDE)])
    );
}

#[test]
fn refuses_elements_that_run_embed_another_document_navigate_or_submit() {
    for (hostile, element) in [
        (
            "<iframe srcdoc=\"&lt;script&gt;alert(1)&lt;/script&gt;\"></iframe>",
            "iframe",
        ),
        ("<object></object>", "object"),
        ("<embed type=\"image/svg+xml\">", "embed"),
        ("<form><button>x</button></form>", "form"),
        (
            "<svg><foreignObject><div></div></foreignObject></svg>",
            "foreignObject",
        ),
        ("<svg><set attributeName=\"fill\" to=\"red\"/></svg>", "set"),
        ("<template><iframe></iframe></template>", "iframe"),
    ] {
        let problems = lint_with(in_slide("</main>", &format!("{hostile}</main>")));
        let refused = problems.iter().any(|p| {
            p.code == ProblemCode::ElementNotAllowed
                && p.file == SLIDE
                && p.message.contains(&format!("<{element}>"))
        });
        assert!(refused, "{hostile}: {problems:?}");
    }
    assert_eq!(
        found(in_slide(
            "</head>",
            "<meta http-equiv=\"refresh\" content=\"0;url=https://x.example\"></head>"
        )),
        pairs(&[("element-not-allowed", SLIDE)])
    );
}

#[test]
fn accepts_the_inert_html_and_svg_vocabulary() {
    let inert = "<ul><li><strong>a</strong> <em>b</em><sup>2</sup></li></ul><table><tr><td>c</td></tr></table>\
        <svg><defs><linearGradient id=\"g\"><stop offset=\"0\"/></linearGradient><clipPath id=\"c\"><rect/></clipPath>\
        <filter id=\"f\"><feGaussianBlur stdDeviation=\"2\"/></filter></defs><g><path d=\"M0 0\"/><text>t</text></g></svg>";
    assert_eq!(found(in_slide("</main>", &format!("{inert}</main>"))), []);
}

#[test]
fn accepts_one_reset_rule_byte_for_byte_and_no_other_style() {
    assert_eq!(
        found(in_slide("p{margin:0}", "p{margin:1px}")),
        pairs(&[("extra-style", SLIDE), ("reset-style", SLIDE)])
    );
    assert_eq!(
        found(in_slide(
            "</head>",
            "<link rel=\"stylesheet\" href=\"x.css\"></head>"
        )),
        pairs(&[("extra-style", SLIDE), ("media-reference", SLIDE)])
    );
    assert_eq!(
        found(in_slide("</main>", "<svg><style>p{}</style></svg></main>")),
        pairs(&[("extra-style", SLIDE)])
    );
    let reset = format!("<style>{}</style>", slidock_format::RESET_STYLE);
    let twice = lint_with(in_slide("</head>", &format!("{reset}</head>")));
    let messages: Vec<&str> = twice.iter().map(|p| p.message.as_str()).collect();
    assert_eq!(
        messages,
        ["slides/ir-s1.html: the reset <style> appears twice"]
    );
}

#[test]
fn requires_every_data_el_element_to_be_placed_absolutely_with_left_top_and_width() {
    let problems = lint_with(in_slide(
        TITLE_DIV,
        &TITLE_DIV.replace("position:absolute;left:40pt;", ""),
    ));
    let problems: Vec<(&str, &str, &str)> = problems
        .iter()
        .map(|p| (p.code.name(), p.file.as_str(), p.message.as_str()))
        .collect();
    assert_eq!(
        problems,
        [(
            "element-unpositioned",
            SLIDE,
            "The element ir-s1.title in slides/ir-s1.html lacks position:absolute, left; elements are placed absolutely in the slide box",
        )]
    );
    assert_eq!(
        found(in_slide(
            "style=\"position:absolute;left:40pt;top:40pt;",
            "style=\"color:red;}"
        )),
        pairs(&[("style-invalid", SLIDE)])
    );
    // The message carries the reason the style does not parse, and a bracket the value ends inside is unclosed
    // whatever its last character.
    for (style, reason) in [
        ("color:red}", "Unexpected }"),
        ("color:red background:blue", "Missed semicolon"),
        ("@media x{}", "a atrule is not a declaration"),
        ("height:calc(80pt + (1pt)", "Unclosed bracket"),
    ] {
        let messages: Vec<String> = lint_with(in_slide(
            "height:80pt\"><div data-sd-text",
            &format!("{style}\"><div data-sd-text"),
        ))
        .into_iter()
        .map(|p| p.message)
        .collect();
        assert_eq!(
            messages,
            [format!(
                "slides/ir-s1.html: a style attribute does not parse ({reason})"
            )],
            "{style}"
        );
    }
    // Keywords are read as tokens: `ABSOLUTE !important` places the element.
    assert_eq!(
        found(in_slide(
            "position:absolute;left:40pt;top:40pt;",
            "position:ABSOLUTE !important;left:40pt;top:40pt;"
        )),
        []
    );
}

#[test]
fn refuses_an_element_id_twice_in_a_document_or_in_a_slide_composed_with_its_layout() {
    assert_eq!(
        found(in_slide(
            "data-el=\"ir-s1.photo\"",
            "data-el=\"ir-s1.title\""
        )),
        pairs(&[("duplicate-element", SLIDE)])
    );
    assert_eq!(
        found(in_slide(
            "data-el=\"ir-s1.photo\"",
            "data-el=\"ir-l1.band\""
        )),
        pairs(&[("duplicate-element", SLIDE)])
    );
    let messages: Vec<String> = lint_with(in_slide(
        "data-el=\"ir-s1.photo\"",
        "data-el=\"ir-l1.band\"",
    ))
    .into_iter()
    .map(|p| p.message)
    .collect();
    assert_eq!(
        messages,
        [
            "data-el ir-l1.band appears more than once in slides/ir-s1.html composed with layouts/ir-l1.html"
        ]
    );
    // The second slide hides its layout, so the layout's ids and its own stay apart.
    let hidden = lint_with(replace_in(
        SLIDE_TWO,
        "data-el=\"ir-S2.link\"".into(),
        "data-el=\"ir-l1.band\"".into(),
    ));
    assert_eq!(hidden, []);
}

#[test]
fn accepts_media_only_as_media_files_of_the_set() {
    let photo = format!("../media/{PHOTO}.png");
    assert_eq!(
        found(in_slide(&photo, &format!("media/{PHOTO}.png"))),
        pairs(&[("media-reference", SLIDE)])
    );
    assert_eq!(
        found(in_slide(
            &photo,
            &format!("../media/{}.png", "e5".repeat(32))
        )),
        pairs(&[("media-missing", SLIDE)])
    );
    assert_eq!(
        found(in_slide(&photo, &format!("../media/{PHOTO}.html"))),
        pairs(&[("media-reference", SLIDE)])
    );
    for remote in [
        "https://example.com/a.png",
        "data:image/png;base64,AA",
        "blob:x",
        "//example.com/a.png",
    ] {
        assert_eq!(
            found(in_slide(&photo, remote)),
            pairs(&[("remote-url", SLIDE)]),
            "{remote}"
        );
    }
    // Every attribute that loads its value, on any element.
    for markup in [
        "<img src=\"https://x.example/a\">",
        "<video src=\"https://x.example/a\" poster=\"../media/x.jpg\"></video>",
        "<video poster=\"https://x.example/a\"></video>",
        "<table background=\"https://x.example/a\"><tr><td>c</td></tr></table>",
        "<table><tr><td background=\"https://x.example/a\">c</td></tr></table>",
        "<div data=\"https://x.example/a\"></div>",
        "<svg><image href=\"https://x.example/a\"></image></svg>",
    ] {
        let problems = found(in_slide("</main>", &format!("{markup}</main>")));
        assert!(
            problems.contains(&("remote-url", SLIDE.to_owned())),
            "{markup}: {problems:?}"
        );
    }
}

#[test]
fn reports_at_most_120_utf16_code_units_of_a_refused_reference() {
    let photo = format!("../media/{PHOTO}.png");
    let link = "<a href=\"https://example.com/\"";
    for (prefix, code) in [
        ("https://", "remote-url"),
        ("../", "media-reference"),
        ("ftp://", "link-scheme"),
    ] {
        let (path, from) = if code == "link-scheme" {
            (SLIDE_TWO, link.to_owned())
        } else {
            (SLIDE, photo.clone())
        };
        let reference = |filler: usize| format!("{prefix}{}\u{1f600}tail", "a".repeat(filler));
        let to = |reference: &str| {
            if code == "link-scheme" {
                format!("<a href=\"{reference}\"")
            } else {
                reference.to_owned()
            }
        };
        let message = |filler: usize| {
            let problems = lint_with(replace_in(path, from.clone(), to(&reference(filler))));
            let messages: Vec<String> = problems
                .iter()
                .filter(|p| p.code.name() == code)
                .map(|p| p.message.clone())
                .collect();
            assert_eq!(messages.len(), 1, "{code}: {problems:?}");
            messages[0].clone()
        };
        // A character outside the BMP is two code units: it is kept when both fit, and dropped when the 120th unit
        // would cut it in half.
        let fits = 118 - prefix.len();
        let shown = format!(" {prefix}{}\u{1f600};", "a".repeat(fits));
        assert!(message(fits).contains(&shown), "{code}: {}", message(fits));
        let cut = 119 - prefix.len();
        let shown = format!(" {prefix}{};", "a".repeat(cut));
        assert!(message(cut).contains(&shown), "{code}: {}", message(cut));
    }
}

#[test]
fn sorts_problems_by_utf16_code_units() {
    // U+FF61 sorts before U+10000 by UTF-8 bytes, and after it by UTF-16 code units (a surrogate is 0xD800).
    let files: Vec<String> = lint_with(|files| {
        files.insert("notes-\u{ff61}.txt".into(), Some("x".into()));
        files.insert("notes-\u{10000}.txt".into(), Some("x".into()));
    })
    .into_iter()
    .map(|p| p.file)
    .collect();
    assert_eq!(files, ["notes-\u{10000}.txt", "notes-\u{ff61}.txt"]);
}

#[test]
fn reads_urls_in_css_the_way_browsers_do() {
    assert_eq!(
        found(in_slide(
            TITLE_DIV,
            &format!("{TITLE_DIV}background:u\\72l(https://x.y/a.png);")
        )),
        pairs(&[("remote-url", SLIDE)])
    );
    assert_eq!(
        found(in_slide(
            TITLE_DIV,
            &format!("{TITLE_DIV}background:image-set(&quot;a.png&quot; 1x);")
        )),
        pairs(&[("media-reference", SLIDE)])
    );
    assert_eq!(
        found(in_slide(
            "</main>",
            "<svg><rect fill=\"url(https://x.y/#g)\"></rect></svg></main>"
        )),
        pairs(&[("remote-url", SLIDE)])
    );
    // A bad URL is still a URL.
    assert_eq!(
        found(in_slide(
            TITLE_DIV,
            &format!("{TITLE_DIV}background:url(https://x.y/a b.png);")
        )),
        pairs(&[("remote-url", SLIDE)])
    );
    for name in [
        "image-set",
        "-webkit-image-set",
        "image",
        "src",
        "cross-fade",
    ] {
        assert_eq!(
            found(in_slide(
                TITLE_DIV,
                &format!("{TITLE_DIV}background:{name}(&quot;../media/{PHOTO}.png&quot;);")
            )),
            pairs(&[("media-reference", SLIDE)]),
            "{name}"
        );
    }
    // A URL-looking font name is text, and a fragment reference stays inside the document.
    assert_eq!(
        found(in_slide(
            TITLE_DIV,
            &format!(
                "{TITLE_DIV}font-family:&quot;url(https://x.y)&quot;;mask:url(&quot;#m&quot;);"
            )
        )),
        []
    );
}

#[test]
fn refuses_fixed_positions_viewport_units_and_videos_without_posters() {
    let fixed = found(in_slide(
        "position:absolute;left:40pt",
        "position:fixed;left:40pt",
    ));
    assert!(
        fixed.contains(&("fixed-position", SLIDE.to_owned())),
        "{fixed:?}"
    );
    let escaped = found(in_slide(
        "position:absolute;left:40pt",
        "position:fi\\78 ed;left:40pt",
    ));
    assert!(
        escaped.contains(&("fixed-position", SLIDE.to_owned())),
        "{escaped:?}"
    );
    assert_eq!(
        found(in_slide("width:640pt", "width:50vw")),
        pairs(&[("viewport-units", SLIDE)])
    );
    assert_eq!(
        found(in_slide("width:640pt", "width:calc(50dvh + 1pt)")),
        pairs(&[("viewport-units", SLIDE)])
    );
    assert_eq!(
        found(in_slide(
            &format!(" poster=\"../media/{}.jpg\"", "c3".repeat(32)),
            ""
        )),
        pairs(&[
            ("orphan-file", &format!("media/{}.jpg", "c3".repeat(32))),
            ("video-poster", SLIDE)
        ])
    );
}

#[test]
fn reads_documents_as_browsers_parse_them_comments_and_text_are_not_markup() {
    let quiet = in_slide(
        "</main>",
        "<!-- <script>x()</script> <div data-el=\"ir-s1.title\"> --><p>&lt;script&gt; and position:fixed</p></main>",
    );
    assert_eq!(found(quiet), []);
}

#[test]
fn accepts_a_failed_document_its_background_and_a_marker() {
    let mut manifest = sample_manifest();
    manifest.slides[0].failed = Some("IR0908".into());
    let failed = document_html(
        "One",
        "data-slide=\"ir-s1\" data-layout=\"ir-l1\" data-failed=\"IR0908\"",
        &[
            "<div data-sd-bg style=\"position:absolute;left:0pt;top:0pt;width:720pt;height:405pt;background:#202020\"></div>",
            "<div data-sd-failed style=\"position:absolute;left:0pt;top:0pt;width:720pt;height:405pt;box-sizing:border-box;outline:2pt dashed #c00;font:9pt sans-serif;color:#c00\">IR0908</div>",
        ],
        &[],
    );
    let problems = found(|files| {
        files.insert(
            "presentation.json".into(),
            Some(serialize_manifest(&manifest).unwrap()),
        );
        files.insert(SLIDE.into(), Some(failed));
        files.remove(&format!("media/{}.mp4", "b2".repeat(32)));
        files.remove(&format!("media/{}.jpg", "c3".repeat(32)));
    });
    assert_eq!(problems, []);
}

#[test]
fn names_a_file_in_every_problem_from_one_registry_of_messages() {
    assert!(ProblemCode::ALL.len() > 20);
    let problems = lint_with(in_slide(
        "</main>",
        "<script></script><video></video></main>",
    ));
    assert!(!problems.is_empty());
    for problem in problems {
        assert_eq!(problem.file, SLIDE);
        let unfilled = problem.message.split('{').skip(1).any(|rest| {
            let key: String = rest.chars().take_while(char::is_ascii_lowercase).collect();
            !key.is_empty() && rest[key.len()..].starts_with('}')
        });
        assert!(!unfilled, "{}", problem.message);
    }
}

#[test]
fn lints_documents_on_their_own_without_a_manifest() {
    let problems = found(|files| {
        files.remove("presentation.json");
    });
    // Each document's root is not checked against an entry, and its media still count as referenced.
    assert_eq!(
        problems,
        pairs(&[("manifest-missing", "presentation.json")])
    );
    let stray = found(|files| {
        files.remove("presentation.json");
        files.insert("notes.txt".into(), Some("x".into()));
    });
    assert_eq!(
        stray,
        pairs(&[
            ("orphan-file", "notes.txt"),
            ("manifest-missing", "presentation.json")
        ])
    );
}
