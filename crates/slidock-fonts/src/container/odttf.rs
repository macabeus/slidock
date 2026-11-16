//! Obfuscated OOXML fonts (ECMA-376 Part 1 §17.8.1): the first 32 bytes of a TrueType/OpenType file XORed with a
//! GUID's 16 bytes, taken in reverse of their written order. WordprocessingML names the key in `w:fontKey`; XPS and
//! other writers name the part after it, which is the only key source a PPTX has.

use super::{ContainerError, Format, Hint, sniff};

const KEY_BYTES: usize = 16;
const OBFUSCATED_BYTES: usize = 32;

/// Why an obfuscated font can't be read.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum OdttfError {
    #[error("The obfuscated font's part name is not a GUID")]
    NoKey,
    #[error("The font key is not a GUID")]
    BadGuid,
    #[error("The font is shorter than its 32 obfuscated bytes")]
    Truncated,
    #[error("The font key does not unlock the font")]
    BadKey,
}

/// A GUID's 16 bytes in written order, from `{XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}` (braces and dashes optional,
/// either case).
pub fn parse_guid(text: &str) -> Option<[u8; KEY_BYTES]> {
    let inner = text
        .strip_prefix('{')
        .and_then(|t| t.strip_suffix('}'))
        .unwrap_or(text);
    let dashed = inner.len() == 36;
    if dashed
        && !inner
            .char_indices()
            .all(|(i, c)| (c == '-') == matches!(i, 8 | 13 | 18 | 23))
    {
        return None;
    }
    let digits: Vec<u8> = inner.bytes().filter(|&b| b != b'-').collect();
    if digits.len() != 2 * KEY_BYTES
        || (!dashed && inner.len() != 2 * KEY_BYTES)
        || !digits.iter().all(u8::is_ascii_hexdigit)
    {
        return None;
    }
    let mut guid = [0u8; KEY_BYTES];
    for (byte, pair) in guid.iter_mut().zip(digits.as_chunks::<2>().0) {
        let pair = std::str::from_utf8(pair).ok()?;
        *byte = u8::from_str_radix(pair, 16).ok()?;
    }
    Some(guid)
}

/// The key of a part named after a GUID, such as `word/fonts/{6C2E…}.odttf`.
pub fn key_from_part_name(part_name: &str) -> Option<[u8; KEY_BYTES]> {
    let file = part_name.rsplit(['/', '\\']).next().unwrap_or(part_name);
    let stem = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
    parse_guid(stem)
}

/// XORs the first 32 bytes of `font` with the key; the same call obfuscates and deobfuscates.
pub fn xor(font: &mut [u8], guid: &[u8; KEY_BYTES]) -> Result<(), OdttfError> {
    let head = font
        .get_mut(..OBFUSCATED_BYTES)
        .ok_or(OdttfError::Truncated)?;
    for (i, byte) in head.iter_mut().enumerate() {
        let key = guid
            .get(KEY_BYTES - 1 - i % KEY_BYTES)
            .copied()
            .unwrap_or(0);
        *byte ^= key;
    }
    Ok(())
}

/// Deobfuscates `font` in place. The result must start like a TrueType/OpenType font or collection, or the key
/// was wrong (and `font` is left as it came).
pub fn deobfuscate(font: &mut [u8], guid: &[u8; KEY_BYTES]) -> Result<(), ContainerError> {
    xor(font, guid)?;
    match sniff(font, &Hint::default()) {
        Some(Format::Sfnt | Format::Collection) => Ok(()),
        _ => {
            xor(font, guid)?;
            Err(OdttfError::BadKey.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GUID: &str = "{0A1B2C3D-4E5F-6071-8293-A4B5C6D7E8F9}";

    #[test]
    fn guids_parse_with_or_without_braces_and_dashes() {
        let bytes = [
            0x0a, 0x1b, 0x2c, 0x3d, 0x4e, 0x5f, 0x60, 0x71, 0x82, 0x93, 0xa4, 0xb5, 0xc6, 0xd7,
            0xe8, 0xf9,
        ];
        assert_eq!(parse_guid(GUID), Some(bytes));
        assert_eq!(parse_guid(&GUID.to_lowercase()), Some(bytes));
        assert_eq!(parse_guid("0A1B2C3D4E5F60718293A4B5C6D7E8F9"), Some(bytes));
        assert_eq!(parse_guid("{0A1B2C3D-4E5F-6071-8293-A4B5C6D7E8F}"), None);
        assert_eq!(parse_guid("{0A1B2C3D4-E5F-6071-8293-A4B5C6D7E8F9}"), None);
        assert_eq!(parse_guid("0A1B2C3D-4E5F60718293A4B5C6D7E8F9"), None);
        assert_eq!(parse_guid("{ZA1B2C3D-4E5F-6071-8293-A4B5C6D7E8F9}"), None);
        assert_eq!(parse_guid("+A1B2C3D4E5F60718293A4B5C6D7E8F9"), None);
    }

    #[test]
    fn the_key_comes_from_the_file_stem() {
        assert!(key_from_part_name(&format!("word/fonts/{GUID}.odttf")).is_some());
        assert!(key_from_part_name(&format!("{GUID}.ODTTF")).is_some());
        assert_eq!(key_from_part_name("ppt/fonts/font1.odttf"), None);
    }

    #[test]
    fn xor_uses_the_guid_bytes_in_reverse_order_over_32_bytes() {
        let guid = parse_guid(GUID).unwrap();
        let mut font = vec![0u8; 40];
        xor(&mut font, &guid).unwrap();
        assert_eq!(font[0], 0xf9);
        assert_eq!(font[15], 0x0a);
        assert_eq!(font[16], 0xf9);
        assert_eq!(font[31], 0x0a);
        assert_eq!(&font[32..], &[0; 8]);
        assert_eq!(xor(&mut [0; 31], &guid), Err(OdttfError::Truncated));
    }
}
