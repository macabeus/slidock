//! The `name` table: one string per name id, from the best-ranked record.

use std::collections::BTreeMap;

use super::reader::{Reader, Table};
use super::{Region, SfntError, TableTag};

const WINDOWS_ENGLISH_US: u16 = 0x409;

/// How good a record is for its id; lower wins, and the first record wins a tie.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Rank {
    WindowsEnglish,
    Windows,
    Unicode,
    MacRoman,
}

#[derive(Clone, Copy)]
enum Encoding {
    Utf16Be,
    Latin1,
}

/// The rank and encoding of a record's platform: Windows Unicode (BMP or full), the Unicode platform, Mac Roman.
/// Other platforms and encodings are skipped.
fn classify(platform: u16, encoding: u16, language: u16) -> Option<(Rank, Encoding)> {
    match (platform, encoding) {
        (3, 1 | 10) if language == WINDOWS_ENGLISH_US => {
            Some((Rank::WindowsEnglish, Encoding::Utf16Be))
        }
        (3, 1 | 10) => Some((Rank::Windows, Encoding::Utf16Be)),
        (0, _) => Some((Rank::Unicode, Encoding::Utf16Be)),
        (1, 0) => Some((Rank::MacRoman, Encoding::Latin1)),
        _ => None,
    }
}

/// UTF-16BE code units; a trailing odd byte is dropped and a lone surrogate becomes U+FFFD.
fn decode_utf16be(bytes: &[u8]) -> String {
    let (units, _) = bytes.as_chunks::<2>();
    char::decode_utf16(units.iter().map(|pair| u16::from_be_bytes(*pair)))
        .map(|unit| unit.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}

/// Mac Roman strings read byte for byte as Latin-1.
fn decode_latin1(bytes: &[u8]) -> String {
    bytes.iter().copied().map(char::from).collect()
}

/// Name strings by id, each from its best-ranked record. Every record must lie inside the table, and every string
/// inside the file, whatever its platform.
pub(super) fn read_names(
    reader: &Reader<'_>,
    table: Table,
) -> Result<BTreeMap<u16, String>, SfntError> {
    let count = reader.u16(table.offset + 2)?;
    let storage = table.offset + u64::from(reader.u16(table.offset + 4)?);
    let end = table.offset + table.length;
    let mut best: BTreeMap<u16, (Rank, String)> = BTreeMap::new();
    for i in 0..u64::from(count) {
        let record = table.offset + 6 + i * 12;
        if record + 12 > end {
            return Err(SfntError::TruncatedTable(TableTag(*b"name")));
        }
        let platform = reader.u16(record)?;
        let encoding = reader.u16(record + 2)?;
        let language = reader.u16(record + 4)?;
        let id = reader.u16(record + 6)?;
        let length = u64::from(reader.u16(record + 8)?);
        let start = storage + u64::from(reader.u16(record + 10)?);
        let raw = reader.bytes(start, length, Region::NameString)?;
        let Some((rank, encoding)) = classify(platform, encoding, language) else {
            continue;
        };
        if best.get(&id).is_some_and(|(held, _)| *held <= rank) {
            continue;
        }
        let value = match encoding {
            Encoding::Utf16Be => decode_utf16be(raw),
            Encoding::Latin1 => decode_latin1(raw),
        };
        best.insert(id, (rank, value));
    }
    Ok(best
        .into_iter()
        .map(|(id, (_, value))| (id, value))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_drops_an_odd_byte_and_replaces_lone_surrogates() {
        assert_eq!(decode_utf16be(&[0, b'A', 0, b'b', 0]), "Ab");
        assert_eq!(decode_utf16be(&[0xd8, 0x3d, 0xde, 0x00]), "\u{1f600}");
        assert_eq!(decode_utf16be(&[0xd8, 0x3d, 0, b'x']), "\u{fffd}x");
    }

    #[test]
    fn mac_roman_reads_as_latin1() {
        assert_eq!(decode_latin1(&[b'R', 0xe9]), "R\u{e9}");
    }
}
