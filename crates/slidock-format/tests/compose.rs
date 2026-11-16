//! Composition, with the `sd` markers.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod common;

use std::path::PathBuf;

use common::{CLIP, LAYOUT, PHOTO, SLIDE_TWO, layout_html, sample_files, slide_two_html};
use slidock_format::{
    ComposeError, MediaUrls, compose, compose_slide, compose_with_sources, lint_presentation,
};

const STYLE: &str = "position:absolute;left:0pt;top:0pt;width:10pt;height:10pt";

/// Compares with the committed file; `UPDATE_GOLDEN=1` rewrites it.
fn expect_golden(name: &str, text: &str) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden/compose")
        .join(name);
    if std::env::var("UPDATE_GOLDEN").as_deref() == Ok("1") {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
    }
    assert_eq!(text, std::fs::read_to_string(&path).unwrap(), "{name}");
}

/// The order of backgrounds and `data-el` elements in composed markup.
fn order(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(at) = rest.find("data-") {
        rest = &rest[at + 5..];
        if let Some(id) = rest.strip_prefix("el=\"") {
            out.push(id[..id.find('"').unwrap()].to_owned());
        } else if rest.starts_with("sd-bg") {
            out.push("bg".to_owned());
        }
    }
    out
}

fn composed(id: &str, media: &MediaUrls<'_>) -> String {
    compose_slide(&sample_files(), id, media).unwrap()
}

#[test]
fn puts_the_layout_behind_the_slide_with_the_slide_number_in_its_fields() {
    let files = sample_files();
    assert_eq!(lint_presentation(&files), []);
    let composed = compose_slide(&files, "ir-s1", &MediaUrls::Unchanged).unwrap();
    expect_golden("ir-s1.html", &composed);
    // The layout's background and elements come first, then the slide's; the field shows the show number.
    assert_eq!(
        order(&composed),
        [
            "bg",
            "ir-l1.band",
            "ir-l1.num",
            "ir-s1.title",
            "ir-s1.photo",
            "ir-s1.clip"
        ]
    );
    assert!(
        composed.contains("<span data-field=\"slideNumber\" style=\"font-size:12pt\">1</span>")
    );
    assert!(composed.contains("<template data-sd-timeline=\"\">"));
}

#[test]
fn keeps_the_slide_background_and_hides_the_layout_when_the_slide_says_so() {
    let composed = composed("ir-S2", &MediaUrls::Unchanged);
    expect_golden("ir-s2.html", &composed);
    assert!(!composed.contains("ir-l1.band"));
    assert_eq!(composed.matches("data-sd-bg").count(), 1);
    assert!(composed.contains(&format!("../media/{PHOTO}.png")));
}

#[test]
fn rewrites_media_references_for_a_document_written_outside_the_file_set() {
    let second = composed("ir-S2", &MediaUrls::Base("../presentation/media/"));
    assert!(second.contains(&format!(
        "url(&quot;../presentation/media/{PHOTO}.png&quot;)"
    )));
    assert!(!second.contains("\"../media/"));
    let first = composed("ir-s1", &MediaUrls::Base("../presentation/media"));
    assert!(first.contains(&format!("src=\"../presentation/media/{PHOTO}.png\"")));
    assert!(first.contains("poster=\"../presentation/media/"));
}

#[test]
fn writes_the_media_base_as_a_url_path_whatever_the_folder_is_called() {
    let base = MediaUrls::Base("../my presentation#1 (it's)/media");
    let second = composed("ir-S2", &base);
    assert!(second.contains(&format!(
        "url(&quot;../my%20presentation%231%20%28it%27s%29/media/{PHOTO}.png&quot;)"
    )));
    let first = composed("ir-s1", &base);
    assert!(first.contains(&format!(
        "src=\"../my%20presentation%231%20%28it%27s%29/media/{PHOTO}.png\""
    )));
}

#[test]
fn rewrites_every_media_reference_the_lint_accepts_in_each_form() {
    let forms = [
        format!(
            "<div style=\"{STYLE};background-image:URL( &quot;../media/{PHOTO}.png&quot; )\"></div>"
        ),
        format!("<div style=\"{STYLE};background-image:u\\72l(../media/{PHOTO}.png)\"></div>"),
        format!("<img srcset=\"../media/{PHOTO}.png 1x, ../media/{PHOTO}.png 2x\" alt=\"\">"),
        format!("<svg><rect fill=\"url(../media/{PHOTO}.png)\"></rect></svg>"),
        format!("<img src=\" ../media/{PHOTO}.png \" alt=\"\">"),
    ];
    for form in forms {
        let mut files = sample_files();
        files.insert(
            SLIDE_TWO.into(),
            Some(slide_two_html().replacen("</main>", &format!("{form}</main>"), 1)),
        );
        assert_eq!(lint_presentation(&files), [], "{form}");
        let url_of = |path: &str| format!("blob:x/\"{}", &path[6..10]);
        let composed = compose_slide(&files, "ir-S2", &MediaUrls::Url(&url_of)).unwrap();
        assert!(!composed.contains("../media/"), "{form}: {composed}");
    }
}

#[test]
fn names_each_media_file_by_the_url_media_url_gives_its_path() {
    let seen = std::cell::RefCell::new(Vec::new());
    let url_of = |path: &str| {
        seen.borrow_mut().push(path.to_owned());
        format!(
            "blob:http://127.0.0.1/{}",
            &path["media/".len().."media/".len() + 8]
        )
    };
    let first = composed("ir-s1", &MediaUrls::Url(&url_of));
    assert!(first.contains(&format!("src=\"blob:http://127.0.0.1/{}\"", &PHOTO[..8])));
    assert!(first.contains("poster=\"blob:http://127.0.0.1/"));
    assert!(!first.contains("../media/"));
    let second = composed("ir-S2", &MediaUrls::Url(&url_of));
    assert!(second.contains(&format!(
        "url(&quot;blob:http://127.0.0.1/{}&quot;)",
        &PHOTO[..8]
    )));
    assert!(!second.contains("../media/"));
    let seen = seen.into_inner();
    assert!(
        seen.iter()
            .all(|path| slidock_format::paths::is_media_path(path)),
        "{seen:?}"
    );
    assert!(seen.contains(&format!("media/{PHOTO}.png")));
}

#[test]
fn gives_the_same_document_for_a_media_url_that_returns_the_media_base_url() {
    for id in ["ir-s1", "ir-S2"] {
        let url_of = |path: &str| format!("../presentation/{path}");
        assert_eq!(
            composed(id, &MediaUrls::Url(&url_of)),
            composed(id, &MediaUrls::Base("../presentation/media"))
        );
    }
}

#[test]
fn offers_a_video_in_the_files_sources_gives_in_place_of_its_src() {
    let clip = format!("media/{CLIP}.mp4");
    let copy = format!("media/{CLIP}.webm");
    let sources = |path: &str| {
        if path == clip {
            vec![
                (path.to_owned(), "video/mp4".to_owned()),
                (copy.clone(), "video/webm; codecs=\"vp9\"".to_owned()),
            ]
        } else {
            Vec::new()
        }
    };
    let main = compose_with_sources(
        &sample_files(),
        "ir-s1",
        &MediaUrls::Base("media"),
        &sources,
    )
    .unwrap()
    .main_html();
    assert!(
        main.contains(&format!(
            "<video poster=\"media/{}.jpg\" preload=\"none\" playsinline=\"\" muted=\"\" style=",
            common::POSTER
        )),
        "{main}"
    );
    assert!(main.contains(&format!(
        "><source src=\"{clip}\" type=\"video/mp4\"><source src=\"{copy}\" type=\"video/webm; codecs=&quot;vp9&quot;\"></video>"
    )));
    // A video sources gives nothing for keeps its src: the same document as `compose`.
    let none = |_: &str| Vec::new();
    assert_eq!(
        compose_with_sources(&sample_files(), "ir-s1", &MediaUrls::Base("media"), &none)
            .unwrap()
            .main_html(),
        compose(&sample_files(), "ir-s1", &MediaUrls::Base("media"))
            .unwrap()
            .main_html()
    );
}

#[test]
fn is_deterministic() {
    assert_eq!(
        composed("ir-s1", &MediaUrls::Unchanged),
        composed("ir-s1", &MediaUrls::Unchanged)
    );
}

#[test]
fn refuses_what_is_not_a_presentation() {
    let empty = slidock_format::PresentationFiles::new();
    let error = compose_slide(&empty, "ir-s1", &MediaUrls::Unchanged).unwrap_err();
    assert_eq!(
        error,
        ComposeError::NotInFileSet("presentation.json".into())
    );
    assert!(error.to_string().contains("presentation.json"));
    let error = compose_slide(&sample_files(), "ir-s9", &MediaUrls::Unchanged).unwrap_err();
    assert_eq!(
        error.to_string(),
        "ir-s9 is not a slide of this presentation"
    );
    let mut files = sample_files();
    files.insert(
        LAYOUT.into(),
        Some(layout_html().replace("data-layout-root", "data-x")),
    );
    let error = compose_slide(&files, "ir-s1", &MediaUrls::Unchanged).unwrap_err();
    assert_eq!(
        error.to_string(),
        "layouts/ir-l1.html has no <main data-layout-root>"
    );
}

#[test]
fn gives_the_composed_main_for_the_deck_page() {
    let composed = compose(&sample_files(), "ir-s1", &MediaUrls::Base("media")).unwrap();
    let main = composed.main_html();
    assert!(main.starts_with("<main data-slide=\"ir-s1\" data-layout=\"ir-l1\" style=\""));
    assert!(main.ends_with("\n</main>"));
    assert_eq!(
        order(&main),
        [
            "bg",
            "ir-l1.band",
            "ir-l1.num",
            "ir-s1.title",
            "ir-s1.photo",
            "ir-s1.clip"
        ]
    );
    assert!(main.contains(&format!("src=\"media/{PHOTO}.png\"")));
    assert!(composed.document_html().contains(&main));
}

#[test]
fn rewrites_layout_media_as_the_slide_media() {
    let mut files = sample_files();
    let layout = layout_html().replace(
        "background:#202020",
        &format!("background-image:url(&quot;../media/{PHOTO}.png&quot;)"),
    );
    files.insert(LAYOUT.into(), Some(layout));
    let url_of = |path: &str| format!("blob:{path}");
    let composed = compose_slide(&files, "ir-s1", &MediaUrls::Url(&url_of)).unwrap();
    assert!(composed.contains(&format!("url(&quot;blob:media/{PHOTO}.png&quot;)")));
    assert!(!composed.contains("../media/"));
}
