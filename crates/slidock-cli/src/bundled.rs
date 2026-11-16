//! The bundled substitutes: Liberation 2.1.5, DejaVu Sans 2.37, Carlito and Caladea, the floor
//! under every font source. They are `slidock-substitutes`' files, which the web app serves too, built into the binary
//! (feature `bundled-substitutes`), so the CLI and the app ship the same bytes. A deck that draws a face of one of these
//! families with no file (a finished session's `substitutes`) takes the manifest's face of that family, weight and
//! style, as the app does, once its SHA-256 matches the manifest.

use slidock::{Sha256, SubstituteFace};
use slidock_ir::fonts::FaceStyle;
pub use slidock_substitutes::{FACES, Face};

fn style(face: &Face) -> FaceStyle {
    match face.style {
        slidock_substitutes::Style::Normal => FaceStyle::Normal,
        slidock_substitutes::Style::Italic => FaceStyle::Italic,
    }
}

/// The manifest's face for a substitute face the session names, matching the family without regard to ASCII case.
pub fn face_for(wanted: &SubstituteFace) -> Option<&'static Face> {
    FACES.iter().find(|face| {
        face.family.eq_ignore_ascii_case(&wanted.family)
            && face.weight == wanted.weight
            && style(face) == wanted.style
    })
}

/// A face's file, once its bytes match the manifest's SHA-256.
pub fn load(face: &Face) -> Result<Vec<u8>, String> {
    let actual = Sha256::of(face.bytes).to_hex();
    if actual == face.sha256 {
        Ok(face.bytes.to_vec())
    } else {
        Err(format!(
            "{}: SHA-256 {actual}, the manifest says {}",
            face.file, face.sha256
        ))
    }
}

#[cfg(all(test, feature = "bundled-substitutes"))]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "test code")]

    use super::*;

    fn wanted(family: &str, weight: u32, style: FaceStyle) -> SubstituteFace {
        SubstituteFace {
            family: family.to_owned(),
            postscript_name: String::new(),
            weight,
            style,
        }
    }

    #[test]
    fn the_built_in_faces_are_the_substitutes_and_load() {
        assert_eq!(FACES.len(), 24);
        for face in FACES {
            assert_eq!(load(face).unwrap(), face.bytes, "{}", face.file);
        }
    }

    #[test]
    fn every_built_in_floor_face_has_a_file() {
        let catalog = slidock::builtin_catalog();
        let mut faces = 0;
        for (key, family) in catalog.families() {
            if slidock::is_bundled_family(key) {
                for face in &family.faces {
                    let file = face_for(&wanted(&family.family, face.weight, face.style)).unwrap();
                    assert_eq!(file.family, family.family, "{}", file.file);
                    faces += 1;
                }
            }
        }
        assert!(faces >= 20, "{faces}");
    }

    #[test]
    fn a_face_is_matched_by_family_in_any_case_weight_and_style() {
        let face = face_for(&wanted("liberation SANS", 700, FaceStyle::Italic)).unwrap();
        assert_eq!(face.file, "LiberationSans-BoldItalic.woff2");
        assert!(face_for(&wanted("Liberation Sans", 500, FaceStyle::Normal)).is_none());
        assert!(face_for(&wanted("Arial", 400, FaceStyle::Normal)).is_none());
    }

    #[test]
    fn a_file_that_does_not_match_its_manifest_entry_is_not_used() {
        let face = Face {
            sha256: "0000000000000000000000000000000000000000000000000000000000000000",
            ..*face_for(&wanted("Caladea", 400, FaceStyle::Normal)).unwrap()
        };
        assert!(load(&face).unwrap_err().contains("the manifest says"));
    }
}
