//! The licence policy: which faces ship in the preview and the download, per `fsType` bit and
//! per source, and the `LICENSES.txt` entry of each shipped face.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use slidock_fonts::policy::{Exclusion, LICENCE_TEXTS, licence_id, licences_text, notice_families};
use slidock_fonts::sfnt::read_details;
use slidock_fonts::{
    CodepointSet, Decision, FaceDescriptor, FontOrigin, FsType, LicenceEntry, LicencePolicy,
    ShipInput, ShipOptions, Target, Toggle, WeightRange, decide, ship_face,
};
use slidock_ir::fonts::FaceStyle;
use support::fixtures::*;
use support::*;

use Decision::{Exclude, Ship, ShipWithNotice};
use FontOrigin::{Bundled, Embedded, Google, Local, User};
use Target::{Download, Preview};

fn default_download(origin: FontOrigin, fs_type: u16) -> Decision {
    decide(
        origin,
        FsType(fs_type),
        &LicencePolicy::default(),
        Download,
        Toggle::Default,
    )
}

fn default_preview(origin: FontOrigin, fs_type: u16) -> Decision {
    decide(
        origin,
        FsType(fs_type),
        &LicencePolicy::default(),
        Preview,
        Toggle::Default,
    )
}

#[test]
fn installable_faces_from_open_sources_ship_everywhere() {
    for origin in [Embedded, Google, Bundled] {
        for fs_type in [0x0000, 0x0004, 0x0008, 0x0100] {
            assert_eq!(
                default_download(origin, fs_type),
                Ship,
                "{origin:?} {fs_type:#x}"
            );
            assert_eq!(
                default_preview(origin, fs_type),
                Ship,
                "{origin:?} {fs_type:#x}"
            );
        }
    }
}

#[test]
fn a_restricted_face_stays_out_of_the_download_only() {
    for origin in [Embedded, Local, Google, User, Bundled] {
        assert_eq!(
            default_download(origin, 0x0002),
            Exclude(Exclusion::Restricted),
            "{origin:?}"
        );
        assert_eq!(default_preview(origin, 0x0002), Ship, "{origin:?}");
    }
    // A more permissive bit beside the restricted one wins.
    assert_eq!(default_download(Embedded, 0x0002 | 0x0004), Ship);
    assert_eq!(default_download(Embedded, 0x0002 | 0x0008), Ship);
    // The policy can ignore fsType.
    let lenient = LicencePolicy {
        respect_restricted_fstype: false,
        ..LicencePolicy::default()
    };
    assert_eq!(
        decide(
            Embedded,
            FsType(0x0002),
            &lenient,
            Download,
            Toggle::Default
        ),
        Ship
    );
    assert_eq!(Exclusion::Restricted.code(), "MID5413");
}

#[test]
fn the_toggle_includes_a_restricted_face_from_the_viewer_or_the_user_only() {
    let policy = LicencePolicy::default();
    for origin in [Local, User] {
        assert_eq!(
            decide(origin, FsType(0x0002), &policy, Download, Toggle::Include),
            ShipWithNotice,
            "{origin:?}"
        );
        assert_eq!(
            decide(origin, FsType(0x0002), &policy, Download, Toggle::LeaveOut),
            Exclude(Exclusion::LeftOut),
            "{origin:?}"
        );
        // Bitmap-only stays out whatever the toggle.
        assert_eq!(
            decide(origin, FsType(0x0202), &policy, Download, Toggle::Include),
            Exclude(Exclusion::BitmapOnly),
            "{origin:?}"
        );
    }
    // The toggle includes a face its source's policy leaves out by default.
    let no_viewer = LicencePolicy {
        include_viewer_fonts_in_download: false,
        ..LicencePolicy::default()
    };
    assert_eq!(
        decide(Local, FsType(0), &no_viewer, Download, Toggle::Include),
        ShipWithNotice
    );
    // Other sources offer no toggle: their restricted faces stay out.
    for origin in [Embedded, Google, Bundled] {
        assert_eq!(
            decide(origin, FsType(0x0002), &policy, Download, Toggle::Include),
            Exclude(Exclusion::Restricted),
            "{origin:?}"
        );
    }
}

#[test]
fn a_bitmap_only_face_never_ships() {
    for origin in [Embedded, Local, Google, User, Bundled] {
        for fs_type in [0x0200, 0x0200 | 0x0004, 0x0200 | 0x0002] {
            assert_eq!(
                default_download(origin, fs_type),
                Exclude(Exclusion::BitmapOnly),
                "{origin:?} {fs_type:#x}"
            );
            assert_eq!(
                default_preview(origin, fs_type),
                Exclude(Exclusion::BitmapOnly),
                "{origin:?} {fs_type:#x}"
            );
        }
    }
}

#[test]
fn fstype_bits_read_as_the_spec_says() {
    assert!(FsType(0x0002).restricted());
    assert!(!FsType(0x0006).restricted() && !FsType(0x000a).restricted());
    assert!(!FsType(0x0000).restricted() && !FsType(0x0004).restricted());
    assert!(FsType(0x0100).no_subsetting() && !FsType(0x0200).no_subsetting());
    assert!(FsType(0x0200).bitmap_only() && !FsType(0x0100).bitmap_only());
}

#[test]
fn faces_from_the_viewer_or_the_user_ship_with_a_notice_or_are_left_out() {
    for origin in [Local, User] {
        assert_eq!(default_preview(origin, 0), Ship, "{origin:?}");
        assert_eq!(default_download(origin, 0), ShipWithNotice, "{origin:?}");
        let toggled = decide(
            origin,
            FsType(0),
            &LicencePolicy::default(),
            Download,
            Toggle::LeaveOut,
        );
        assert_eq!(toggled, Exclude(Exclusion::LeftOut), "{origin:?}");
        // The preview ships whatever the toggle.
        let preview = decide(
            origin,
            FsType(0),
            &LicencePolicy::default(),
            Preview,
            Toggle::LeaveOut,
        );
        assert_eq!(preview, Ship, "{origin:?}");
    }
    let no_viewer = LicencePolicy {
        include_viewer_fonts_in_download: false,
        ..LicencePolicy::default()
    };
    assert_eq!(
        decide(Local, FsType(0), &no_viewer, Download, Toggle::Default),
        Exclude(Exclusion::LeftOut)
    );
    assert_eq!(
        decide(User, FsType(0), &no_viewer, Download, Toggle::Default),
        ShipWithNotice
    );
    let no_user = LicencePolicy {
        include_user_fonts_in_download: false,
        ..LicencePolicy::default()
    };
    assert_eq!(
        decide(User, FsType(0), &no_user, Download, Toggle::Default),
        Exclude(Exclusion::LeftOut)
    );
    // The toggle is offered only for those sources: other faces ignore it.
    for origin in [Embedded, Google, Bundled] {
        assert_eq!(
            decide(
                origin,
                FsType(0),
                &LicencePolicy::default(),
                Download,
                Toggle::LeaveOut
            ),
            Ship
        );
    }
    assert!(ShipWithNotice.ships() && Ship.ships() && !Exclude(Exclusion::LeftOut).ships());
}

#[test]
fn the_notice_lists_each_family_once_in_key_order() {
    let families = notice_families([
        ("Brand Sans", ShipWithNotice),
        ("Roboto", Ship),
        ("Avenir Next", ShipWithNotice),
        ("brand-sans", ShipWithNotice),
        ("Helvetica", Exclude(Exclusion::LeftOut)),
    ]);
    assert_eq!(families, ["Avenir Next", "Brand Sans"]);
}

#[test]
fn a_restricted_embedded_face_is_left_out_of_the_download_end_to_end() {
    let restricted = with_fs_type(OSWALD, 0x0002);
    let details = read_details(&restricted, 0).unwrap();
    assert_eq!(details.fs_type, 0x0002);
    let decision = decide(
        Embedded,
        FsType(details.fs_type),
        &LicencePolicy::default(),
        Download,
        Toggle::Default,
    );
    assert_eq!(decision, Exclude(Exclusion::Restricted));
    // The unpatched file ships.
    let open = read_details(OSWALD, 0).unwrap();
    assert_eq!(default_download(Embedded, open.fs_type), Ship);
}

mod licences {
    use super::*;

    fn descriptor() -> FaceDescriptor {
        FaceDescriptor {
            weight: WeightRange::point(400),
            style: FaceStyle::Normal,
        }
    }

    #[test]
    fn an_entry_reads_the_name_table_and_names_the_licence() {
        for (name, font) in ALL {
            let details = read_details(font, 0).unwrap();
            let entry = LicenceEntry::new("Family", descriptor(), Google, &details, None);
            assert_eq!(entry.licence_id.as_deref(), Some("OFL-1.1"), "{name}");
            assert!(entry.copyright.is_some(), "{name}");
            assert!(
                entry
                    .licence
                    .as_deref()
                    .unwrap()
                    .contains("SIL Open Font License"),
                "{name}"
            );
        }
        let details = read_details(OSWALD, 0).unwrap();
        let entry = LicenceEntry::new("Oswald", descriptor(), Embedded, &details, None);
        let text = entry.text();
        let lines: Vec<&str> = text.lines().collect();
        assert!(
            lines[0].starts_with("Copyright 2016 The Oswald Project Authors"),
            "{text}"
        );
        assert_eq!(lines[1], "Licence: OFL-1.1");
        assert_eq!(lines.last().copied(), Some("https://scripts.sil.org/OFL"));
    }

    #[test]
    fn the_source_licence_wins_over_the_name_table() {
        let details = read_details(OSWALD, 0).unwrap();
        let entry = LicenceEntry::new("Oswald", descriptor(), Google, &details, Some("Apache-2.0"));
        assert_eq!(entry.licence_id.as_deref(), Some("Apache-2.0"));
    }

    #[test]
    fn a_face_without_licence_names_says_so() {
        let bare = sfnt(&FaceSpec {
            grave_top: Some(1400),
            ..FaceSpec::new("Bare")
        });
        let details = read_details(&bare, 0).unwrap();
        let entry = LicenceEntry::new("Bare", descriptor(), User, &details, None);
        assert_eq!(entry.licence_id, None);
        assert_eq!(entry.text(), "No copyright or licence string.");
        assert_eq!(
            licences_text(&[entry]),
            "Bare 400 normal (added by the user)\nNo copyright or licence string.\n\n"
        );
        assert_eq!(
            licences_text(&[]),
            "No font files ship with this presentation.\n"
        );
        assert_eq!(licence_id("Copyright Apple Inc."), None);
    }

    #[test]
    fn each_named_licence_follows_the_entries_in_full_once() {
        let details = read_details(OSWALD, 0).unwrap();
        let entry = |family: &str, licence: Option<&str>| {
            LicenceEntry::new(family, descriptor(), Google, &details, licence)
        };
        let text = licences_text(&[
            entry("Oswald", None),
            entry("Roboto", Some("Apache-2.0")),
            entry("Oswald Bold", Some("OFL-1.1")),
            entry("Mystery", Some("LicenseRef-Mystery")),
        ]);
        assert_eq!(
            text.matches("SIL OPEN FONT LICENSE Version 1.1 - 26 February 2007")
                .count(),
            1
        );
        assert_eq!(
            text.matches("TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION")
                .count(),
            1
        );
        let apache = text.find("\nThe Apache-2.0 licence\n\n").unwrap();
        let ofl = text.find("\nThe OFL-1.1 licence\n\n").unwrap();
        assert!(text.find("Mystery 400 normal").unwrap() < apache && apache < ofl);
        assert!(
            !text.contains("UBUNTU FONT LICENCE")
                && !text.contains("Bitstream Vera Fonts Copyright")
        );
        assert!(
            text.ends_with("OTHER DEALINGS IN THE FONT SOFTWARE.\n\n"),
            "{text}"
        );
    }

    #[test]
    fn the_licence_texts_are_the_repositorys() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../LICENSES");
        for (id, text) in LICENCE_TEXTS {
            let path = root.join(format!("{id}.txt"));
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                text,
                "{}",
                path.display()
            );
        }
    }

    #[test]
    fn a_shipped_face_carries_its_entry() {
        let mut wanted = CodepointSet::new();
        wanted.insert_text("Shalom שלום");
        let shipped = ship_face(
            &ShipInput {
                family: "Noto Sans Hebrew",
                bytes: NOTO_HEBREW,
                index: 0,
                origin: Google,
                codepoints: &wanted,
                small_caps: false,
                source_licence: Some("OFL-1.1"),
            },
            &ShipOptions::default(),
        )
        .unwrap();
        let entry = &shipped.licence;
        assert_eq!(
            (
                entry.family.as_str(),
                entry.weight.as_str(),
                entry.style,
                entry.origin
            ),
            ("Noto Sans Hebrew", "100 900", "normal", Google)
        );
        let text = licences_text(std::slice::from_ref(entry));
        assert!(text.starts_with("Noto Sans Hebrew 100 900 normal (Google Fonts)\nCopyright 2024 The Noto Project Authors"), "{text}");
    }
}
