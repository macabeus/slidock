//! `LICENSES.txt`: slidock's notice for the player embedded in `index.html`, Heroicons' for the presenter view when
//! the page ships one, then the copyright and licence of every face the download ships and the full text of each
//! licence they name.

use slidock_fonts::LicenceEntry;
use slidock_fonts::policy::licences_text;

use crate::ShippedFace;

/// slidock's licence: a copy of the repository's `LICENSE`, which a test keeps equal to it.
const SLIDOCK_LICENSE: &str = include_str!("../LICENSE");

/// The licence of the Heroicons icon paths the presenter view draws, as NOTICE gives it.
const HEROICONS_LICENSE: &str = include_str!("../licenses/Heroicons.txt");

pub fn licenses_text(shipped: &[&ShippedFace], presenter: bool) -> String {
    let mut text = String::from(
        "This presentation was made with slidock. The player in index.html is part of slidock:\n\n",
    );
    text.push_str(SLIDOCK_LICENSE.trim_end());
    if presenter {
        text.push_str(
            "\n\n\nThe presenter view in index.html is part of slidock too. It draws icons from Heroicons \
             (https://github.com/tailwindlabs/heroicons):\n\n",
        );
        text.push_str(HEROICONS_LICENSE.trim_end());
    }
    text.push_str("\n\n\nFonts\n\n");
    if !shipped.is_empty() {
        text.push_str(
            "The font files are the faces below as slidock ships them: subset to the characters this presentation \
             draws (a face that cannot be subset ships whole), and renamed when the face's licence reserves its \
             names.\n\n",
        );
    }
    let entries: Vec<LicenceEntry> = shipped.iter().map(|face| face.licence.clone()).collect();
    text.push_str(&licences_text(&entries));
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repository_file(name: &str) -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(name);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    #[test]
    fn the_licence_copy_is_the_repository_licence() {
        assert_eq!(SLIDOCK_LICENSE, repository_file("LICENSE"));
    }

    #[test]
    fn the_heroicons_licence_is_the_one_notice_gives() {
        let notice = repository_file("NOTICE");
        for line in HEROICONS_LICENSE.lines().filter(|line| !line.is_empty()) {
            let words = line.split_whitespace().collect::<Vec<_>>().join(" ");
            let flat = notice.split_whitespace().collect::<Vec<_>>().join(" ");
            assert!(flat.contains(&words), "NOTICE lacks {line:?}");
        }
    }

    #[test]
    fn heroicons_ship_with_the_presenter_view_only() {
        let with = licenses_text(&[], true);
        let without = licenses_text(&[], false);
        assert!(with.contains("Copyright (c) Tailwind Labs, Inc."));
        assert!(!without.contains("Tailwind"));
        for text in [&with, &without] {
            assert!(text.starts_with("This presentation was made with slidock."));
            assert!(text.contains("Copyright (c) 2026 Bruno Macabeus"));
            assert!(text.contains("\n\n\nFonts\n\n"));
        }
    }
}
