//! The font probe deck LibreOffice wrote with its fonts embedded (`probes/out/fonts-probe.odp`, built by
//! `probes/fonts/build.sh`): every `Fonts/` file is handed on once with the faces its declarations name, and a session
//! that takes embedded fonts reads them, accepts the faces that prove their identity and demotes the rest.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use std::collections::BTreeMap;

use slidock_core::{FontIssue, Session, SessionOptions};
use slidock_core_api::frontend::FontContainer;
use slidock_core_api::{Input, MediaSource};
use slidock_fonts::FamilyKey;
use slidock_fonts::resolve::{Fill, Mismatch, SourceKind};
use slidock_io::BytesReader;
use slidock_ir::fonts::FaceStyle;
use support::{lower, setup};

fn probe() -> Vec<u8> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../probes/out/fonts-probe.odp"
    );
    std::fs::read(path).unwrap()
}

type Faces = Vec<(u32, bool)>;

#[test]
fn hands_on_each_embedded_file_once_with_every_face_declared_for_it() {
    let fonts = lower(&probe()).lowered.fonts;
    let mut by_family: BTreeMap<&str, Vec<Faces>> = BTreeMap::new();
    for font in &fonts {
        assert_eq!(font.container, FontContainer::Sfnt);
        assert!(
            matches!(font.source, MediaSource::Reader(_)),
            "{:?}",
            font.source
        );
        let faces = font
            .faces
            .iter()
            .map(|face| (face.weight, face.style == FaceStyle::Italic))
            .collect();
        by_family
            .entry(font.family.as_str())
            .or_default()
            .push(faces);
    }
    let (regular, bold, italic, bold_italic) =
        ((400, false), (700, false), (400, true), (700, true));
    assert_eq!(
        by_family,
        BTreeMap::from([
            // LibreOffice declares one file for the missing italic.
            ("Barlow Condensed", vec![vec![regular], vec![bold, italic]]),
            // One variable file for three slots.
            ("Caveat", vec![vec![regular, bold, italic]]),
            (
                "Lato",
                vec![vec![regular], vec![bold], vec![italic], vec![bold_italic]]
            ),
            (
                "PT Serif",
                vec![vec![regular], vec![bold], vec![italic], vec![bold_italic]]
            ),
        ])
    );
}

#[test]
fn a_session_reads_the_probe_faces_and_demotes_the_ones_declared_for_a_face_they_are_not() {
    let bytes = probe();
    let (registry, pipeline) = setup();
    let reader = BytesReader::new(bytes.as_slice());
    let mut session = Session::open(
        &Input::new("fonts-probe.odp", &reader),
        SessionOptions {
            embedded_fonts: true,
            ..SessionOptions::new(&registry, &pipeline)
        },
    )
    .unwrap();
    let added = session.added_fonts();
    assert_eq!(added.len(), 11);
    let mut families: Vec<String> = added
        .iter()
        .map(|font| format!("{} {}", font.faces[0].family, font.faces[0].weight))
        .collect();
    families.sort();
    assert_eq!(
        families,
        [
            "Barlow Condensed 400",
            "Barlow Condensed 700",
            "Caveat 400",
            "Lato 400",
            "Lato 400",
            "Lato 700",
            "Lato 700",
            "PT Serif 400",
            "PT Serif 400",
            "PT Serif 700",
            "PT Serif 700",
        ]
    );
    // An upright file declared for the italic slot is demoted; the variable Caveat proves its bold.
    let issues = session.font_issues();
    let mislabeled: Vec<(&str, u32, FaceStyle, &Mismatch)> = issues
        .iter()
        .filter_map(|issue| match issue {
            FontIssue::EmbeddedMislabeled {
                family,
                weight,
                style,
                mismatch,
                ..
            } => Some((family.as_str(), *weight, *style, mismatch)),
            _ => None,
        })
        .collect();
    let upright = Mismatch::Style { file_italic: false };
    assert_eq!(
        mislabeled,
        vec![
            ("Barlow Condensed", 400, FaceStyle::Italic, &upright),
            ("Caveat", 400, FaceStyle::Italic, &upright),
        ]
    );
    assert_eq!(issues.len(), 2, "{issues:?}");
    assert_eq!(
        issues[1].to_string(),
        "Caveat 400 italic: the file is Caveat 400 (upright, for an italic face); used only if no source has the face"
    );
    // Lato's four embedded faces fill their slots; LibreOffice embeds no file for the Light (read as 350) and Black
    // runs, which stay open for a later source.
    let needs = session.font_needs();
    let lato = needs
        .iter()
        .find(|need| need.key == FamilyKey::of("Lato"))
        .unwrap();
    let filled: Vec<(u32, FaceStyle)> = lato
        .slots
        .iter()
        .filter(|slot| slot.fill.is_exact())
        .map(|slot| (slot.weight, slot.style))
        .collect();
    assert_eq!(
        filled,
        [
            (400, FaceStyle::Normal),
            (400, FaceStyle::Italic),
            (700, FaceStyle::Normal),
            (700, FaceStyle::Italic)
        ]
    );
    assert!(lato.slots.iter().all(|slot| match slot.fill {
        Fill::Exact { source, .. } => source == SourceKind::Embedded,
        _ => true,
    }));
    let open: Vec<u32> = lato.faces.iter().map(|face| face.weight).collect();
    assert_eq!(open, [350, 900]);
    let finished = session.finish().unwrap();
    assert!(finished.presentation.is_some());
}
