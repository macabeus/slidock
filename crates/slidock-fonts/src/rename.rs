//! Neutral names for a subset whose licence reserves the face's names. A subset is a modified version of its face
//! (OFL-FAQ 2.6: removing glyphs or layout from a web font modifies it), and some licences forbid a modified version
//! their names: the SIL Open Font License its Reserved Font Names, which the copyright or licence description
//! (name ids 0 and 13) declares ("with Reserved Font Name …"), and the Bitstream Vera licence the words "Bitstream"
//! and "Vera".
//!
//! Such a subset is renamed after a hash of its names: family "Subset 1a2b3c4d" (name ids 1 and 16, each hashed from
//! its own string, so faces that shared a family still do), full name family + subfamily (4), and PostScript name and
//! unique id `Subset1a2b3c4d-Bold` (6, 3). The subfamily (2, 17), copyright (0), version (5) and licence (13, 14) stay.
//! A deck's `@font-face` rules name the family the slides use, whatever the file's own names, so the face draws as
//! before. A name table that cannot be rewritten fails the subset, and the face ships whole, unmodified.

use read_fonts::tables::name::Encoding;
use read_fonts::types::Tag;
use read_fonts::{FontRef, TableProvider};
use write_fonts::FontBuilder;

use crate::sfnt::FaceDetails;
use crate::subset::SubsetError;

const FAMILY: u16 = 1;
const SUBFAMILY: u16 = 2;
const UNIQUE_ID: u16 = 3;
const FULL_NAME: u16 = 4;
const POSTSCRIPT: u16 = 6;
const TYPOGRAPHIC_FAMILY: u16 = 16;
const TYPOGRAPHIC_SUBFAMILY: u16 = 17;

/// The longest PostScript name (OpenType `name` id 6).
const POSTSCRIPT_MAX: usize = 63;

/// Whether a modified version of the face may not keep its names: its copyright or licence description (`details`)
/// reserves font names, or its licence (an SPDX id) is Bitstream Vera's and `names` (its own) hold "Bitstream" or
/// "Vera".
pub fn reserves_names<'a>(
    details: &FaceDetails,
    licence: Option<&str>,
    names: impl IntoIterator<Item = &'a str>,
) -> bool {
    let declares = |text: &Option<String>| {
        text.as_deref()
            .is_some_and(|text| text.to_ascii_lowercase().contains("reserved font"))
    };
    if declares(&details.copyright) || declares(&details.licence) {
        return true;
    }
    licence == Some("Bitstream-Vera")
        && names.into_iter().any(|name| {
            let lower = name.to_ascii_lowercase();
            lower.contains("bitstream") || lower.contains("vera")
        })
}

/// A subset as it ships: with neutral names when its face's licence reserves them ([`reserves_names`]), else as it
/// is.
pub fn shipped_names(
    sfnt: Vec<u8>,
    details: &FaceDetails,
    licence: Option<&str>,
) -> Result<Vec<u8>, SubsetError> {
    let names = NameTable::read(&sfnt)?;
    let own: Vec<&str> = [FAMILY, FULL_NAME, POSTSCRIPT, TYPOGRAPHIC_FAMILY]
        .into_iter()
        .filter_map(|id| names.get(id))
        .collect();
    if !reserves_names(details, licence, own) {
        return Ok(sfnt);
    }
    let renamed = names.neutral()?;
    replace_name(&sfnt, &renamed)
}

/// FNV-1a: a stable 32-bit hash of a name.
fn hash(name: &str) -> u32 {
    name.bytes().fold(0x811c_9dc5, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
    })
}

fn neutral_family(name: &str) -> String {
    format!("Subset {:08x}", hash(name))
}

/// The ASCII letters and digits of `name`.
fn alphanumeric(name: &str) -> String {
    name.chars().filter(char::is_ascii_alphanumeric).collect()
}

/// One record: its platform, encoding, language and name id, and the string's bytes.
struct Record {
    platform: u16,
    encoding: u16,
    language: u16,
    id: u16,
    bytes: Vec<u8>,
}

/// A format 0 `name` table, and the decoded string of each id (its Windows English record first).
struct NameTable {
    records: Vec<Record>,
    strings: Vec<(u16, String)>,
}

fn bad(what: &str) -> SubsetError {
    SubsetError::BadOutput(format!("the name table {what}"))
}

impl NameTable {
    fn read(sfnt: &[u8]) -> Result<NameTable, SubsetError> {
        let font = FontRef::new(sfnt).map_err(|e| SubsetError::BadOutput(e.to_string()))?;
        let name = font.name().map_err(|_| bad("cannot be read"))?;
        if name.version() != 0 {
            return Err(bad("has language tags"));
        }
        let data = name.string_data();
        let mut records = Vec::new();
        let mut strings: Vec<(u16, u8, String)> = Vec::new();
        for record in name.name_record() {
            let start = usize::try_from(record.string_offset().to_u32())
                .map_err(|_| bad("points past its strings"))?;
            let end = start + usize::from(record.length());
            let bytes = data
                .as_bytes()
                .get(start..end)
                .ok_or_else(|| bad("points past its strings"))?
                .to_vec();
            let (platform, encoding) = (record.platform_id(), record.encoding_id());
            let id = record.name_id().to_u16();
            let rank = match (platform, Encoding::new(platform, encoding)) {
                (3, Encoding::Utf16Be) if record.language_id() == 0x409 => Some(0),
                (_, Encoding::Utf16Be) => Some(1),
                (_, Encoding::MacRoman) => Some(2),
                _ => None,
            };
            if let Some(rank) = rank
                && let Ok(text) = record.string(data)
            {
                let text: String = text.chars().collect();
                match strings.iter_mut().find(|(held, _, _)| *held == id) {
                    Some(held) if held.1 > rank => *held = (id, rank, text),
                    Some(_) => {}
                    None => strings.push((id, rank, text)),
                }
            }
            records.push(Record {
                platform,
                encoding,
                language: record.language_id(),
                id,
                bytes,
            });
        }
        Ok(NameTable {
            records,
            strings: strings
                .into_iter()
                .map(|(id, _, text)| (id, text))
                .collect(),
        })
    }

    fn get(&self, id: u16) -> Option<&str> {
        self.strings
            .iter()
            .find(|(held, _)| *held == id)
            .map(|(_, text)| text.as_str())
    }

    /// The table with neutral family, full, PostScript and unique names, in every record that holds them. A record
    /// of those ids in an encoding other than UTF-16 or Mac Roman is dropped.
    fn neutral(&self) -> Result<Vec<u8>, SubsetError> {
        let family = self
            .get(FAMILY)
            .or_else(|| self.get(TYPOGRAPHIC_FAMILY))
            .ok_or_else(|| bad("names no family"))?;
        let family = neutral_family(family);
        let typographic = self.get(TYPOGRAPHIC_FAMILY).map(neutral_family);
        let style = self
            .get(TYPOGRAPHIC_SUBFAMILY)
            .or_else(|| self.get(SUBFAMILY))
            .unwrap_or("Regular");
        let grouped = typographic.as_deref().unwrap_or(&family);
        let full = format!("{grouped} {style}");
        let mut postscript = format!("{}-{}", alphanumeric(grouped), alphanumeric(style));
        postscript.truncate(POSTSCRIPT_MAX);
        let value = |id: u16| match id {
            FAMILY => Some(family.as_str()),
            TYPOGRAPHIC_FAMILY => typographic.as_deref(),
            FULL_NAME => Some(full.as_str()),
            UNIQUE_ID | POSTSCRIPT => Some(postscript.as_str()),
            _ => None,
        };
        let mut records = Vec::new();
        for record in &self.records {
            let bytes = match value(record.id) {
                None => record.bytes.clone(),
                Some(text) => match Encoding::new(record.platform, record.encoding) {
                    Encoding::Utf16Be => text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
                    // The neutral names are ASCII, which Mac Roman encodes as itself.
                    Encoding::MacRoman => text.as_bytes().to_vec(),
                    Encoding::Unknown => continue,
                },
            };
            records.push((record, bytes));
        }
        let count = u16::try_from(records.len()).map_err(|_| bad("is too large"))?;
        let mut header = Vec::new();
        let mut storage: Vec<u8> = Vec::new();
        header.extend(0u16.to_be_bytes());
        header.extend(count.to_be_bytes());
        header.extend((6 + 12 * count).to_be_bytes());
        for (record, bytes) in records {
            let length = u16::try_from(bytes.len()).map_err(|_| bad("is too large"))?;
            let offset = u16::try_from(storage.len()).map_err(|_| bad("is too large"))?;
            for field in [
                record.platform,
                record.encoding,
                record.language,
                record.id,
                length,
                offset,
            ] {
                header.extend(field.to_be_bytes());
            }
            storage.extend(bytes);
        }
        header.extend(storage);
        Ok(header)
    }
}

/// `sfnt` with `name` as its name table, every other table as it is.
fn replace_name(sfnt: &[u8], name: &[u8]) -> Result<Vec<u8>, SubsetError> {
    let font = FontRef::new(sfnt).map_err(|e| SubsetError::BadOutput(e.to_string()))?;
    let name_tag = Tag::new(b"name");
    let mut builder = FontBuilder::new();
    for record in font.table_directory().table_records() {
        let tag = record.tag();
        if tag == name_tag {
            continue;
        }
        let data = font
            .table_data(tag)
            .ok_or_else(|| SubsetError::BadOutput(format!("the {tag} table cannot be read")))?;
        builder.add_raw(tag, data.as_bytes().to_vec());
    }
    builder.add_raw(name_tag, name.to_vec());
    Ok(builder.build())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_neutral_family_is_a_stable_hash_of_the_name() {
        assert_eq!(neutral_family("Carlito"), neutral_family("Carlito"));
        assert_ne!(neutral_family("Carlito"), neutral_family("Caladea"));
        assert!(neutral_family("Carlito").starts_with("Subset "));
        assert_eq!(hash(""), 0x811c_9dc5);
        assert_eq!(hash("a"), 0xe40c_292c);
    }
}
