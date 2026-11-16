//! Fixtures shared by the integration tests: a two-slide document set on one layout, in the exact shell the HTML
//! backend writes.
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

use std::collections::BTreeMap;

use slidock_format::manifest::{
    Generator, Manifest, ManifestLayout, ManifestSize, ManifestSlide, ManifestSource,
    ManifestTheme, ManifestTransition, SchemeColors, SizeUnit, TransitionFallback,
};
use slidock_format::paths::{MediaExtension, media_path};
use slidock_format::{PresentationFiles, RESET_STYLE, serialize_manifest};

pub const PHOTO: &str = "a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1";
pub const CLIP: &str = "b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2";
pub const POSTER: &str = "c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3";

pub const SLIDE: &str = "slides/ir-s1.html";
pub const SLIDE_TWO: &str = "slides/ir-s2-da7f247d.html";
pub const LAYOUT: &str = "layouts/ir-l1.html";

const BOX: &str = "position:relative;width:720pt;height:405pt;overflow:hidden";
const FULL: &str = "position:absolute;left:0pt;top:0pt;width:720pt;height:405pt";

/// A small valid manifest: two slides on one layout.
pub fn sample_manifest() -> Manifest {
    Manifest {
        title: "Sample".into(),
        size: ManifestSize {
            w: 720.0,
            h: 405.0,
            unit: SizeUnit::Pt,
        },
        slides: vec![
            ManifestSlide {
                id: "ir-s1".into(),
                file: "slides/ir-s1.html".into(),
                title: "One".into(),
                hidden: false,
                layout: Some("ir-l1".into()),
                notes: None,
                transition: None,
                failed: None,
            },
            ManifestSlide {
                id: "ir-S2".into(),
                file: "slides/ir-s2-da7f247d.html".into(),
                title: String::new(),
                hidden: true,
                layout: Some("ir-l1".into()),
                notes: Some("First\nSecond".into()),
                transition: Some(ManifestTransition {
                    effect: "pptx:fade".into(),
                    fallback: TransitionFallback::Dissolve,
                    duration_ms: 500,
                    advance: None,
                }),
                failed: None,
            },
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

/// A document in the exact shell the HTML backend writes.
pub fn document_html(title: &str, main: &str, children: &[&str], head: &[&str]) -> String {
    let mut lines = vec![
        "<!doctype html>".to_owned(),
        "<html>".to_owned(),
        "<head>".to_owned(),
        "<meta charset=\"utf-8\">".to_owned(),
        format!("<title>{title}</title>"),
        format!("<style>{RESET_STYLE}</style>"),
    ];
    lines.extend(head.iter().map(|line| (*line).to_owned()));
    lines.extend([
        "</head>".to_owned(),
        "<body>".to_owned(),
        format!("<main {main} style=\"{BOX}\">"),
    ]);
    lines.extend(children.iter().map(|line| (*line).to_owned()));
    lines.extend(["</main>", "</body>", "</html>", ""].map(str::to_owned));
    lines.join("\n")
}

pub fn layout_html() -> String {
    document_html(
        "Title",
        "data-layout-root=\"ir-l1\"",
        &[
            &format!("<div data-sd-bg style=\"{FULL};background:#202020\"></div>"),
            "<div data-el=\"ir-l1.band\" data-kind=\"shape\" style=\"position:absolute;left:0pt;top:380pt;width:720pt;height:25pt;background:#4472c4\"></div>",
            "<div data-el=\"ir-l1.num\" data-kind=\"shape\" style=\"position:absolute;left:660pt;top:380pt;width:50pt;height:20pt\"><div data-sd-text style=\"position:absolute;left:0pt;top:0pt;width:100%;height:100%\"><p style=\"line-height:1.2\"><span data-field=\"slideNumber\" style=\"font-size:12pt\">‹#›</span></p></div></div>",
        ],
        &[],
    )
}

pub fn slide_one_html() -> String {
    document_html(
        "One",
        "data-slide=\"ir-s1\" data-layout=\"ir-l1\"",
        &[
            "<div data-el=\"ir-s1.title\" data-kind=\"shape\" style=\"position:absolute;left:40pt;top:40pt;width:640pt;height:80pt\"><div data-sd-text style=\"position:absolute;left:0pt;top:0pt;width:100%;height:100%;font-family:&quot;Liberation Sans&quot;,sans-serif\"><p style=\"line-height:1.2\"><span style=\"font-size:40pt;color:var(--sd-color-tx1,#ffffff)\">One</span></p></div></div>",
            &format!(
                "<div data-el=\"ir-s1.photo\" data-kind=\"picture\" style=\"position:absolute;left:40pt;top:140pt;width:200pt;height:150pt;overflow:hidden;clip-path:path(&quot;M0 0L10 0Z&quot;)\"><img src=\"../media/{PHOTO}.png\" alt=\"\" style=\"position:absolute;left:0%;top:0%;width:100%;height:100%;display:block\"></div>"
            ),
            &format!(
                "<div data-el=\"ir-s1.clip\" data-kind=\"media\" style=\"position:absolute;left:300pt;top:140pt;width:320pt;height:180pt\"><video src=\"../media/{CLIP}.mp4\" poster=\"../media/{POSTER}.jpg\" preload=\"none\" playsinline muted style=\"position:absolute;left:0pt;top:0pt;width:100%;height:100%;object-fit:fill\"></video></div>"
            ),
        ],
        &[
            "<template data-sd-timeline><ol>",
            "<li data-target=\"ir-s1.title\" data-trigger=\"click\" data-kind=\"entrance\" data-effect=\"keynote:dissolve\" data-fallback=\"dissolve\" data-delivery=\"all\" data-delay-ms=\"0\" data-duration-ms=\"500\"></li>",
            "</ol></template>",
        ],
    )
}

pub fn slide_two_html() -> String {
    document_html(
        "",
        "data-slide=\"ir-S2\" data-layout=\"ir-l1\" data-show-layout=\"false\"",
        &[
            &format!(
                "<div data-sd-bg style=\"{FULL};background-image:url(&quot;../media/{PHOTO}.png&quot;);background-size:100% 100%\"></div>"
            ),
            "<div data-el=\"ir-S2.link\" data-kind=\"shape\" style=\"position:absolute;left:10pt;top:10pt;width:100pt;height:20pt\"><div data-sd-text style=\"position:absolute;left:0pt;top:0pt;width:100%;height:100%\"><p><a href=\"https://example.com/\" style=\"color:inherit\"><span>Site</span></a></p></div></div>",
        ],
        &[],
    )
}

/// A file set that lints clean: two slides on one layout, with media.
pub fn sample_files() -> PresentationFiles {
    let media = |sha: &str, ext| media_path(sha, ext).unwrap();
    BTreeMap::from([
        (
            "presentation.json".to_owned(),
            Some(serialize_manifest(&sample_manifest()).unwrap()),
        ),
        (LAYOUT.to_owned(), Some(layout_html())),
        (SLIDE.to_owned(), Some(slide_one_html())),
        (SLIDE_TWO.to_owned(), Some(slide_two_html())),
        (media(PHOTO, MediaExtension::Png), None),
        (media(CLIP, MediaExtension::Mp4), None),
        (media(POSTER, MediaExtension::Jpg), None),
    ])
}

/// The text of a file of the set.
pub fn text<'f>(files: &'f PresentationFiles, path: &str) -> &'f str {
    files[path].as_deref().unwrap()
}
