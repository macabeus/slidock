//! Media types from magic bytes. The MIME type recorded in the IR is what the bytes are, whatever a package claims
//! (content types, file extensions).

use slidock_canon::is_js_whitespace;

use crate::text::decode_utf8;

/// A media type [`sniff_mime`] recognises.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Mime {
    Png,
    Jpeg,
    Gif,
    Webp,
    Wav,
    Tiff,
    Bmp,
    Emf,
    Wmf,
    Pdf,
    Zip,
    Webm,
    Matroska,
    Ogg,
    Flac,
    Mpeg,
    Ttf,
    Otf,
    FontCollection,
    Woff,
    Woff2,
    Avif,
    Heic,
    QuickTime,
    M4v,
    M4a,
    Mp4,
    Svg,
    Eot,
}

impl Mime {
    pub const ALL: [Mime; 29] = [
        Mime::Png,
        Mime::Jpeg,
        Mime::Gif,
        Mime::Webp,
        Mime::Wav,
        Mime::Tiff,
        Mime::Bmp,
        Mime::Emf,
        Mime::Wmf,
        Mime::Pdf,
        Mime::Zip,
        Mime::Webm,
        Mime::Matroska,
        Mime::Ogg,
        Mime::Flac,
        Mime::Mpeg,
        Mime::Ttf,
        Mime::Otf,
        Mime::FontCollection,
        Mime::Woff,
        Mime::Woff2,
        Mime::Avif,
        Mime::Heic,
        Mime::QuickTime,
        Mime::M4v,
        Mime::M4a,
        Mime::Mp4,
        Mime::Svg,
        Mime::Eot,
    ];

    /// The MIME type as the IR records it.
    pub const fn as_str(self) -> &'static str {
        match self {
            Mime::Png => "image/png",
            Mime::Jpeg => "image/jpeg",
            Mime::Gif => "image/gif",
            Mime::Webp => "image/webp",
            Mime::Wav => "audio/wav",
            Mime::Tiff => "image/tiff",
            Mime::Bmp => "image/bmp",
            Mime::Emf => "image/emf",
            Mime::Wmf => "image/wmf",
            Mime::Pdf => "application/pdf",
            Mime::Zip => "application/zip",
            Mime::Webm => "video/webm",
            Mime::Matroska => "video/x-matroska",
            Mime::Ogg => "audio/ogg",
            Mime::Flac => "audio/flac",
            Mime::Mpeg => "audio/mpeg",
            Mime::Ttf => "font/ttf",
            Mime::Otf => "font/otf",
            Mime::FontCollection => "font/collection",
            Mime::Woff => "font/woff",
            Mime::Woff2 => "font/woff2",
            Mime::Avif => "image/avif",
            Mime::Heic => "image/heic",
            Mime::QuickTime => "video/quicktime",
            Mime::M4v => "video/x-m4v",
            Mime::M4a => "audio/mp4",
            Mime::Mp4 => "video/mp4",
            Mime::Svg => "image/svg+xml",
            Mime::Eot => "application/vnd.ms-fontobject",
        }
    }

    /// The type a MIME string names, when it is one of these.
    pub fn from_mime(mime: &str) -> Option<Mime> {
        Mime::ALL
            .into_iter()
            .find(|candidate| candidate.as_str() == mime)
    }
}

impl std::fmt::Display for Mime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Whether `bytes` holds `signature` at `at`.
fn has_at(bytes: &[u8], at: usize, signature: &[u8]) -> bool {
    bytes
        .get(at..)
        .is_some_and(|rest| rest.starts_with(signature))
}

fn u32le(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([
        *b.first()?,
        *b.get(1)?,
        *b.get(2)?,
        *b.get(3)?,
    ]))
}

/// At most this many compatible brands are read from an `ftyp` box.
const MAX_BRANDS: usize = 64;

/// The type one `ftyp` brand names on its own: AV1 or HEVC images and image sequences (AV1-ISOBMFF, ISO/IEC
/// 23008-12), QuickTime, Apple's M4V and M4A, and the ISO, MP4, DASH, CMAF and 3GPP brands of an MP4 file.
fn brand_type(brand: &[u8]) -> Option<Mime> {
    Some(match brand {
        b"avif" | b"avis" => Mime::Avif,
        b"heic" | b"heix" | b"heim" | b"heis" | b"hevc" | b"hevx" | b"hevm" | b"hevs" => Mime::Heic,
        b"qt  " => Mime::QuickTime,
        b"M4V " | b"M4VH" | b"M4VP" => Mime::M4v,
        b"M4A " | b"M4B " => Mime::M4a,
        b"isom" | b"iso2" | b"iso3" | b"iso4" | b"iso5" | b"iso6" | b"iso7" | b"iso8" | b"iso9"
        | b"avc1" | b"mp41" | b"mp42" | b"mp71" | b"dash" | b"msdh" | b"msix" | b"cmfc"
        | b"cmf2" | b"mmp4" | b"MSNV" | b"f4v " | b"3gp4" | b"3gp5" | b"3gp6" | b"3gp7"
        | b"3gp8" | b"3gp9" | b"3g2a" | b"3g2b" | b"3g2c" => Mime::Mp4,
        _ => return None,
    })
}

/// The HEIF structural brands, which name no codec: an image item (`mif1`, `miaf`) or an image sequence (`msf1`).
fn is_heif_structure(brand: &[u8]) -> bool {
    matches!(brand, b"mif1" | b"miaf" | b"msf1")
}

/// ISO base media files (MP4, QuickTime, HEIF/AVIF) by their `ftyp` brands. A major brand that names a type decides;
/// otherwise the compatible brands do, an image codec (AVIF, then HEIC) first. A HEIF file whose brands name no codec
/// listed here, and a file whose brands name nothing listed here, are unrecognised.
fn iso_media_type(head: &[u8]) -> Option<Mime> {
    if head.len() < 12 {
        return None;
    }
    let box_type = head.get(4..8)?;
    if matches!(box_type, b"moov" | b"mdat" | b"wide" | b"free") {
        return Some(Mime::QuickTime);
    }
    if box_type != b"ftyp" {
        return None;
    }
    let size = u32::from_be_bytes(head.get(..4)?.try_into().ok()?);
    // A 64-bit size (1) follows the type; 0 runs to the end of the file.
    let (major_at, end) = match size {
        1 => {
            let large = u64::from_be_bytes(head.get(8..16)?.try_into().ok()?);
            (16, usize::try_from(large).unwrap_or(usize::MAX))
        }
        0 => (8, usize::MAX),
        size => (8, usize::try_from(size).unwrap_or(usize::MAX)),
    };
    let major = head.get(major_at..major_at + 4)?;
    if let Some(mime) = brand_type(major) {
        return Some(mime);
    }
    // The minor version, then the compatible brands to the box's end.
    let compatible_at = major_at + 8;
    let end = end.min(head.len()).min(compatible_at + 4 * MAX_BRANDS);
    let compatible: Vec<&[u8]> = head
        .get(compatible_at..end.max(compatible_at))
        .unwrap_or_default()
        .as_chunks::<4>()
        .0
        .iter()
        .map(|brand| brand.as_slice())
        .collect();
    let named: Vec<Mime> = compatible
        .iter()
        .filter_map(|brand| brand_type(brand))
        .collect();
    if let Some(image) = [Mime::Avif, Mime::Heic]
        .into_iter()
        .find(|image| named.contains(image))
    {
        return Some(image);
    }
    if is_heif_structure(major) || compatible.iter().any(|brand| is_heif_structure(brand)) {
        return None;
    }
    named.first().copied()
}

/// Whether `text` holds `prefix` at byte `at`, ASCII case-insensitively.
fn starts_with_ci(text: &str, at: usize, prefix: &str) -> bool {
    text.as_bytes()
        .get(at..)
        .and_then(|rest| rest.get(..prefix.len()))
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix.as_bytes()))
}

/// The byte offset past the JavaScript whitespace (`\s`) at `at`.
fn skip_space(text: &str, at: usize) -> usize {
    text.get(at..).map_or(at, |rest| {
        at + rest.len() - rest.trim_start_matches(is_js_whitespace).len()
    })
}

/// An SVG document: optional BOM, XML declaration, comments or doctype, then an `<svg` root. The same language as
/// `/^\s*(?:<\?xml[^>]*\?>\s*)?(?:<!--[\s\S]*?-->\s*|<!DOCTYPE[^>]*>\s*)*<svg[\s>]/i` over the first 4 KiB,
/// backtracking included: a comment may end at any of its `-->`.
fn is_svg(head: &[u8]) -> bool {
    let text = decode_utf8(head.get(..4096).unwrap_or(head));
    let text = text.as_ref();
    let mut at = skip_space(text, 0);
    if starts_with_ci(text, at, "<?xml") {
        // `[^>]*\?>`: the first `>` after `<?xml` closes it, and a `?` past `<?xml` precedes it.
        let Some(close) = text.get(at + 5..).and_then(|rest| rest.find('>')) else {
            return false;
        };
        if close == 0 || text.as_bytes().get(at + 5 + close - 1) != Some(&b'?') {
            return false;
        }
        at = skip_space(text, at + 5 + close + 1);
    }
    // Every position the `(comment | doctype)*` loop can stand at, explored once each.
    let mut seen = vec![false; text.len() + 1];
    let mut pending = vec![at];
    while let Some(at) = pending.pop() {
        match seen.get_mut(at) {
            Some(seen) if !*seen => *seen = true,
            _ => continue,
        }
        if starts_with_ci(text, at, "<svg") {
            let next = text.get(at + 4..).and_then(|rest| rest.chars().next());
            if next.is_some_and(|c| c == '>' || is_js_whitespace(c)) {
                return true;
            }
        }
        if starts_with_ci(text, at, "<!--") {
            let body = at + 4;
            let mut from = body;
            while let Some(found) = text.get(from..).and_then(|rest| rest.find("-->")) {
                pending.push(skip_space(text, from + found + 3));
                from += found + 1;
            }
        } else if starts_with_ci(text, at, "<!DOCTYPE")
            && let Some(close) = text.get(at + 9..).and_then(|rest| rest.find('>'))
        {
            pending.push(skip_space(text, at + 9 + close + 1));
        }
    }
    false
}

/// An Embedded OpenType font: its magic number and a known version.
fn is_eot(head: &[u8]) -> bool {
    has_at(head, 34, &[0x4c, 0x50])
        && matches!(
            u32le(head, 8),
            Some(0x0001_0000 | 0x0002_0001 | 0x0002_0002)
        )
}

/// The MIME type of a file from its first bytes (4 KiB is enough), or `None` when unrecognised.
pub fn sniff_mime(head: &[u8]) -> Option<Mime> {
    let starts = |signature: &[u8]| has_at(head, 0, signature);
    if starts(&[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]) {
        return Some(Mime::Png);
    }
    if starts(&[0xff, 0xd8, 0xff]) {
        return Some(Mime::Jpeg);
    }
    if starts(b"GIF87a") || starts(b"GIF89a") {
        return Some(Mime::Gif);
    }
    if starts(b"RIFF") && has_at(head, 8, b"WEBP") {
        return Some(Mime::Webp);
    }
    if starts(b"RIFF") && has_at(head, 8, b"WAVE") {
        return Some(Mime::Wav);
    }
    if starts(&[0x49, 0x49, 0x2a, 0x00]) || starts(&[0x4d, 0x4d, 0x00, 0x2a]) {
        return Some(Mime::Tiff);
    }
    if starts(b"BM") && head.len() >= 26 {
        return Some(Mime::Bmp);
    }
    if u32le(head, 0) == Some(1) && head.len() >= 44 && has_at(head, 40, b" EMF") {
        return Some(Mime::Emf);
    }
    if starts(&[0xd7, 0xcd, 0xc6, 0x9a]) || starts(&[0x01, 0x00, 0x09, 0x00, 0x00, 0x03]) {
        return Some(Mime::Wmf);
    }
    if starts(b"%PDF-") {
        return Some(Mime::Pdf);
    }
    if starts(&[0x50, 0x4b, 0x03, 0x04]) || starts(&[0x50, 0x4b, 0x05, 0x06]) {
        return Some(Mime::Zip);
    }
    if starts(&[0x1a, 0x45, 0xdf, 0xa3]) {
        let first = head.get(..64).unwrap_or(head);
        let webm = first.windows(4).any(|window| window == b"webm");
        return Some(if webm { Mime::Webm } else { Mime::Matroska });
    }
    if starts(b"OggS") {
        return Some(Mime::Ogg);
    }
    if starts(b"fLaC") {
        return Some(Mime::Flac);
    }
    if starts(b"ID3") || matches!(head, [0xff, second, ..] if second & 0xe0 == 0xe0) {
        return Some(Mime::Mpeg);
    }
    if starts(&[0x00, 0x01, 0x00, 0x00]) || starts(b"true") {
        return Some(Mime::Ttf);
    }
    if starts(b"OTTO") {
        return Some(Mime::Otf);
    }
    if starts(b"ttcf") {
        return Some(Mime::FontCollection);
    }
    if starts(b"wOFF") {
        return Some(Mime::Woff);
    }
    if starts(b"wOF2") {
        return Some(Mime::Woff2);
    }
    if let Some(iso) = iso_media_type(head) {
        return Some(iso);
    }
    if is_svg(head) {
        return Some(Mime::Svg);
    }
    if is_eot(head) {
        return Some(Mime::Eot);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_every_type_once() {
        for mime in Mime::ALL {
            assert_eq!(Mime::from_mime(mime.as_str()), Some(mime));
        }
        assert_eq!(Mime::from_mime("image/x-unknown"), None);
    }

    #[test]
    fn reads_svg_heads_as_the_regular_expression_does() {
        let svg = |text: &str| is_svg(text.as_bytes());
        assert!(svg("<svg>"));
        assert!(svg("\u{feff} \n<SVG\txmlns='x'/>"));
        assert!(svg("<?xml version=\"1.0\"?><!-- a --><!DOCTYPE svg><svg>"));
        // `[\s>]` follows `<svg`, so a self-closed root does not match.
        assert!(!svg("<svg/>"));
        assert!(svg("<?xml?><svg>"));
        // A comment may close at a later `-->`, when what follows the first does not match.
        assert!(svg("<!-- a --> junk --> <svg>"));
        assert!(!svg("<!-- a --> junk <svg>"));
        assert!(!svg("<?xml><svg>"));
        assert!(!svg("<?xml?>?><svg>"));
        assert!(!svg("<svgx>"));
        assert!(!svg("<svg"));
        assert!(!svg("<!-- unclosed <svg>"));
        assert!(!svg("<!--><svg>"));
        // The comment's `-->` starts past its `<!--`.
        assert!(!svg("<!---><svg>"));
        assert!(svg("<!---><svg>-->\n<svg "));
        assert!(!svg("x<svg>"));
        // Only the first 4 KiB are read.
        let mut long = " ".repeat(4092);
        long.push_str("<svg>");
        assert!(!svg(&long));
    }

    #[test]
    fn recognises_eot_after_every_other_type() {
        let mut eot = vec![0; 120];
        eot[8..12].copy_from_slice(&0x0002_0002u32.to_le_bytes());
        eot[34..36].copy_from_slice(&[0x4c, 0x50]);
        assert_eq!(sniff_mime(&eot), Some(Mime::Eot));
        eot[8] = 9;
        assert_eq!(sniff_mime(&eot), None);
    }
}
