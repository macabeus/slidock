//! Embedded fonts: the `svg:font-face-uri` files under `Fonts/` that `style:font-face` declarations name, handed to
//! the host with the faces the declarations say they hold. The presentation's IR records no font for them; reading,
//! unwrapping and checking the files is the font stage's. A file is never inflated here: a STORED entry is handed on
//! as a range of the input, a DEFLATE entry as its compressed bytes, inflated when the font stage first reads it.

use std::borrow::Cow;
use std::sync::Arc;
use std::sync::OnceLock;

use slidock_core_api::frontend::FontContainer;
use slidock_core_api::{EmbeddedFont, MediaSource};
use slidock_io::zip::{DEFLATE, EntryReader, LOCAL_HEADER_SIZE, STORED};
use slidock_io::{RangeError, RangeReader, ZipArchive, ZipEntry, inflate_raw_exact};
use slidock_ir::fonts::{FaceRequest, FaceStyle, font_key};

use crate::names::qn;
use crate::odf::{Element, Ns};
use crate::package::{OdpPackage, Reference, resolve_reference};
use crate::styles::unquote;

/// A DEFLATE entry: its compressed bytes, inflated on the first read (and only once). An entry over the size limit
/// holds no bytes and refuses every read.
pub(crate) struct DeflatedPart {
    compressed: Vec<u8>,
    size: u64,
    inflated: OnceLock<Result<Vec<u8>, String>>,
}

impl RangeReader for DeflatedPart {
    fn len(&self) -> u64 {
        self.size
    }

    fn read_at(&self, offset: u64, len: usize) -> Result<Cow<'_, [u8]>, RangeError> {
        let failed = |message: String| RangeError::Io {
            offset,
            len: u64::try_from(len).unwrap_or(u64::MAX),
            message,
        };
        let inflated = self.inflated.get_or_init(|| {
            if self.compressed.is_empty() && self.size > 0 {
                return Err("the entry is over the size limit".to_owned());
            }
            let size = usize::try_from(self.size).map_err(|error| error.to_string())?;
            inflate_raw_exact(&self.compressed, size).map_err(|error| error.to_string())
        });
        let bytes = inflated
            .as_ref()
            .map_err(|message| failed(message.clone()))?;
        let start = usize::try_from(offset).map_err(|error| failed(error.to_string()))?;
        start
            .checked_add(len)
            .and_then(|end| bytes.get(start..end))
            .map(Cow::Borrowed)
            .ok_or_else(|| failed("past the end of the entry".to_owned()))
    }
}

/// An entry's bytes as the font stage reads them; `None` for an encrypted entry, another method or a corrupt one.
fn entry_source(
    archive: &ZipArchive<&dyn RangeReader>,
    entry: &ZipEntry,
    limit: u64,
) -> Option<MediaSource> {
    if entry.encrypted {
        return None;
    }
    match entry.method {
        STORED => match archive.open_entry(entry).ok()? {
            EntryReader::Slice(slice) => Some(MediaSource::InputRange {
                offset: slice.offset(),
                len: entry.size,
            }),
            EntryReader::Memory(_) => None,
        },
        DEFLATE => {
            let over = entry.size > limit || entry.compressed_size > limit;
            let compressed = if over {
                Vec::new()
            } else {
                compressed_bytes(*archive.reader(), entry)?
            };
            Some(MediaSource::Reader(Arc::new(DeflatedPart {
                compressed,
                size: entry.size,
                inflated: OnceLock::new(),
            })))
        }
        _ => None,
    }
}

/// The compressed bytes of an entry, after its local header.
fn compressed_bytes(reader: &dyn RangeReader, entry: &ZipEntry) -> Option<Vec<u8>> {
    let header = reader
        .read_at(
            entry.local_header_offset,
            usize::try_from(LOCAL_HEADER_SIZE).ok()?,
        )
        .ok()?;
    if header.get(..4) != Some(&[0x50, 0x4b, 0x03, 0x04]) {
        return None;
    }
    let u16_at = |at: usize| -> Option<u64> {
        let bytes = header.get(at..at + 2)?;
        Some(u64::from(u16::from_le_bytes([
            *bytes.first()?,
            *bytes.get(1)?,
        ])))
    };
    let data = entry
        .local_header_offset
        .checked_add(LOCAL_HEADER_SIZE)?
        .checked_add(u16_at(26)?)?
        .checked_add(u16_at(28)?)?;
    let len = usize::try_from(entry.compressed_size).ok()?;
    reader.read_at(data, len).ok().map(Cow::into_owned)
}

/// `loext:font-weight`: `normal`, `bold` or 100 to 900.
fn weight_of(value: Option<&str>) -> u32 {
    match value {
        Some("bold") => 700,
        Some(value) => value
            .parse::<u32>()
            .ok()
            .filter(|weight| (100..=900).contains(weight))
            .unwrap_or(400),
        None => 400,
    }
}

fn container_of(format: Option<&str>) -> Option<FontContainer> {
    match format {
        None | Some("truetype" | "opentype" | "truetype-collection") => Some(FontContainer::Sfnt),
        Some("woff") => Some(FontContainer::Woff),
        Some("woff2") => Some(FontContainer::Woff2),
        Some("embedded-opentype") => Some(FontContainer::Eot),
        Some(_) => None,
    }
}

/// The container a file with no format attribute is, from the manifest's media type, else its extension.
fn container_without_format(media_type: Option<&str>, path: &str) -> FontContainer {
    let extension = path
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase());
    match (media_type, extension.as_deref()) {
        (Some("font/woff" | "application/font-woff" | "application/x-font-woff"), _)
        | (_, Some("woff")) => FontContainer::Woff,
        (Some("font/woff2" | "application/font-woff2"), _) | (_, Some("woff2")) => {
            FontContainer::Woff2
        }
        (Some("application/vnd.ms-fontobject"), _) | (_, Some("eot")) => FontContainer::Eot,
        _ => FontContainer::Sfnt,
    }
}

/// The fonts the package embeds, each file once with every face declared for it (LibreOffice declares one file for
/// several slots when the family lacks a face), in declaration order (content.xml's, then styles.xml's). A
/// declaration with no `loext:font-weight` or `loext:font-style` (older LibreOffice) declares no face: the file's own
/// tables say which face it is. An entry over `limit` is handed on unread.
pub(crate) fn embedded_fonts(
    pkg: &OdpPackage<'_>,
    roots: &[Option<&Element>],
    limit: u64,
) -> Vec<EmbeddedFont> {
    let mut out: Vec<(String, EmbeddedFont)> = Vec::new();
    for root in roots.iter().flatten() {
        let decls = root.child(Ns::Office, "font-face-decls");
        for face in decls
            .into_iter()
            .flat_map(|d| d.children_of(Ns::Style, "font-face"))
        {
            let family = Some(unquote(face.attr(qn!("svg:font-family")).unwrap_or("")))
                .filter(|family| !family.is_empty())
                .or_else(|| face.attr(qn!("style:name")).map(str::to_owned))
                .unwrap_or_default();
            if family.is_empty() {
                continue;
            }
            let uris = face
                .children_of(Ns::Svg, "font-face-src")
                .flat_map(|src| src.children_of(Ns::Svg, "font-face-uri"));
            for uri in uris {
                let Reference::Internal(path) =
                    resolve_reference(uri.attr(qn!("xlink:href")).unwrap_or(""))
                else {
                    continue;
                };
                let format = uri
                    .child(Ns::Svg, "font-face-format")
                    .and_then(|f| f.attr(qn!("svg:string")));
                let container = match format {
                    Some(_) => match container_of(format) {
                        Some(container) => container,
                        None => continue,
                    },
                    None => container_without_format(pkg.declared_type(&path), &path),
                };
                let weight = uri.attr(qn!("loext:font-weight"));
                let style = uri.attr(qn!("loext:font-style"));
                let face = (weight.is_some() || style.is_some()).then(|| FaceRequest {
                    weight: weight_of(weight),
                    style: match style {
                        Some("italic" | "oblique") => FaceStyle::Italic,
                        _ => FaceStyle::Normal,
                    },
                });
                if let Some((_, font)) = out.iter_mut().find(|(seen, _)| *seen == path) {
                    if let Some(face) = face
                        && font_key(&font.family) == font_key(&family)
                        && !font.faces.contains(&face)
                    {
                        font.faces.push(face);
                    }
                    continue;
                }
                let Some(source) = pkg
                    .entry(&path)
                    .and_then(|entry| entry_source(&pkg.archive, entry, limit))
                else {
                    continue;
                };
                out.push((
                    path,
                    EmbeddedFont {
                        family: family.clone(),
                        faces: face.into_iter().collect(),
                        container,
                        source,
                    },
                ));
            }
        }
    }
    out.into_iter().map(|(_, font)| font).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_declared_weights_and_formats() {
        assert_eq!(weight_of(Some("bold")), 700);
        assert_eq!(weight_of(Some("300")), 300);
        assert_eq!(weight_of(Some("1000")), 400);
        assert_eq!(weight_of(None), 400);
        assert_eq!(container_of(None), Some(FontContainer::Sfnt));
        assert_eq!(container_of(Some("woff2")), Some(FontContainer::Woff2));
        assert_eq!(container_of(Some("svg")), None);
        assert_eq!(
            container_without_format(None, "Fonts/a.ttf"),
            FontContainer::Sfnt
        );
        assert_eq!(
            container_without_format(Some("font/woff2"), "Fonts/a"),
            FontContainer::Woff2
        );
        assert_eq!(
            container_without_format(None, "Fonts/a.WOFF"),
            FontContainer::Woff
        );
    }
}
