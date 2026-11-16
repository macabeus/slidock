//! The Google Fonts source: the generated catalog holds only open-licence families, `css2_url` asks only for
//! catalog families and faces they have, and `parse_css2` reads Google's recorded responses (`tests/data/google`).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

use slidock_fonts::FamilyKey;
use slidock_fonts::bind::WeightRange;
use slidock_fonts::google::{
    Css2Error, GoogleCatalog, Licence, Weights, css2_url, google_catalog, google_catalog_data,
    parse_css2,
};
use slidock_ir::fonts::{FaceStyle, GenericFamily};

const N: FaceStyle = FaceStyle::Normal;
const I: FaceStyle = FaceStyle::Italic;

fn recorded(name: &str) -> String {
    let path = format!("{}/tests/data/google/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path}: {error}"))
}

#[test]
fn every_catalog_line_parses_and_the_catalog_is_pinned() {
    let catalog = google_catalog();
    assert_eq!(
        catalog.families().len(),
        GoogleCatalog::data_lines(google_catalog_data())
    );
    assert!(
        catalog.families().len() > 1900,
        "{}",
        catalog.families().len()
    );
    assert_eq!(catalog.commit().len(), 40);
    assert!(
        catalog
            .families()
            .windows(2)
            .all(|pair| pair[0].family < pair[1].family)
    );
}

#[test]
fn the_catalog_holds_only_open_licences_and_never_commercial_families() {
    let catalog = google_catalog();
    for family in catalog.families() {
        assert!(matches!(
            family.licence,
            Licence::Ofl | Licence::Apache | Licence::Ufl
        ));
    }
    // css2 serves these (licensed to Google, unlisted): they must never be requested.
    for family in [
        "Helvetica Neue",
        "Helvetica",
        "Calibri",
        "Arial",
        "Times New Roman",
        "Avenir Next",
        "Segoe UI",
    ] {
        assert!(catalog.find(family).is_none(), "{family}");
        assert_eq!(css2_url(catalog, family, &[(400, N)]), None, "{family}");
    }
}

#[test]
fn the_corpus_families_are_in_the_catalog() {
    let catalog = google_catalog();
    let roboto = catalog.get(&FamilyKey::of("Roboto")).unwrap();
    assert_eq!(roboto.licence, Licence::Ofl);
    assert_eq!(roboto.generic, GenericFamily::SansSerif);
    assert_eq!(
        roboto.upright,
        Some(Weights::Range(WeightRange { min: 100, max: 900 }))
    );
    assert!(roboto.axes.iter().any(|axis| axis.tag == "wdth"));
    assert!(roboto.subsets.contains(&"latin") && roboto.subsets.contains(&"cyrillic"));
    let oswald = catalog.get(&FamilyKey::of("Oswald")).unwrap();
    assert_eq!(oswald.italic, None);
    assert!(oswald.has_face(200, N) && !oswald.has_face(800, N) && !oswald.has_face(400, I));
    let mono = catalog.find("Roboto Mono").unwrap();
    assert_eq!(mono.family.generic, GenericFamily::Monospace);
    let medium = catalog.find("Roboto Medium").unwrap();
    assert_eq!(
        (medium.family.family.as_str(), medium.weight),
        ("Roboto", Some(500))
    );
    let barlow = catalog.find("barlow-condensed").unwrap().family;
    assert_eq!(
        barlow.upright,
        Some(Weights::Statics(vec![
            100, 200, 300, 400, 500, 600, 700, 800, 900
        ]))
    );
}

#[test]
fn css2_urls_ask_only_for_faces_the_family_has() {
    let catalog = google_catalog();
    // The recorded requests.
    assert_eq!(
        css2_url(catalog, "Roboto", &[(700, N), (400, I), (400, N), (400, N)]).unwrap(),
        "https://fonts.googleapis.com/css2?family=Roboto:ital,wght@0,400;0,700;1,400&display=block"
    );
    assert_eq!(
        css2_url(catalog, "Oswald", &[(400, N), (700, N)]).unwrap(),
        "https://fonts.googleapis.com/css2?family=Oswald:wght@400;700&display=block"
    );
    // Oswald has no italic and stops at 700: the request asks for what a browser would draw instead.
    assert_eq!(
        css2_url(catalog, "Oswald", &[(400, I), (900, N)]).unwrap(),
        "https://fonts.googleapis.com/css2?family=Oswald:wght@400;700&display=block"
    );
    // A weight name sets the drawn weight; a bold run of it draws bold.
    assert_eq!(
        css2_url(catalog, "Oswald Medium", &[(400, N), (700, N)]).unwrap(),
        "https://fonts.googleapis.com/css2?family=Oswald:wght@500;700&display=block"
    );
    // Statics: 650 is not a Barlow Condensed weight, so CSS's choice (700) is asked for.
    assert_eq!(
        css2_url(catalog, "Barlow Condensed", &[(650, I)]).unwrap(),
        "https://fonts.googleapis.com/css2?family=Barlow+Condensed:ital,wght@1,700&display=block"
    );
    // No slot asks for the regular face; no URL ever carries text=.
    let url = css2_url(catalog, "Roboto Mono", &[]).unwrap();
    assert_eq!(
        url,
        "https://fonts.googleapis.com/css2?family=Roboto+Mono:wght@400&display=block"
    );
    assert!(!url.contains("text="));
}

#[test]
fn parses_the_static_ttf_response_a_non_browser_agent_gets() {
    let faces = parse_css2(&recorded("roboto-cli.css")).unwrap();
    let summary: Vec<(FaceStyle, WeightRange, Option<&str>, bool)> = faces
        .iter()
        .map(|face| {
            (
                face.style,
                face.weight,
                face.format.as_deref(),
                face.unicode_range.is_none(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        vec![
            (I, WeightRange::point(400), Some("truetype"), true),
            (N, WeightRange::point(400), Some("truetype"), true),
            (N, WeightRange::point(700), Some("truetype"), true),
        ]
    );
    assert!(faces.iter().all(|face| face.family == "Roboto"
        && face.url.starts_with("https://fonts.gstatic.com/s/roboto/")
        && face.url.ends_with(".ttf")));
    let two = parse_css2(&recorded("two-families-cli.css")).unwrap();
    let families: Vec<&str> = two.iter().map(|face| face.family.as_str()).collect();
    assert_eq!(
        families,
        ["Barlow Condensed", "Barlow Condensed", "Roboto Mono"]
    );
    assert_eq!(parse_css2(&recorded("oswald-cli.css")).unwrap().len(), 2);
}

#[test]
fn parses_the_unicode_range_chunks_a_browser_gets() {
    let faces = parse_css2(&recorded("roboto-browser.css")).unwrap();
    assert_eq!(faces.len(), 27);
    assert!(
        faces
            .iter()
            .all(|face| face.format.as_deref() == Some("woff2"))
    );
    let latin: Vec<_> = faces
        .iter()
        .filter(|face| face.subset.as_deref() == Some("latin"))
        .collect();
    assert_eq!(latin.len(), 3);
    let range = latin[0].unicode_range.as_ref().unwrap();
    assert!(range.contains(u32::from('A')) && range.contains(0xC0) && !range.contains(0x0410));
    let cyrillic = faces
        .iter()
        .find(|face| face.subset.as_deref() == Some("cyrillic"))
        .unwrap();
    assert!(cyrillic.unicode_range.as_ref().unwrap().contains(0x0410));
    // Oswald's browser response points both weights at the same variable chunk.
    let oswald = parse_css2(&recorded("oswald-browser.css")).unwrap();
    let latin: Vec<_> = oswald
        .iter()
        .filter(|face| face.subset.as_deref() == Some("latin"))
        .collect();
    assert_eq!(latin.len(), 2);
    assert_eq!(latin[0].url, latin[1].url);
}

#[test]
fn refuses_responses_that_are_not_css_or_point_elsewhere() {
    assert_eq!(
        parse_css2("<!DOCTYPE html><html lang=en><head><title>Error 400 (Bad Request)</title>"),
        Err(Css2Error::NoFontFace)
    );
    let elsewhere =
        "@font-face { font-family: 'X'; src: url(https://evil.example/x.ttf) format('truetype'); }";
    assert!(matches!(
        parse_css2(elsewhere),
        Err(Css2Error::UrlRefused(_))
    ));
    let scheme = "@font-face { font-family: 'X'; src: url(data:font/ttf;base64,AAAA); }";
    assert!(matches!(parse_css2(scheme), Err(Css2Error::UrlRefused(_))));
    // A look-alike host is refused (the check is on the whole prefix, slash included).
    let lookalike = "@font-face { font-family: 'X'; src: url(https://fonts.gstatic.com.evil.example/s/x.ttf); }";
    assert!(matches!(
        parse_css2(lookalike),
        Err(Css2Error::UrlRefused(_))
    ));
    // Negative control: the same rule on gstatic is read.
    let ok = "@font-face { font-family: 'X'; font-weight: 200 700; src: url(https://fonts.gstatic.com/s/x/v1/x.woff2) format('woff2'); }";
    let face = &parse_css2(ok).unwrap()[0];
    assert_eq!(face.weight, WeightRange { min: 200, max: 700 });
    assert!(matches!(
        parse_css2("@font-face { font-family: 'X'; src: url(https://fonts.gstatic.com/s/x.ttf)"),
        Err(Css2Error::Syntax(_))
    ));
    assert_eq!(
        parse_css2(&" ".repeat(slidock_fonts::google::MAX_CSS2_BYTES + 1)),
        Err(Css2Error::TooLarge)
    );
}

/// A deterministic xorshift generator for mutations.
struct Mutator(u64);

impl Mutator {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    /// `text` with a few edits: characters CSS gives meaning to inserted, ranges deleted or repeated.
    fn mutate(&mut self, text: &str) -> String {
        const PIECES: &[&str] = &[
            "{",
            "}",
            "(",
            ")",
            ";",
            ":",
            "'",
            "\"",
            "/*",
            "*/",
            "@font-face",
            ",",
            "-",
            "U+",
            "?",
            "url(",
            "format(",
            "\u{00e9}",
            "\n",
        ];
        let mut chars: Vec<char> = text.chars().collect();
        for _ in 0..1 + self.below(4) {
            let at = self.below(chars.len() + 1);
            match self.below(3) {
                0 => {
                    let piece = PIECES[self.below(PIECES.len())];
                    chars.splice(at..at, piece.chars());
                }
                1 => {
                    let end = (at + self.below(64)).min(chars.len());
                    chars.drain(at..end);
                }
                _ => {
                    let end = (at + self.below(64)).min(chars.len());
                    let copy: Vec<char> = chars[at..end].to_vec();
                    chars.splice(at..at, copy);
                }
            }
        }
        chars.into_iter().collect()
    }
}

#[test]
#[allow(clippy::disallowed_types, reason = "the test times each input")]
fn reading_takes_one_pass_over_a_response_at_the_size_limit() {
    let limit = slidock_fonts::google::MAX_CSS2_BYTES;
    let rule = "@font-face{font-family:'X';src:url(https://fonts.gstatic.com/s/x.ttf)}";
    let inputs = [
        // Comments and no rule: each comment once rescanned the rest of the response for a rule.
        "/**/".repeat(limit / 4),
        // Slashes and at-signs that start neither a comment nor a rule.
        "/@".repeat(limit / 2),
        format!("{}{rule}", "/*latin*/".repeat((limit - rule.len()) / 9)),
    ];
    for css in &inputs {
        assert!(css.len() <= limit);
        let started = std::time::Instant::now();
        let parsed = parse_css2(css);
        let elapsed = started.elapsed();
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "{} bytes took {elapsed:?}",
            css.len()
        );
        assert!(matches!(parsed, Err(Css2Error::NoFontFace) | Ok(_)));
    }
    assert_eq!(
        parse_css2(&inputs[2]).unwrap()[0].subset.as_deref(),
        Some("latin")
    );
}

#[test]
fn mutated_responses_never_panic() {
    let mut mutator = Mutator(0x5eed_c552);
    for name in [
        "oswald-browser.css",
        "oswald-cli.css",
        "roboto-browser.css",
        "roboto-cli.css",
        "two-families-cli.css",
    ] {
        let css = recorded(name);
        for _ in 0..1500 {
            let mutated = mutator.mutate(&css);
            if let Ok(faces) = parse_css2(&mutated) {
                for face in faces {
                    assert!(
                        face.url.starts_with("https://fonts.gstatic.com/"),
                        "{}",
                        face.url
                    );
                }
            }
        }
    }
}

#[test]
fn mutated_unicode_ranges_never_panic() {
    let mut mutator = Mutator(0x0055_2b30);
    let seeds = [
        "U+0000-00FF, U+0131, U+0152-0153, U+02BB-02BC, U+2000-206F, U+FEFF, U+FFFD",
        "U+0460-052F, U+1C80-1C8A, U+20B4, U+2DE0-2DFF, U+A640-A69F, U+FE2E-FE2F",
        "U+4??, U+1F00-1FFF, u+10FFFF",
    ];
    for seed in seeds {
        for _ in 0..3000 {
            let value = mutator.mutate(seed);
            if let Some(range) = slidock_fonts::google::UnicodeRange::parse(&value) {
                assert!(range.ranges().windows(2).all(|w| w[0].1 < w[1].0));
                assert!(
                    range
                        .ranges()
                        .iter()
                        .all(|&(lo, hi)| lo <= hi && hi <= 0x10_FFFF)
                );
            }
        }
    }
}
