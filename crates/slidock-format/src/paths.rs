//! Where each file of a document set lives. Slides and layouts are named by a file id derived from their own id, and
//! media by the SHA-256 of the source bytes, so compiling the same deck again writes the same paths.

use std::fmt;

use slidock_canon::sha256_hex;
use thiserror::Error;

/// The manifest, at the root of a document set.
pub const MANIFEST_PATH: &str = "presentation.json";
pub const SLIDES_DIR: &str = "slides";
pub const LAYOUTS_DIR: &str = "layouts";
pub const MEDIA_DIR: &str = "media";

/// What a document may load: images and video a browser shows. Never HTML, which can run scripts; SVG only as the
/// compiler writes it, a file that passed `slidock_io::check_svg`'s allow-list (no script, no outside reference).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MediaExtension {
    Png,
    Jpg,
    Gif,
    Webp,
    Avif,
    Mp4,
    Webm,
    Mov,
    Svg,
}

impl MediaExtension {
    pub const ALL: [MediaExtension; 9] = [
        MediaExtension::Png,
        MediaExtension::Jpg,
        MediaExtension::Gif,
        MediaExtension::Webp,
        MediaExtension::Avif,
        MediaExtension::Mp4,
        MediaExtension::Webm,
        MediaExtension::Mov,
        MediaExtension::Svg,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            MediaExtension::Png => "png",
            MediaExtension::Jpg => "jpg",
            MediaExtension::Gif => "gif",
            MediaExtension::Webp => "webp",
            MediaExtension::Avif => "avif",
            MediaExtension::Mp4 => "mp4",
            MediaExtension::Webm => "webm",
            MediaExtension::Mov => "mov",
            MediaExtension::Svg => "svg",
        }
    }

    /// The extension spelled exactly `ext` (lowercase), if the format allows it.
    pub fn parse(ext: &str) -> Option<MediaExtension> {
        MediaExtension::ALL
            .into_iter()
            .find(|candidate| candidate.as_str() == ext)
    }
}

impl fmt::Display for MediaExtension {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum PathError {
    #[error("Not a SHA-256: {0}")]
    NotSha256(String),
    #[error("Not a media path: {0}")]
    NotMediaPath(String),
}

const PREFIX_CHARS: usize = 60;
const HASH_CHARS: usize = 8;

fn is_file_id_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-'
}

/// `^[a-z0-9][a-z0-9.-]{0,79}$`
fn is_plain_file_id(id: &str) -> bool {
    let mut chars = id.chars();
    let first_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    first_ok && id.len() <= 80 && chars.all(is_file_id_char)
}

/// The file name of an id: the id itself when it is already a safe lowercase name, else its lowercased, sanitised
/// head and the first 8 hex of its SHA-256. Case-insensitive file systems then keep ids that differ only in case
/// apart, and every id names a path inside its folder.
pub fn file_id(id: &str) -> String {
    if is_plain_file_id(id) {
        return id.to_owned();
    }
    // Each UTF-16 code unit outside the safe set becomes one `-`, as the reference's per-unit regex does.
    let mut head = String::new();
    for c in id.chars() {
        let c = c.to_ascii_lowercase();
        if is_file_id_char(c) {
            head.push(c);
        } else {
            for _ in 0..c.len_utf16() {
                head.push('-');
            }
        }
        if head.len() >= PREFIX_CHARS {
            break;
        }
    }
    head.truncate(PREFIX_CHARS);
    let hash = sha256_hex(id.as_bytes());
    let short = hash.get(..HASH_CHARS).unwrap_or(&hash);
    format!("{head}-{short}")
}

pub fn slide_path(id: &str) -> String {
    format!("{SLIDES_DIR}/{}.html", file_id(id))
}

pub fn layout_path(id: &str) -> String {
    format!("{LAYOUTS_DIR}/{}.html", file_id(id))
}

/// `^[0-9a-f]{64}$`
pub fn is_sha256(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// `media/<sha256>.<ext>`: the source bytes' hash and the extension of what the file holds.
pub fn media_path(sha256: &str, ext: MediaExtension) -> Result<String, PathError> {
    if !is_sha256(sha256) {
        return Err(PathError::NotSha256(sha256.to_owned()));
    }
    Ok(format!("{MEDIA_DIR}/{sha256}.{ext}"))
}

/// A media file of the allowed form, named by its path in the document set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaRef {
    /// `media/<sha256>.<ext>`
    pub path: String,
    pub sha256: String,
    pub ext: MediaExtension,
}

/// The parts of `media/<sha256>.<ext>`.
fn parse_media_path(path: &str) -> Option<MediaRef> {
    let rest = path.strip_prefix(MEDIA_DIR)?.strip_prefix('/')?;
    let (sha256, ext) = rest.split_once('.')?;
    if !is_sha256(sha256) {
        return None;
    }
    Some(MediaRef {
        path: path.to_owned(),
        sha256: sha256.to_owned(),
        ext: MediaExtension::parse(ext)?,
    })
}

/// How a slide or layout document references a media file: relative, from its own folder.
pub fn media_reference(path: &str) -> Result<String, PathError> {
    if !is_media_path(path) {
        return Err(PathError::NotMediaPath(path.to_owned()));
    }
    Ok(format!("../{path}"))
}

/// The media file a document reference names, or `None` for any form but `../media/<sha256>.<ext>`.
pub fn parse_media_reference(reference: &str) -> Option<MediaRef> {
    parse_media_path(reference.strip_prefix("../")?)
}

/// Whether a path is a media file of the allowed form.
pub fn is_media_path(path: &str) -> bool {
    parse_media_path(path).is_some()
}

/// `^(?:slides|layouts)/[^/]+\.html?$`, case-insensitive: a path in a document folder.
pub(crate) fn is_document_path(path: &str) -> bool {
    let Some((dir, name)) = path.split_once('/') else {
        return false;
    };
    let in_folder = dir.eq_ignore_ascii_case(SLIDES_DIR) || dir.eq_ignore_ascii_case(LAYOUTS_DIR);
    let lower = name.to_ascii_lowercase();
    let stem = lower
        .strip_suffix(".html")
        .or_else(|| lower.strip_suffix(".htm"));
    in_folder && !name.contains('/') && stem.is_some_and(|stem| !stem.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn keeps_a_safe_lowercase_id_as_it_is() {
        assert_eq!(file_id("pp-256"), "pp-256");
        assert_eq!(file_id("kn-5213335"), "kn-5213335");
        assert_eq!(file_id("od-p3.m.12"), "od-p3.m.12");
    }

    #[test]
    fn hashes_mixed_case_so_case_insensitive_file_systems_keep_ids_apart() {
        let upper = file_id("gs-SLIDES_API1");
        assert_eq!(
            upper,
            format!("gs-slides-api1-{}", &sha256_hex("gs-SLIDES_API1")[..8])
        );
        assert_ne!(file_id("gs-slides_api1"), upper);
        assert_ne!(file_id("ir-Ab"), file_id("ir-ab"));
        // The sample manifest's second slide.
        assert_eq!(file_id("ir-S2"), "ir-s2-da7f247d");
    }

    #[test]
    fn cuts_long_ids_to_60_characters_plus_the_hash() {
        let id = format!("ir-{}", "x".repeat(100));
        let name = file_id(&id);
        assert_eq!(name, format!("{}-{}", &id[..60], &sha256_hex(&id)[..8]));
        assert_eq!(name.len(), 69);
        // An 80-character safe id is plain; 81 is cut.
        assert_eq!(file_id(&"a".repeat(80)), "a".repeat(80));
        assert_eq!(file_id(&"a".repeat(81)).len(), 69);
    }

    #[test]
    fn turns_each_utf16_unit_outside_the_safe_set_into_a_dash() {
        assert_eq!(
            file_id("ir-ünï"),
            format!("ir--n--{}", &sha256_hex("ir-ünï")[..8])
        );
        // An astral character is two UTF-16 units.
        assert_eq!(
            file_id("ir-😀"),
            format!("ir----{}", &sha256_hex("ir-😀")[..8])
        );
    }

    #[test]
    fn never_names_a_path_outside_its_folder() {
        for id in ["../x", "a/b", "..", "ir-a\u{0}b", "ir-ünï"] {
            let name = file_id(id);
            assert!(
                name.chars().all(is_file_id_char) && !name.is_empty(),
                "{name}"
            );
            let path = slide_path(id);
            assert!(path.starts_with("slides/") && path.ends_with(".html"));
            assert_eq!(path.matches('/').count(), 1, "{path}");
        }
    }

    #[test]
    fn places_slides_and_layouts_in_their_folders() {
        assert_eq!(slide_path("pp-256"), "slides/pp-256.html");
        assert_eq!(layout_path("pp-2147483649"), "layouts/pp-2147483649.html");
    }

    #[test]
    fn names_media_by_the_source_hash_and_the_output_extension() {
        assert_eq!(
            media_path(SHA, MediaExtension::Png).unwrap(),
            format!("media/{SHA}.png")
        );
        assert_eq!(
            media_reference(&media_path(SHA, MediaExtension::Mp4).unwrap()).unwrap(),
            format!("../media/{SHA}.mp4")
        );
        assert!(is_media_path(&format!("media/{SHA}.webm")));
        assert_eq!(MediaExtension::parse("svg"), Some(MediaExtension::Svg));
        assert_eq!(MediaExtension::parse("html"), None);
        assert!(
            MediaExtension::ALL
                .iter()
                .all(|ext| MediaExtension::parse(ext.as_str()) == Some(*ext))
        );
    }

    #[test]
    fn refuses_other_names() {
        assert!(media_path("ABC", MediaExtension::Png).is_err());
        assert!(media_path(&SHA.to_uppercase(), MediaExtension::Png).is_err());
        assert!(media_reference("media/x.png").is_err());
        assert!(!is_media_path(&format!("media/{SHA}.html")));
        assert!(!is_media_path(&format!("media/{SHA}.PNG")));
    }

    #[test]
    fn parses_only_the_one_allowed_reference_form() {
        assert_eq!(
            parse_media_reference(&format!("../media/{SHA}.jpg")),
            Some(MediaRef {
                path: format!("media/{SHA}.jpg"),
                sha256: SHA.to_owned(),
                ext: MediaExtension::Jpg
            })
        );
        for reference in [
            format!("media/{SHA}.jpg"),
            format!("../media/{SHA}.html"),
            format!("../media/{}.jpg", SHA.to_uppercase()),
            format!("../media/{SHA}.jpg?x=1"),
            format!("https://example.com/media/{SHA}.jpg"),
            format!("../../media/{SHA}.jpg"),
            format!("../media/{SHA}.jpg.png"),
        ] {
            assert_eq!(parse_media_reference(&reference), None, "{reference}");
        }
    }

    #[test]
    fn tells_document_paths() {
        assert!(is_document_path("slides/a.html"));
        assert!(is_document_path("LAYOUTS/b.HTM"));
        assert!(!is_document_path("slides/a/b.html"));
        assert!(!is_document_path("slides/.html"));
        assert!(!is_document_path("media/a.html"));
        assert!(!is_document_path("a.html"));
    }
}
