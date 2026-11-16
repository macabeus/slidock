//! Embedded fonts (`p:embeddedFontLst`): each `p:regular|p:bold|p:italic|p:boldItalic` part handed to the font stage
//! as it is in the package, with the family and face the presentation declares for it. The font stage unwraps the
//! part (`.fntdata` is Embedded OpenType, usually MicroType Express compressed) and checks its identity. A STORED
//! part is a range of the input; a DEFLATE part is kept compressed and inflated only when the font stage reads it.

use std::borrow::Cow;
use std::sync::Arc;
use std::sync::OnceLock;

use slidock_core_api::frontend::FontContainer;
use slidock_core_api::{EmbeddedFont, MediaSource};
use slidock_io::zip::{DEFLATE, EntryReader, LOCAL_HEADER_SIZE, STORED};
use slidock_io::{RangeError, RangeReader, ZipArchive, ZipEntry, inflate_raw_exact};

use crate::package::Package;
use crate::presentation::EmbeddedFontEntry;

/// A DEFLATE part: its compressed bytes, inflated on the first read (and only once). A part over the size limit
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
                return Err("the part is over the size limit".to_owned());
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
            .ok_or_else(|| failed("past the end of the part".to_owned()))
    }
}

/// A part's bytes as the font stage reads them: a range of the input when STORED, the compressed bytes when
/// DEFLATE. `None` for an encrypted part, another method or a corrupt entry.
pub(crate) fn part_source(
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

/// The container a part's name suggests: `.fntdata` (and anything unknown) is Embedded OpenType.
fn container_of(part: &str) -> Option<FontContainer> {
    let extension = part
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase());
    match extension.as_deref() {
        Some("ttf" | "otf" | "ttc" | "otc") => Some(FontContainer::Sfnt),
        Some("woff") => Some(FontContainer::Woff),
        Some("woff2") => Some(FontContainer::Woff2),
        // A GUID-obfuscated part needs its key, which PresentationML does not carry.
        Some("odttf") => None,
        _ => Some(FontContainer::Eot),
    }
}

/// The embedded font parts, each part once (with every face that names it), in declaration order.
pub(crate) fn embedded_fonts(
    pkg: &Package<'_>,
    entries: &[EmbeddedFontEntry],
    limit: u64,
) -> Vec<EmbeddedFont> {
    let mut out: Vec<(String, EmbeddedFont)> = Vec::new();
    for entry in entries {
        for (face, part) in &entry.parts {
            if let Some((_, font)) = out
                .iter_mut()
                .find(|(seen, font)| seen == part && font.family == entry.family)
            {
                if !font.faces.contains(face) {
                    font.faces.push(face.clone());
                }
                continue;
            }
            let Some(container) = container_of(part) else {
                continue;
            };
            let Some(source) = pkg
                .entry(part)
                .and_then(|zip| part_source(pkg.archive(), zip, limit))
            else {
                continue;
            };
            out.push((
                part.clone(),
                EmbeddedFont {
                    family: entry.family.clone(),
                    faces: vec![face.clone()],
                    container,
                    source,
                },
            ));
        }
    }
    out.into_iter().map(|(_, font)| font).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_container_by_extension() {
        assert_eq!(
            container_of("ppt/fonts/Roboto-regular.fntdata"),
            Some(FontContainer::Eot)
        );
        assert_eq!(
            container_of("ppt/fonts/font1.TTF"),
            Some(FontContainer::Sfnt)
        );
        assert_eq!(
            container_of("ppt/fonts/a.woff2"),
            Some(FontContainer::Woff2)
        );
        assert_eq!(container_of("ppt/fonts/{GUID}.odttf"), None);
        assert_eq!(container_of("ppt/fonts/noext"), Some(FontContainer::Eot));
    }

    #[test]
    fn a_deflated_part_inflates_once_and_refuses_when_over_the_limit() {
        let data = b"font bytes, font bytes, font bytes".to_vec();
        let compressed = miniz_oxide::deflate::compress_to_vec(&data, 6);
        let part = DeflatedPart {
            compressed,
            size: data.len() as u64,
            inflated: OnceLock::new(),
        };
        assert_eq!(part.read_at(0, data.len()).unwrap().as_ref(), &data[..]);
        assert_eq!(part.read_at(5, 5).unwrap().as_ref(), &data[5..10]);
        assert!(part.read_at(30, 10).is_err());
        let over = DeflatedPart {
            compressed: Vec::new(),
            size: 10,
            inflated: OnceLock::new(),
        };
        assert!(over.read_at(0, 1).is_err());
    }
}
