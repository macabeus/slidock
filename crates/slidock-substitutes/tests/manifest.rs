//! `FACES` is `fonts/core.json`, in its order, with each file's bytes; the folder holds those files, their licences
//! and the manifest, and nothing else.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "test code")]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use slidock_substitutes::{FACES, Style};

fn fonts() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fonts")
}

fn manifest() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(fonts().join("core.json")).unwrap()).unwrap()
}

#[test]
fn the_faces_are_the_manifest_s_files_in_its_order() {
    let manifest = manifest();
    let listed = manifest["faces"].as_array().unwrap();
    assert_eq!(
        FACES.len(),
        24,
        "Liberation ×12, DejaVu Sans ×4, Carlito ×4, Caladea ×4"
    );
    assert_eq!(FACES.len(), listed.len());
    for (face, entry) in FACES.iter().zip(listed) {
        assert_eq!(face.file, entry["file"]);
        assert_eq!(face.family, entry["family"]);
        assert_eq!(u64::from(face.weight), entry["weight"]);
        let style = match face.style {
            Style::Normal => "normal",
            Style::Italic => "italic",
        };
        assert_eq!(style, entry["style"]);
        assert_eq!(face.licence, entry["licence"]);
        assert_eq!(face.sha256, entry["sha256"]);
        assert_eq!(face.bytes, std::fs::read(fonts().join(face.file)).unwrap());
        assert_eq!(
            hex::encode(Sha256::digest(face.bytes)),
            face.sha256,
            "{}",
            face.file
        );
    }
}

#[test]
fn the_folder_holds_the_faces_their_licences_and_the_manifest() {
    let manifest = manifest();
    let mut expected = BTreeSet::from(["core.json".to_owned()]);
    for entry in manifest["faces"].as_array().unwrap() {
        expected.insert(entry["file"].as_str().unwrap().to_owned());
        expected.insert(entry["licenceFile"].as_str().unwrap().to_owned());
    }
    let present: BTreeSet<String> = std::fs::read_dir(fonts())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(present, expected);
}
