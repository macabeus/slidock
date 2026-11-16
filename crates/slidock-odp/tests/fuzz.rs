//! Hostile packages: whatever the bytes, the frontend refuses the input or lowers it with diagnostics, within
//! seconds, and never fails a container for a crash (OD9005).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]
#![allow(clippy::disallowed_types, reason = "the test times each input")]

mod support;

use serde_json::json;
use slidock_core_api::{Frontend, FrontendError, Input, Limits, LowerOptions};
use slidock_io::BytesReader;
use slidock_odp::OdpFrontend;
use support::odp::{OdpSpec, PageSpec, files, odp, replace, unzip, zip_entries};
use support::{compile, fixtures, lower, lower_with, with_ctx};

const ITERATIONS: usize = 200;
const PER_INPUT_MS: u128 = 5000;

/// A xorshift generator: the same mutations on every run.
struct Random(u64);

impl Random {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }

    fn below(&mut self, n: usize) -> usize {
        ((self.next() * n as f64) as usize).min(n.saturating_sub(1))
    }
}

/// One structure-blind mutation: cut the tail, flip a bit, scramble 8 bytes, or repeat a range.
fn mutate(bytes: &[u8], random: &mut Random) -> Vec<u8> {
    if bytes.is_empty() {
        return Vec::new();
    }
    let at = random.below(bytes.len());
    let mut out = bytes.to_vec();
    match random.below(4) {
        0 => out.truncate(at),
        1 => out[at] ^= 1 << random.below(8),
        2 => {
            for byte in out.iter_mut().skip(at).take(8) {
                *byte = random.below(256) as u8;
            }
        }
        _ => {
            let len = random.below(64).min(bytes.len() - at);
            let piece = bytes[at..at + len].to_vec();
            out.splice(at..at, piece);
        }
    }
    out
}

const EXTREMES: [&str; 10] = [
    "0",
    "-1",
    "9007199254740992",
    "9e15",
    "1e308cm",
    "-99999999999999999999",
    "NaN",
    "",
    "1e308",
    "constructor",
];

/// Structure-aware mutations of an XML part, which stays well-formed: attribute values replaced with extremes or
/// swapped, and empty elements dropped.
fn mutate_xml(bytes: &[u8], random: &mut Random) -> Vec<u8> {
    let mut text = String::from_utf8(bytes.to_vec()).unwrap();
    for _ in 0..3 {
        let values: Vec<(usize, usize)> = text
            .match_indices("=\"")
            .filter_map(|(at, _)| {
                let start = at + 2;
                text[start..].find('"').map(|len| (start, start + len))
            })
            .filter(|(start, _)| {
                !text[..*start].ends_with("xmlns=\"")
                    && !text[..*start - 2]
                        .rsplit(' ')
                        .next()
                        .unwrap_or("")
                        .starts_with("xmlns")
            })
            .collect();
        match random.below(3) {
            0 if !values.is_empty() => {
                let (start, end) = values[random.below(values.len())];
                text.replace_range(start..end, EXTREMES[random.below(EXTREMES.len())]);
            }
            1 if values.len() > 1 => {
                let (a, b) = (
                    values[random.below(values.len())],
                    values[random.below(values.len())],
                );
                let (first, second) = if a.0 <= b.0 { (a, b) } else { (b, a) };
                if first.1 <= second.0 {
                    let (x, y) = (
                        text[first.0..first.1].to_owned(),
                        text[second.0..second.1].to_owned(),
                    );
                    text.replace_range(second.0..second.1, &x);
                    text.replace_range(first.0..first.1, &y);
                }
            }
            _ => {
                let empties: Vec<(usize, usize)> = text
                    .match_indices("/>")
                    .filter_map(|(end, _)| text[..end].rfind('<').map(|start| (start, end + 2)))
                    .filter(|(start, end)| {
                        !text[*start..*end].contains('>') || text[*start..*end].ends_with("/>")
                    })
                    .filter(|(start, end)| !text[start + 1..*end].contains('<'))
                    .collect();
                if !empties.is_empty() {
                    let (start, end) = empties[random.below(empties.len())];
                    text.replace_range(start..end, "");
                }
            }
        }
    }
    text.into_bytes()
}

/// What an input did: refused as a whole or lowered; and what went wrong.
fn outcome(bytes: &[u8]) -> (bool, Option<String>) {
    outcome_by(false, bytes)
}

fn outcome_by(native_motion: bool, bytes: &[u8]) -> (bool, Option<String>) {
    let frontend = OdpFrontend;
    let started = std::time::Instant::now();
    let reader = BytesReader::new(bytes);
    let input = Input::new("presentation.odp", &reader);
    let (result, diagnostics) = with_ctx(&Limits::default(), |ctx| {
        let analysis = frontend.analyze(&input, ctx)?;
        let facts = analysis.facts();
        frontend
            .lower(
                &input,
                ctx,
                &LowerOptions {
                    facts: &facts,
                    only: None,
                    native_motion,
                },
            )
            .map(|_| ())
    });
    let refused = match result {
        Ok(()) => false,
        Err(FrontendError::Refused(_)) => true,
        Err(other) => return (false, Some(format!("{other:?}"))),
    };
    if let Some(crashed) = diagnostics.iter().find(|d| d.code.code() == "OD9005") {
        return (
            refused,
            Some(format!(
                "OD9005 on {:?}: {:?}",
                crashed.container, crashed.data
            )),
        );
    }
    let elapsed = started.elapsed().as_millis();
    (
        refused,
        (elapsed > PER_INPUT_MS).then(|| format!("took {elapsed} ms")),
    )
}

/// A package rebuilt with its `mimetype` first and stored, as ODF requires.
fn repack(parts: &[(String, Vec<u8>)]) -> Vec<u8> {
    zip_entries(parts)
}

#[test]
fn survives_structure_blind_mutations_of_every_fixture() {
    let bases: Vec<Vec<u8>> = fixtures::all()
        .into_iter()
        .map(|(_, build)| build())
        .collect();
    let mut random = Random(0x0d9);
    let mut failures = Vec::new();
    for i in 0..ITERATIONS {
        let mut bytes = bases[i % bases.len()].clone();
        for _ in 0..1 + random.below(3) {
            bytes = mutate(&bytes, &mut random);
        }
        if let (_, Some(failure)) = outcome(&bytes) {
            failures.push(format!("iteration {i}: {failure}"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn survives_mutations_inside_content_xml_and_styles_xml() {
    let bases: Vec<Vec<(String, Vec<u8>)>> = [fixtures::text(), fixtures::shapes()]
        .iter()
        .map(|b| unzip(b))
        .collect();
    let mut random = Random(0x57e);
    let mut failures = Vec::new();
    for i in 0..ITERATIONS {
        let mut parts = bases[i % bases.len()].clone();
        let part = if i % 4 < 3 {
            "content.xml"
        } else {
            "styles.xml"
        };
        let original = parts
            .iter()
            .find(|(name, _)| name == part)
            .unwrap()
            .1
            .clone();
        replace(&mut parts, part, mutate(&original, &mut random));
        if let (_, Some(failure)) = outcome(&repack(&parts)) {
            failures.push(format!("{part}: {failure}"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn survives_structure_aware_mutations_most_of_them_reaching_the_readers() {
    let bases: Vec<Vec<(String, Vec<u8>)>> = fixtures::all()
        .into_iter()
        .map(|(_, build)| unzip(&build()))
        .collect();
    let mut random = Random(0x0d5a);
    let mut failures = Vec::new();
    let mut lowered = 0;
    for i in 0..ITERATIONS {
        let mut parts = bases[i % bases.len()].clone();
        let part = if random.next() < 0.75 {
            "content.xml"
        } else {
            "styles.xml"
        };
        let original = parts
            .iter()
            .find(|(name, _)| name == part)
            .unwrap()
            .1
            .clone();
        replace(&mut parts, part, mutate_xml(&original, &mut random));
        let (refused, failure) = outcome(&repack(&parts));
        if let Some(failure) = failure {
            failures.push(format!("iteration {i}, {part}: {failure}"));
        }
        if !refused {
            lowered += 1;
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
    // A floor, so most mutations reach the readers rather than stopping at a refusal.
    assert!(lowered >= ITERATIONS / 2, "{lowered} lowered");
}

#[test]
fn reads_object_member_names_as_unknown_values() {
    for value in ["constructor", "toString", "__proto__"] {
        let body = format!(
            concat!(
                r#"<draw:frame presentation:class="{v}" svg:x="1cm" svg:y="1cm" svg:width="5cm" svg:height="2cm">"#,
                r#"<draw:text-box><text:p text:style-name="P1">x</text:p></draw:text-box></draw:frame>"#,
                r#"<draw:ellipse draw:kind="{v}" draw:style-name="gr1" svg:x="1cm" svg:y="4cm" svg:width="2cm" svg:height="2cm"/>"#
            ),
            v = value
        );
        let automatic = format!(
            concat!(
                r#"<style:style style:name="P1" style:family="paragraph"><style:paragraph-properties fo:text-align="{v}"/></style:style>"#,
                r#"<style:style style:name="gr1" style:family="graphic"><style:graphic-properties svg:stroke-linecap="{v}" draw:stroke-linejoin="{v}"/></style:style>"#
            ),
            v = value
        );
        let out = compile(&odp(&OdpSpec {
            automatic_styles: automatic,
            ..OdpSpec::pages(vec![PageSpec::body(body), PageSpec::new()])
        }));
        assert_eq!(out.errors(), Vec::<&str>::new(), "{value}");
        let presentation = out.presentation.unwrap();
        let slides = support::js(&presentation)["slides"].clone();
        assert!(
            slides
                .as_array()
                .unwrap()
                .iter()
                .all(|slide| slide.get("failed").is_none()),
            "{value}"
        );
    }
}

#[test]
fn refuses_content_holding_an_entity_declaration() {
    let mut parts = files(&OdpSpec::pages(vec![PageSpec::new()]));
    replace(
        &mut parts,
        "content.xml",
        r#"<?xml version="1.0"?><!DOCTYPE x [<!ENTITY a "aaaaaaaaaa"><!ENTITY b "&a;&a;&a;&a;">]><office:document-content/>"#,
    );
    let bytes = repack(&parts);
    assert_eq!(outcome(&bytes), (true, None));
    let refusal = support::refusal(&bytes);
    assert_eq!(refusal.code.code(), "OD9001");
    assert!(refusal.message.contains("doctype"), "{}", refusal.message);
}

#[test]
fn cuts_style_chains_that_loop_and_groups_nested_past_the_limit_fail_their_slide_alone() {
    let looped = concat!(
        r#"<style:style style:name="a" style:family="graphic" style:parent-style-name="b"/>"#,
        r#"<style:style style:name="b" style:family="graphic" style:parent-style-name="a"/>"#
    );
    let deep = format!(
        "{}<draw:rect svg:width=\"1cm\" svg:height=\"1cm\"/>{}",
        "<draw:g>".repeat(20),
        "</draw:g>".repeat(20)
    );
    let bytes = odp(&OdpSpec {
        styles: looped.to_owned(),
        ..OdpSpec::pages(vec![
            PageSpec::body(r#"<draw:rect draw:style-name="a" svg:width="1cm" svg:height="1cm"/>"#),
            PageSpec::body(deep),
        ])
    });
    let lowered = lower(&bytes);
    let cut = lowered
        .diagnostics
        .iter()
        .find(|d| d.code.code() == "OD9006")
        .unwrap();
    let data = support::js(&cut.data);
    assert_eq!(
        (data["part"].clone(), data["reason"].clone()),
        (json!("styles.xml"), json!("cycle"))
    );
    let failed: Vec<Option<String>> = lowered
        .presentation
        .slides
        .iter()
        .map(|s| s.failed.as_ref().map(|f| f.code.as_str().to_owned()))
        .collect();
    assert_eq!(failed, [None, Some("OD9901".to_owned())]);
}

#[test]
fn resolves_no_reference_outside_the_package() {
    let body = concat!(
        r#"<draw:frame svg:width="1cm" svg:height="1cm"><draw:image xlink:href="../../etc/passwd"/></draw:frame>"#,
        r#"<draw:frame svg:width="1cm" svg:height="1cm"><draw:image xlink:href="/etc/passwd"/></draw:frame>"#
    );
    let lowered = lower(&odp(&OdpSpec::pages(vec![PageSpec::body(body)])));
    assert_eq!(lowered.json["media"], json!({}));
    assert_eq!(
        lowered
            .diagnostics
            .iter()
            .filter(|d| d.code.code() == "OD9303")
            .count(),
        2
    );
}

#[test]
fn fails_the_part_whose_declared_size_exceeds_the_limit() {
    let mut parts = files(&OdpSpec::pages(vec![PageSpec::new()]));
    replace(&mut parts, "styles.xml", vec![0u8; 4 * 1024 * 1024]);
    let mut limits = Limits::default();
    limits.zip.entry_bytes = 1024 * 1024;
    let lowered = lower_with(&repack(&parts), None, limits);
    assert_eq!(
        lowered.json["layouts"]["od-m.Default"]["failed"],
        json!({ "code": "OD9003" })
    );
}

/// How deep the hostile nestings go: past any real document, inside the XML depth limit.
const DEEP: usize = 240;

fn deep_frame(text: &str) -> String {
    format!(
        r#"<draw:frame svg:width="1cm" svg:height="1cm"><draw:text-box>{text}</draw:text-box></draw:frame>"#
    )
}

#[test]
fn survives_links_nested_as_deep_as_the_xml_limit() {
    let links = format!(
        "{}{}{}",
        r#"<draw:a xlink:href="https://example.com">"#.repeat(DEEP),
        deep_frame("<text:p>x</text:p>"),
        "</draw:a>".repeat(DEEP)
    );
    assert_eq!(
        outcome(&odp(&OdpSpec::pages(vec![PageSpec::body(links)]))),
        (false, None)
    );
}

#[test]
fn survives_spans_nested_as_deep_as_the_xml_limit() {
    let spans = deep_frame(&format!(
        "<text:p>{}x{}</text:p>",
        r#"<text:span text:style-name="T">"#.repeat(DEEP),
        "</text:span>".repeat(DEEP)
    ));
    assert_eq!(
        outcome(&odp(&OdpSpec::pages(vec![PageSpec::body(spans)]))),
        (false, None)
    );
}

#[test]
fn survives_lists_nested_as_deep_as_the_xml_limit() {
    let lists = deep_frame(&format!(
        "{}<text:p>x</text:p>{}",
        "<text:list><text:list-item>".repeat(DEEP / 2),
        "</text:list-item></text:list>".repeat(DEEP / 2)
    ));
    assert_eq!(
        outcome(&odp(&OdpSpec::pages(vec![PageSpec::body(lists)]))),
        (false, None)
    );
}

/// Timing values a hostile timing tree writes.
const TIMING_EXTREMES: [&str; 14] = [
    "NaN",
    "-1s",
    "1e308s",
    "4294967296s",
    "99999999999999999999",
    "indefinite",
    "next",
    "s1.end",
    "-PT5S",
    "",
    "0-width/0",
    "((((((((((((((((((((((((((((((((((((x",
    "by-nothing",
    "M 0 0 C",
];

#[test]
fn native_motion_survives_hostile_timing_values_in_the_motion_probe() {
    let probe = include_bytes!("../../../probes/out/motion-probe.odp");
    let parts = unzip(probe);
    let content = parts
        .iter()
        .find(|(path, _)| path == "content.xml")
        .unwrap()
        .1
        .clone();
    let text = String::from_utf8(content).unwrap();
    // The values of the timing attributes: SMIL, animation and presentation timing ones.
    let values: Vec<(usize, usize)> = text
        .match_indices("=\"")
        .filter(|(at, _)| {
            let name = text[..*at].rsplit(' ').next().unwrap_or("");
            name.starts_with("smil:")
                || name.starts_with("anim:")
                || name.starts_with("svg:path")
                || name.starts_with("presentation:")
        })
        .filter_map(|(at, _)| {
            let start = at + 2;
            text[start..].find('"').map(|len| (start, start + len))
        })
        .collect();
    let mut random = Random(0x7e1);
    let mut failures = Vec::new();
    for i in 0..24 {
        let mut mutated = text.clone();
        let mut picks: Vec<(usize, usize)> = (0..40)
            .map(|_| values[random.below(values.len())])
            .collect();
        picks.sort_unstable();
        picks.dedup();
        for (start, end) in picks.into_iter().rev() {
            mutated.replace_range(
                start..end,
                TIMING_EXTREMES[random.below(TIMING_EXTREMES.len())],
            );
        }
        let mut files = parts.clone();
        replace(&mut files, "content.xml", mutated.into_bytes());
        let bytes = repack(&files);
        if let (_, Some(failure)) = outcome_by(true, &bytes) {
            failures.push(format!("iteration {i}: {failure}"));
            continue;
        }
        // Whatever the values, the IR verifies: every one is bounded before it reaches it.
        let lowered = support::lower_motion(&bytes);
        let issues = slidock_ir::verify::verify_source(&lowered.presentation, &Default::default());
        if !issues.is_empty() {
            failures.push(format!(
                "iteration {i}: {:?}",
                issues.iter().map(|d| &d.message).collect::<Vec<_>>()
            ));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
