#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

use super::*;
use crate::google::google_catalog;

const N: FaceStyle = FaceStyle::Normal;
const I: FaceStyle = FaceStyle::Italic;

fn face(weight: u32, style: FaceStyle) -> FaceRequest {
    FaceRequest { weight, style }
}

fn uses(family: &str, faces: &[(u32, FaceStyle)]) -> FontUse {
    FontUse {
        family: family.to_owned(),
        faces: faces.iter().map(|&(w, s)| face(w, s)).collect(),
        postscript_names: Vec::new(),
    }
}

fn candidate(source: SourceKind, family: &str, weight: u32, style: FaceStyle) -> Candidate {
    Candidate {
        source,
        family: family.to_owned(),
        legacy_family: None,
        postscript_name: format!("{}-{weight}", family.replace(' ', "")),
        weight: WeightRange::point(weight),
        style,
        declared: Vec::new(),
    }
}

/// An embedded face, checked against the face the presentation declares it to be.
fn embedded(
    file_family: &str,
    weight: u32,
    style: FaceStyle,
    declared: (&str, u32, FaceStyle),
) -> Candidate {
    let mut face = candidate(SourceKind::Embedded, file_family, weight, style);
    declare(
        &mut face,
        [Declared {
            family: declared.0.to_owned(),
            weight: declared.1,
            style: declared.2,
        }],
    );
    face
}

/// Per slot of the one family: `(drawn weight, style, filling candidate or None, its source)`.
fn fills(resolved: &[ResolvedFamily]) -> Vec<(u32, FaceStyle, Option<usize>, Option<SourceKind>)> {
    resolved[0]
        .slots
        .iter()
        .map(|slot| match &slot.fill {
            Fill::Exact { candidate, source } => {
                (slot.weight, slot.style, Some(*candidate), Some(*source))
            }
            Fill::Demoted { candidate, .. } => (slot.weight, slot.style, Some(*candidate), None),
            Fill::Empty => (slot.weight, slot.style, None, None),
        })
        .collect()
}

#[test]
fn a_slot_takes_the_first_source_in_the_order_that_has_the_face() {
    let candidates = [
        candidate(SourceKind::UserDropped, "Lato", 400, N),
        candidate(SourceKind::Google, "Lato", 400, N),
        candidate(SourceKind::ViewerInstalled, "Lato", 400, N),
        embedded("Lato", 400, N, ("Lato", 400, N)),
    ];
    let request = [uses("Lato", &[(400, N)])];
    // (order, the candidate that fills the slot)
    let table: [(Vec<SourceKind>, Option<usize>); 6] = [
        (SourceKind::ALL.to_vec(), Some(3)),
        (
            vec![SourceKind::ViewerInstalled, SourceKind::Google],
            Some(2),
        ),
        (vec![SourceKind::Google, SourceKind::Embedded], Some(1)),
        (vec![SourceKind::UserDropped, SourceKind::Embedded], Some(0)),
        (vec![SourceKind::UserDropped], Some(0)),
        (Vec::new(), None),
    ];
    for (order, expected) in table {
        let resolved = resolve(&request, &candidates, &SourceOrder::new(order.clone()));
        assert_eq!(fills(&resolved)[0].2, expected, "{order:?}");
    }
}

#[test]
fn a_family_mixes_sources_and_later_sources_fill_only_empty_slots() {
    let candidates = [
        candidate(SourceKind::Google, "Lato", 400, I),
        candidate(SourceKind::Google, "Lato", 400, N),
        embedded("Lato", 400, N, ("Lato", 400, N)),
        embedded("Lato", 700, N, ("Lato", 700, N)),
    ];
    let resolved = resolve(
        &[uses("Lato", &[(700, N), (400, I), (400, N), (400, N)])],
        &candidates,
        &SourceOrder::default(),
    );
    assert_eq!(
        fills(&resolved),
        vec![
            (400, N, Some(2), Some(SourceKind::Embedded)),
            (400, I, Some(0), Some(SourceKind::Google)),
            (700, N, Some(3), Some(SourceKind::Embedded)),
        ]
    );
}

#[test]
fn a_slot_is_filled_only_by_an_exact_face() {
    // A Black file in a bold slot passes the identity check but is not the bold face: it stands in until a source
    // has 700.
    let black = embedded("Barlow Condensed", 900, N, ("Barlow Condensed", 700, N));
    assert!(black.declared[0].1.is_accepted());
    let request = [uses("Barlow Condensed", &[(700, N)])];
    let resolved = resolve(
        &request,
        std::slice::from_ref(&black),
        &SourceOrder::default(),
    );
    assert_eq!(
        resolved[0].slots[0].fill,
        Fill::Demoted {
            candidate: 0,
            mismatch: None
        }
    );
    let with_google = [
        black,
        candidate(SourceKind::Google, "Barlow Condensed", 700, N),
    ];
    let resolved = resolve(&request, &with_google, &SourceOrder::default());
    assert_eq!(
        fills(&resolved),
        vec![(700, N, Some(1), Some(SourceKind::Google))]
    );
}

#[test]
fn a_regular_file_for_a_medium_request_is_demoted_and_google_is_planned() {
    let regular = embedded("Roboto", 400, N, ("Roboto Medium", 400, N));
    assert!(regular.is_demoted());
    let request = [uses("Roboto Medium", &[(400, N)])];
    let resolved = resolve(
        &request,
        std::slice::from_ref(&regular),
        &SourceOrder::default(),
    );
    let slot = &resolved[0].slots[0];
    assert_eq!(slot.weight, 500, "the family name's weight is drawn");
    assert_eq!(
        slot.fill,
        Fill::Demoted {
            candidate: 0,
            mismatch: Some(Mismatch::Weight { file_weight: 400 })
        }
    );
    let needs = font_needs(&resolved, &SourceOrder::default(), google_catalog());
    let plan = needs[0].google.as_ref().unwrap();
    assert_eq!(plan.family, "Roboto");
    assert_eq!(plan.faces, vec![(500, N)]);
    assert!(plan.url.contains("family=Roboto:wght@500&"), "{}", plan.url);
    // Google's Medium then fills the slot exactly.
    let fetched = [regular, candidate(SourceKind::Google, "Roboto", 500, N)];
    let resolved = resolve(&request, &fetched, &SourceOrder::default());
    assert_eq!(
        fills(&resolved),
        vec![(500, N, Some(1), Some(SourceKind::Google))]
    );
    assert!(font_needs(&resolved, &SourceOrder::default(), google_catalog()).is_empty());
}

#[test]
fn a_demoted_face_comes_after_every_source_even_where_it_is_exact() {
    // The Medium request's file is genuine Roboto Regular: it is exact for a Roboto 400 slot, but after Google.
    let request = [uses("Roboto", &[(400, N)])];
    let google = candidate(SourceKind::Google, "Roboto", 400, N);
    let demoted = [
        embedded("Roboto", 400, N, ("Roboto Medium", 400, N)),
        google.clone(),
    ];
    let resolved = resolve(&request, &demoted, &SourceOrder::default());
    assert_eq!(
        fills(&resolved),
        vec![(400, N, Some(1), Some(SourceKind::Google))]
    );
    // Negative control: the same file declared as Roboto Regular is accepted, and embedded comes first.
    let accepted = [embedded("Roboto", 400, N, ("Roboto", 400, N)), google];
    let resolved = resolve(&request, &accepted, &SourceOrder::default());
    assert_eq!(
        fills(&resolved),
        vec![(400, N, Some(0), Some(SourceKind::Embedded))]
    );
    // Alone, the demoted file still fills the slot rather than leave it to a substitute.
    let resolved = resolve(&request, &demoted[..1], &SourceOrder::default());
    assert_eq!(
        fills(&resolved),
        vec![(400, N, Some(0), Some(SourceKind::Embedded))]
    );
}

#[test]
fn a_weight_named_family_resolves_to_the_split_family_at_that_weight() {
    let mut variable = candidate(SourceKind::Google, "Oswald", 400, N);
    variable.weight = WeightRange { min: 200, max: 700 };
    let request = [uses("Oswald Medium", &[(400, N), (700, N)])];
    let resolved = resolve(&request, &[variable], &SourceOrder::default());
    assert_eq!(resolved[0].name_weight, Some(500));
    assert_eq!(
        fills(&resolved),
        vec![
            (500, N, Some(0), Some(SourceKind::Google)),
            (700, N, Some(0), Some(SourceKind::Google)),
        ]
    );
    // An installed face named by its legacy family ("Roboto Medium" for Roboto-Medium.ttf) is the Medium face.
    let mut installed = candidate(SourceKind::ViewerInstalled, "Roboto", 500, N);
    installed.legacy_family = Some("Roboto Medium".into());
    let resolved = resolve(
        &[uses("Roboto Medium", &[(400, N)])],
        &[installed],
        &SourceOrder::default(),
    );
    assert_eq!(fills(&resolved)[0].3, Some(SourceKind::ViewerInstalled));
}

#[test]
fn the_catalog_takes_the_earliest_source_last_and_demoted_faces_first() {
    let candidates = [
        candidate(SourceKind::Google, "Lato", 400, N),
        embedded("Lato", 400, N, ("Lato", 400, N)),
        candidate(SourceKind::UserDropped, "Lato", 400, N),
        embedded("Roboto", 400, N, ("Roboto Medium", 400, N)),
        candidate(SourceKind::ViewerInstalled, "Off", 400, N),
    ];
    let order = SourceOrder::default().without(SourceKind::ViewerInstalled);
    let resolved = resolve(&[uses("Lato", &[(400, N)])], &candidates, &order);
    let entries: Vec<usize> = catalog_entries(&resolved, &candidates, &order)
        .iter()
        .map(|entry| entry.candidate)
        .collect();
    assert_eq!(entries, vec![3, 2, 0, 1]);
}

#[test]
fn a_demoted_face_stands_in_under_the_declared_name_when_no_source_has_the_family() {
    let candidates = [embedded("BrandSans Text", 400, N, ("Brand Sans", 400, N))];
    let order = SourceOrder::default();
    let resolved = resolve(&[uses("Brand Sans", &[(400, N)])], &candidates, &order);
    let entries = catalog_entries(&resolved, &candidates, &order);
    assert_eq!(
        entries,
        vec![
            CatalogEntry {
                candidate: 0,
                family: "BrandSans Text".into()
            },
            CatalogEntry {
                candidate: 0,
                family: "Brand Sans".into()
            },
        ]
    );
    // Negative control: once a source has the family, the demoted file is not aliased.
    let with_family = [
        candidates[0].clone(),
        candidate(SourceKind::UserDropped, "Brand Sans", 700, N),
    ];
    let resolved = resolve(&[uses("Brand Sans", &[(400, N)])], &with_family, &order);
    assert!(
        catalog_entries(&resolved, &with_family, &order)
            .iter()
            .all(|entry| entry.family != "Brand Sans" || entry.candidate == 1)
    );
}

#[test]
fn needs_plan_google_only_for_catalog_families_and_slots_no_earlier_source_has() {
    let request = [
        uses("Helvetica Neue", &[(400, N)]),
        uses("Calibri", &[(400, N), (700, I)]),
        uses("Roboto", &[(400, N), (700, N)]),
    ];
    let candidates = [embedded("Roboto", 400, N, ("Roboto", 400, N))];
    let resolved = resolve(&request, &candidates, &SourceOrder::default());
    let needs = font_needs(&resolved, &SourceOrder::default(), google_catalog());
    let plans: Vec<(&str, Option<&str>)> = needs
        .iter()
        .map(|need| {
            (
                need.family.as_str(),
                need.google.as_ref().map(|plan| plan.url.as_str()),
            )
        })
        .collect();
    assert_eq!(
        plans,
        vec![
            ("Helvetica Neue", None),
            ("Calibri", None),
            (
                "Roboto",
                Some("https://fonts.googleapis.com/css2?family=Roboto:wght@700&display=block")
            ),
        ]
    );
    let local = needs[2].local.as_ref().unwrap();
    assert_eq!(local.families, vec![FamilyKey::of("Roboto")]);
    // With Google off, nothing is planned; with every source off, nothing is queried.
    let no_google = SourceOrder::default().without(SourceKind::Google);
    assert!(
        font_needs(&resolved, &no_google, google_catalog())
            .iter()
            .all(|n| n.google.is_none())
    );
    let off = SourceOrder::off();
    let resolved = resolve(&request, &candidates, &off);
    assert!(
        font_needs(&resolved, &off, google_catalog())
            .iter()
            .all(|n| n.google.is_none() && n.local.is_none())
    );
}

#[test]
fn local_selection_reads_exact_postscript_names_then_the_open_slots_of_a_family() {
    let meta = |postscript: &str, family: &str| LocalFontMeta {
        postscript_name: postscript.to_owned(),
        full_name: postscript.to_owned(),
        family: family.to_owned(),
        style: "Regular".to_owned(),
    };
    let metas = [
        meta("Roboto-Regular", "Roboto"),
        meta("Roboto-Medium", "Roboto Medium"),
        meta("Roboto-Bold", "Roboto"),
        meta("HelveticaNeue-Medium", "Helvetica Neue"),
        meta("Arial-BoldMT", "Arial"),
    ];
    let mut keynote = uses("Helvetica Neue", &[(500, N)]);
    keynote.postscript_names = vec!["HelveticaNeue-Medium".into()];
    let request = [keynote, uses("Roboto Medium", &[(400, N)])];
    let resolved = resolve(&request, &[], &SourceOrder::default());
    let needs = font_needs(&resolved, &SourceOrder::default(), google_catalog());
    let picks: Vec<(usize, bool)> = select_local(&metas, &needs)
        .iter()
        .map(|pick| (pick.index, pick.exact_postscript))
        .collect();
    assert_eq!(picks, vec![(3, true), (1, false)]);
}

#[test]
fn of_one_source_s_equal_faces_the_last_added_wins() {
    let candidates = [
        candidate(SourceKind::UserDropped, "Lato", 400, N),
        candidate(SourceKind::UserDropped, "Lato", 400, N),
    ];
    let order = SourceOrder::default();
    let resolved = resolve(&[uses("Lato", &[(400, N)])], &candidates, &order);
    assert_eq!(fills(&resolved)[0].2, Some(1));
    let last = catalog_entries(&resolved, &candidates, &order)
        .last()
        .unwrap()
        .candidate;
    assert_eq!(last, 1);
}

#[test]
fn a_subset_file_that_lacks_drawn_code_points_stands_in_only_after_every_source() {
    let mut subset = embedded("Roboto", 400, N, ("Roboto", 400, N));
    require_coverage(&mut subset, &[3]);
    assert_eq!(
        subset.declared[0].1,
        Identity::Demoted(Mismatch::Subset { missing: 3 })
    );
    let candidates = [subset, candidate(SourceKind::Google, "Roboto", 400, N)];
    let resolved = resolve(
        &[uses("Roboto", &[(400, N)])],
        &candidates,
        &SourceOrder::default(),
    );
    assert_eq!(
        fills(&resolved),
        [(400, N, Some(1), Some(SourceKind::Google))]
    );
    // With no other source, the subset file fills its slot.
    let resolved = resolve(
        &[uses("Roboto", &[(400, N)])],
        &candidates[..1],
        &SourceOrder::default(),
    );
    assert_eq!(
        fills(&resolved),
        [(400, N, Some(0), Some(SourceKind::Embedded))]
    );

    // Negative control: a subset file that covers every drawn code point is accepted and comes first.
    let mut covering = embedded("Roboto", 400, N, ("Roboto", 400, N));
    require_coverage(&mut covering, &[0]);
    assert!(covering.declared[0].1.is_accepted());
    let candidates = [covering, candidate(SourceKind::Google, "Roboto", 400, N)];
    let resolved = resolve(
        &[uses("Roboto", &[(400, N)])],
        &candidates,
        &SourceOrder::default(),
    );
    assert_eq!(
        fills(&resolved),
        [(400, N, Some(0), Some(SourceKind::Embedded))]
    );
    // A face that already failed its identity check keeps that mismatch.
    let mut medium = embedded("Roboto", 400, N, ("Roboto Medium", 400, N));
    require_coverage(&mut medium, &[5]);
    assert_eq!(
        medium.declared[0].1,
        Identity::Demoted(Mismatch::Weight { file_weight: 400 })
    );
}

#[test]
fn installed_files_are_read_within_a_count_and_byte_budget() {
    const MIB: u64 = 1024 * 1024;
    let mut budget = LocalBudget::default();
    assert_eq!(
        budget.check(49 * MIB, false),
        Err(LocalRefusal::FileTooLarge { bytes: 49 * MIB })
    );
    // An exact PostScript hit may be larger than one file's cap, never larger than the session's.
    budget.spend(60 * MIB, true).unwrap();
    assert_eq!(
        budget.spend(40 * MIB, false),
        Err(LocalRefusal::OverBudget {
            bytes: 40 * MIB,
            left: 36 * MIB
        })
    );
    budget.spend(36 * MIB, false).unwrap();
    assert!(matches!(
        budget.check(1, true),
        Err(LocalRefusal::OverBudget { .. })
    ));
    let mut count = LocalBudget::default();
    for _ in 0..MAX_LOCAL_PICKS {
        count.spend(1, false).unwrap();
    }
    assert_eq!(count.check(1, false), Err(LocalRefusal::TooManyFiles));
}
