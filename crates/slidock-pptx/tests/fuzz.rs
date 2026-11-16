//! Hostile packages: whatever the bytes, the frontend refuses the input or lowers it with diagnostics, never
//! panics, and does bounded work: each input within seconds, groups nested to the limit within the stack a wasm
//! module has.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::disallowed_types,
    reason = "test code; the wall clock bounds the work"
)]

mod support;

use std::borrow::Cow;
use std::time::Instant;

use indexmap::IndexMap;
use slidock_core_api::{
    Deadline, Frontend, FrontendCtx, FrontendError, Input, Limits, LowerOptions, Sink,
};
use slidock_io::{BytesReader, RangeError, RangeReader, ZipLimits};
use slidock_ir::diagnostics::Stage;
use slidock_pptx::PptxFrontend;
use support::fixtures::{FIXTURES, png, shapes, text};
use support::mutate::{iterations, mutate, mutate_xml, seeded};
use support::{
    PptxSpec, SlideSpec, Sp, compile_bytes, external, lower, lower_with, pptx, rel, slide, sp,
    unzip, xfrm, zip,
};

const PER_INPUT_MS: u128 = 5000;

/// What an input did: refused as a whole, or lowered; and what went wrong.
struct Outcome {
    refused: bool,
    failure: Option<String>,
}

fn outcome_with(bytes: &[u8], limits: &Limits) -> Outcome {
    let started = Instant::now();
    let mut refused = false;
    let mut failure = None;
    match lower_with(bytes, None, limits) {
        Err(FrontendError::Refused(_)) => refused = true,
        Err(other) => failure = Some(other.to_string()),
        Ok(lowered) => {
            if let Some(crashed) = lowered
                .diagnostics
                .iter()
                .find(|d| matches!(d.code.code(), "PX2007" | "IR0906"))
            {
                let reason = crashed
                    .data
                    .as_ref()
                    .and_then(|data| data.get("reason"))
                    .map(|reason| reason.to_js_string())
                    .unwrap_or_default();
                failure = Some(format!(
                    "{} on {:?}: {reason}",
                    crashed.code.code(),
                    crashed.container
                ));
            }
        }
    }
    let elapsed = started.elapsed().as_millis();
    if failure.is_none() && elapsed > PER_INPUT_MS {
        failure = Some(format!("took {elapsed} ms"));
    }
    Outcome { refused, failure }
}

fn survive(bytes: &[u8]) -> Option<String> {
    outcome_with(bytes, &Limits::default()).failure
}

/// Reads its bytes, except the range of one entry, which fails: a crash inside one container's lowering.
struct FailingAt<'b> {
    bytes: &'b [u8],
    from: u64,
    to: u64,
}

impl RangeReader for FailingAt<'_> {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn read_at(&self, offset: u64, len: usize) -> Result<Cow<'_, [u8]>, RangeError> {
        let end = offset + len as u64;
        if offset < self.to && end > self.from && len < self.bytes.len() {
            return Err(RangeError::Io {
                offset,
                len: len as u64,
                message: "an unexpected crash".to_owned(),
            });
        }
        BytesReader::new(self.bytes)
            .read_at(offset, len)
            .map(|bytes| Cow::Owned(bytes.into_owned()))
    }
}

#[test]
fn fails_the_one_container_whose_lowering_crashed_with_px2007_and_lowers_the_rest() {
    // A STORED media entry the first slide shows, whose bytes the source fails to read.
    let mut files = unzip(&shapes());
    let media = files.shift_remove("ppt/media/image1.png").unwrap();
    files.insert("ppt/media/image1.png".to_owned(), media);
    let names: Vec<String> = files.keys().cloned().collect();
    let stored: IndexMap<String, Vec<u8>> = files;
    let bytes = zip(&stored, 0);
    let reader = BytesReader::new(&bytes[..]);
    let archive = slidock_io::ZipArchive::open(&reader, ZipLimits::default()).unwrap();
    let entry = archive.get("ppt/media/image1.png").unwrap();
    let from = entry.local_header_offset + 30 + entry.name.len() as u64;
    let failing = FailingAt {
        bytes: &bytes,
        from,
        to: from + entry.size,
    };
    assert!(names.contains(&"ppt/media/image1.png".to_owned()));

    let mut out = Vec::new();
    let mut sink = Sink::new(&mut out, Stage::Frontend, "pptx");
    let limits = Limits::default();
    let mut ctx = FrontendCtx {
        limits: &limits,
        diag: &mut sink,
        deadline: Deadline::never(),
        spent: slidock_core_api::Output::default(),
    };
    let input = Input::new("presentation.pptx", &failing);
    let analysis = PptxFrontend.analyze(&input, &mut ctx).unwrap();
    let facts = analysis.facts();
    let lowered = PptxFrontend
        .lower(
            &input,
            &mut ctx,
            &LowerOptions {
                facts: &facts,
                only: None,
                native_motion: false,
            },
        )
        .unwrap();
    let slidock_core_api::Unverified::Typed(presentation) = lowered.presentation else {
        panic!("typed IR");
    };
    let failed: Vec<(String, String)> = presentation
        .slides
        .iter()
        .filter_map(|slide| {
            slide.failed.as_ref().map(|failed| {
                (
                    slide.id.as_str().to_owned(),
                    failed.code.as_str().to_owned(),
                )
            })
        })
        .collect();
    assert_eq!(failed, [("pp-256".to_owned(), "PX2007".to_owned())]);
    assert!(presentation.slides[1].failed.is_none());
    // A PX2007 names a crash, so the fuzz loops below count it as a failure.
    assert!(survive(&shapes()).is_none());
}

#[test]
fn survives_structure_blind_mutations_of_every_fixture() {
    let bases: Vec<Vec<u8>> = FIXTURES.iter().map(|(_, build)| build()).collect();
    let mut random = seeded(0x9077);
    let mut failures = Vec::new();
    for i in 0..iterations() {
        let mut bytes = bases[i % bases.len()].clone();
        let rounds = 1 + (random() * 3.0) as usize;
        for _ in 0..rounds {
            bytes = mutate(&bytes, &mut random);
        }
        if let Some(failure) = survive(&bytes) {
            failures.push(format!("iteration {i}: {failure}"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn survives_mutations_inside_the_xml_of_each_part() {
    let base = unzip(&text());
    let parts: Vec<String> = base
        .keys()
        .filter(|name| name.ends_with(".xml") || name.ends_with(".rels"))
        .cloned()
        .collect();
    let mut random = seeded(0x3a1);
    let mut failures = Vec::new();
    for i in 0..iterations() {
        let part = &parts[i % parts.len()];
        let mut files = base.clone();
        let mutated = mutate(&base[part], &mut random);
        files.insert(part.clone(), mutated);
        if let Some(failure) = survive(&zip(&files, 6)) {
            failures.push(format!("{part}: {failure}"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn survives_structure_aware_mutations_of_every_xml_part_most_of_them_reaching_the_readers() {
    let bases: Vec<IndexMap<String, Vec<u8>>> =
        FIXTURES.iter().map(|(_, build)| unzip(&build())).collect();
    let mut random = seeded(0x51ab);
    let mut failures = Vec::new();
    let mut lowered = 0;
    let total = iterations();
    for i in 0..total {
        let base = &bases[i % bases.len()];
        let parts: Vec<&String> = base.keys().filter(|name| name.ends_with(".xml")).collect();
        let part = parts[((random() * parts.len() as f64) as usize).min(parts.len() - 1)].clone();
        let mut files = base.clone();
        let mutated = mutate_xml(&base[&part], &mut random, 3);
        files.insert(part.clone(), mutated);
        let result = outcome_with(&zip(&files, 6), &Limits::default());
        if let Some(failure) = result.failure {
            failures.push(format!("iteration {i}, {part}: {failure}"));
        }
        if !result.refused {
            lowered += 1;
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
    // A floor, so most mutations reach the readers rather than stopping at a refusal.
    assert!(lowered >= total / 2, "{lowered} of {total} lowered");
}

#[test]
fn reads_object_member_names_as_attribute_values_like_any_unknown_value() {
    for value in ["constructor", "toString", "__proto__", "hasOwnProperty"] {
        let shape = sp(Sp {
            id: 2,
            ph: format!("<p:ph type=\"{value}\" idx=\"1\"/>"),
            sp_pr: format!(
                "{}<a:prstGeom prst=\"rect\"><a:avLst><a:gd name=\"{value}\" fmla=\"val 1\"/></a:avLst></a:prstGeom><a:ln cap=\"{value}\"/>",
                xfrm(10.0, 10.0, 200.0, 50.0, "")
            ),
            tx_body: Some(format!(
                "<a:bodyPr anchor=\"{value}\"/><a:lstStyle/><a:p><a:pPr algn=\"{value}\"/><a:r><a:rPr u=\"{value}\" strike=\"{value}\" cap=\"{value}\"/><a:t>x</a:t></a:r></a:p>"
            )),
            ..Sp::default()
        });
        let compiled = compile_bytes(
            &pptx(PptxSpec {
                slides: vec![slide(shape), slide("")],
                ..PptxSpec::default()
            }),
            "presentation.pptx",
        );
        assert!(
            compiled.errors().is_empty(),
            "{value}: {:?}",
            compiled.errors()
        );
        let failed: Vec<bool> = compiled
            .presentation
            .unwrap()
            .slides
            .iter()
            .map(|slide| slide.body.failed().is_some())
            .collect();
        assert_eq!(failed, [false, false], "{value}");
    }
}

#[test]
fn fails_a_slide_holding_an_entity_declaration_alone() {
    let mut files = unzip(&pptx(PptxSpec {
        slides: vec![SlideSpec::default(), SlideSpec::default()],
        ..PptxSpec::default()
    }));
    files.insert(
        "ppt/slides/slide1.xml".to_owned(),
        b"<?xml version=\"1.0\"?><!DOCTYPE p [<!ENTITY a \"aaaaaaaaaa\"><!ENTITY b \"&a;&a;&a;&a;&a;\">]><p:sld/>"
            .to_vec(),
    );
    let lowered = lower(&zip(&files, 6));
    let failed: Vec<Option<String>> = lowered
        .presentation()
        .slides
        .iter()
        .map(|slide| {
            slide
                .failed
                .as_ref()
                .map(|failed| failed.code.as_str().to_owned())
        })
        .collect();
    assert_eq!(failed, [Some("PX2002".to_owned()), None]);
    let reason = lowered.find("PX2002").unwrap().data.as_ref().unwrap()["reason"].to_js_string();
    assert!(reason.contains("doctype"), "{reason}");
}

#[test]
fn resolves_no_relationship_outside_the_package() {
    let tree = sp(Sp {
        id: 2,
        sp_pr: format!(
            "{}<a:blipFill><a:blip r:embed=\"rIdUp\"/></a:blipFill>",
            xfrm(0.0, 0.0, 10.0, 10.0, "")
        ),
        ..Sp::default()
    });
    let lowered = lower(&pptx(PptxSpec {
        slides: vec![SlideSpec {
            tree,
            rels: vec![rel("rIdUp", "image", "../../../../etc/passwd")],
            ..SlideSpec::default()
        }],
        ..PptxSpec::default()
    }));
    assert!(lowered.presentation().media.is_empty());
    assert!(lowered.find("PX2307").is_some());
}

#[test]
fn never_fetches_an_external_target() {
    let tree = sp(Sp {
        id: 2,
        sp_pr: format!(
            "{}<a:blipFill><a:blip r:embed=\"rIdFar\"/></a:blipFill>",
            xfrm(0.0, 0.0, 10.0, 10.0, "")
        ),
        ..Sp::default()
    });
    let lowered = lower(&pptx(PptxSpec {
        slides: vec![SlideSpec {
            tree,
            rels: vec![external("rIdFar", "image", "file:///etc/passwd")],
            ..SlideSpec::default()
        }],
        parts: vec![("ppt/media/image1.png".to_owned(), png(4, 2))],
        ..PptxSpec::default()
    }));
    assert!(lowered.presentation().media.is_empty());
    assert!(lowered.find("PX2301").is_some());
}

#[test]
fn survives_relationships_that_point_back_at_the_slide_or_loop_between_layouts_and_masters() {
    let mut files = unzip(&pptx(PptxSpec {
        slides: vec![SlideSpec::default()],
        ..PptxSpec::default()
    }));
    let rels = |kind: &str, target: &str| {
        format!(
            "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rIdX\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}\" Target=\"{target}\"/></Relationships>"
        )
        .into_bytes()
    };
    files.insert(
        "ppt/slides/_rels/slide1.xml.rels".to_owned(),
        rels("slideLayout", "slide1.xml"),
    );
    files.insert(
        "ppt/slideLayouts/_rels/slideLayout1.xml.rels".to_owned(),
        rels("slideMaster", "slideLayout1.xml"),
    );
    assert_eq!(survive(&zip(&files, 6)), None);
}

#[test]
fn refuses_a_package_over_the_entry_limit_before_reading_it() {
    let mut files: IndexMap<String, Vec<u8>> = (0..20_001)
        .map(|i| (format!("x/{i}"), Vec::new()))
        .collect();
    files.insert("ppt/presentation.xml".to_owned(), b"<p/>".to_vec());
    let outcome = outcome_with(&zip(&files, 0), &Limits::default());
    assert!(outcome.refused);
    assert_eq!(outcome.failure, None);
}

#[test]
fn fails_the_container_whose_part_declares_more_bytes_than_its_limit() {
    let mut files = unzip(&pptx(PptxSpec {
        slides: vec![SlideSpec::default(), SlideSpec::default()],
        ..PptxSpec::default()
    }));
    files.insert("ppt/slides/slide2.xml".to_owned(), vec![0; 4 * 1024 * 1024]);
    let limits = Limits {
        zip: ZipLimits {
            entry_bytes: 1024 * 1024,
            ..ZipLimits::default()
        },
        ..Limits::default()
    };
    let lowered = lower_with(&zip(&files, 6), None, &limits).unwrap();
    let failed = lowered.presentation().slides[1].failed.as_ref().unwrap();
    assert_eq!(failed.code.as_str(), "PX2002");
}

fn nested(depth: usize) -> String {
    if depth == 0 {
        return sp(Sp {
            id: 900,
            sp_pr: format!(
                "{}<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom>",
                xfrm(0.0, 0.0, 10.0, 10.0, "")
            ),
            tx_body: Some(support::body(&["deep"], "<a:bodyPr/>")),
            ..Sp::default()
        });
    }
    format!(
        "<p:grpSp><p:nvGrpSpPr><p:cNvPr id=\"{depth}\" name=\"g\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>{}</p:grpSp>",
        nested(depth - 1)
    )
}

#[test]
fn lowers_groups_nested_to_the_limit_within_a_wasm_modules_stack() {
    let limits = Limits::default();
    let bytes = pptx(PptxSpec {
        slides: vec![slide(nested(limits.group_depth))],
        ..PptxSpec::default()
    });
    // wasm-ld gives a module 1 MiB of stack; an unoptimised build needs more than an optimised one.
    let failed = std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(move || lower(&bytes).presentation().slides[0].failed.clone())
        .unwrap()
        .join()
        .unwrap();
    assert_eq!(failed, None);
}
