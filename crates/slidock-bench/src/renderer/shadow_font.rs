//! The fonts lane's shadow face: a valid TrueType font whose `cmap` maps no code point, built here byte by byte. A
//! family the deck ships no face of is given this face, so the browser never takes an installed font of that name:
//! every character falls through to the next family of the stack, as it does in a renderer without that font. A
//! character no font has is drawn with the first family's `.notdef`, which is this face's (empty) glyph; the lane
//! counts such glyphs as the renderer's fallback.

/// The face's PostScript name, which `CSS.getPlatformFontsForNode` reports for the glyphs it draws.
pub const SHADOW_POSTSCRIPT_NAME: &str = "SlidockBenchShadow-Regular";
const FAMILY: &str = "Slidock Bench Shadow";

fn u16s(out: &mut Vec<u8>, values: &[u16]) {
    for value in values {
        out.extend_from_slice(&value.to_be_bytes());
    }
}

fn i16s(out: &mut Vec<u8>, values: &[i16]) {
    for value in values {
        out.extend_from_slice(&value.to_be_bytes());
    }
}

fn u32s(out: &mut Vec<u8>, values: &[u32]) {
    for value in values {
        out.extend_from_slice(&value.to_be_bytes());
    }
}

/// `cmap`: one Windows Unicode BMP subtable (format 4) holding only the closing segment.
fn cmap() -> Vec<u8> {
    let mut out = Vec::new();
    u16s(&mut out, &[0, 1, 3, 1]);
    u32s(&mut out, &[12]);
    // format, length, language, segCountX2, searchRange, entrySelector, rangeShift
    u16s(&mut out, &[4, 24, 0, 2, 2, 0, 0]);
    // endCode, reservedPad, startCode, idDelta, idRangeOffset
    u16s(&mut out, &[0xffff, 0, 0xffff, 1, 0]);
    out
}

/// `head`, with `checkSumAdjustment` zero until the font is assembled.
fn head() -> Vec<u8> {
    let mut out = Vec::new();
    u32s(&mut out, &[0x0001_0000, 0x0001_0000, 0, 0x5f0f_3cf5]);
    // flags (baseline at 0, lsb at 0, integer ppem), unitsPerEm
    u16s(&mut out, &[0x000b, 1000]);
    // created, modified
    u32s(&mut out, &[0, 0, 0, 0]);
    // xMin, yMin, xMax, yMax
    i16s(&mut out, &[0, 0, 0, 0]);
    // macStyle, lowestRecPPEM, fontDirectionHint, indexToLocFormat (short), glyphDataFormat
    u16s(&mut out, &[0, 8, 2, 0, 0]);
    out
}

fn hhea() -> Vec<u8> {
    let mut out = Vec::new();
    u32s(&mut out, &[0x0001_0000]);
    // ascender, descender, lineGap
    i16s(&mut out, &[800, -200, 0]);
    // advanceWidthMax
    u16s(&mut out, &[0]);
    // minLeftSideBearing, minRightSideBearing, xMaxExtent, caretSlopeRise, caretSlopeRun, caretOffset, 4 reserved,
    // metricDataFormat
    i16s(&mut out, &[0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]);
    // numberOfHMetrics
    u16s(&mut out, &[1]);
    out
}

fn maxp() -> Vec<u8> {
    let mut out = Vec::new();
    u32s(&mut out, &[0x0001_0000]);
    // numGlyphs, maxPoints, maxContours, maxCompositePoints, maxCompositeContours, maxZones, maxTwilightPoints,
    // maxStorage, maxFunctionDefs, maxInstructionDefs, maxStackElements, maxSizeOfInstructions, maxComponentElements,
    // maxComponentDepth
    u16s(&mut out, &[1, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0]);
    out
}

/// `OS/2` version 4: installable embedding, regular, the line metrics of `hhea`.
fn os2() -> Vec<u8> {
    let mut out = Vec::new();
    // version, xAvgCharWidth, usWeightClass, usWidthClass, fsType
    u16s(&mut out, &[4, 0, 400, 5, 0]);
    // subscript and superscript sizes and offsets, strikeout size and position, sFamilyClass
    i16s(&mut out, &[650, 600, 0, 75, 650, 600, 0, 350, 50, 250, 0]);
    // panose
    out.extend_from_slice(&[0; 10]);
    // ulUnicodeRange1-4
    u32s(&mut out, &[0, 0, 0, 0]);
    out.extend_from_slice(b"SDBK");
    // fsSelection (REGULAR, USE_TYPO_METRICS), usFirstCharIndex, usLastCharIndex
    u16s(&mut out, &[0x00c0, 0xffff, 0xffff]);
    // sTypoAscender, sTypoDescender, sTypoLineGap
    i16s(&mut out, &[800, -200, 0]);
    // usWinAscent, usWinDescent
    u16s(&mut out, &[800, 200]);
    // ulCodePageRange1-2
    u32s(&mut out, &[1, 0]);
    // sxHeight, sCapHeight
    i16s(&mut out, &[500, 700]);
    // usDefaultChar, usBreakChar, usMaxContext
    u16s(&mut out, &[0, 32, 0]);
    out
}

/// One metric: the `.notdef` glyph advances nothing.
fn hmtx() -> Vec<u8> {
    vec![0, 0, 0, 0]
}

/// `.notdef`: a simple glyph with no contour.
fn glyf() -> Vec<u8> {
    let mut out = Vec::new();
    // numberOfContours, xMin, yMin, xMax, yMax, instructionLength
    i16s(&mut out, &[0, 0, 0, 0, 0, 0]);
    out
}

/// Short offsets (halved): the one glyph spans the whole `glyf`.
fn loca(glyf_len: usize) -> Vec<u8> {
    let mut out = Vec::new();
    u16s(&mut out, &[0, (glyf_len / 2) as u16]);
    out
}

/// Windows Unicode English names: family, subfamily, full name, PostScript name.
fn name() -> Vec<u8> {
    let records: [(u16, String); 4] = [
        (1, FAMILY.to_owned()),
        (2, "Regular".to_owned()),
        (4, format!("{FAMILY} Regular")),
        (6, SHADOW_POSTSCRIPT_NAME.to_owned()),
    ];
    let mut strings = Vec::new();
    let mut out = Vec::new();
    u16s(
        &mut out,
        &[0, records.len() as u16, (6 + 12 * records.len()) as u16],
    );
    for (id, text) in &records {
        let encoded: Vec<u8> = text.encode_utf16().flat_map(u16::to_be_bytes).collect();
        u16s(
            &mut out,
            &[
                3,
                1,
                0x0409,
                *id,
                encoded.len() as u16,
                strings.len() as u16,
            ],
        );
        strings.extend(encoded);
    }
    out.extend(strings);
    out
}

/// `post` version 3: no glyph names.
fn post() -> Vec<u8> {
    let mut out = Vec::new();
    u32s(&mut out, &[0x0003_0000, 0]);
    i16s(&mut out, &[-100, 50]);
    u32s(&mut out, &[0, 0, 0, 0, 0]);
    out
}

fn checksum(data: &[u8]) -> u32 {
    data.chunks(4).fold(0u32, |sum, chunk| {
        let mut word = [0u8; 4];
        for (to, from) in word.iter_mut().zip(chunk) {
            *to = *from;
        }
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}

/// The shadow face's bytes: the table directory in tag order, every table 4-byte padded, checksums set.
pub fn shadow_font() -> Vec<u8> {
    let glyf = glyf();
    let mut tables: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"OS/2", os2()),
        (b"cmap", cmap()),
        (b"glyf", glyf.clone()),
        (b"head", head()),
        (b"hhea", hhea()),
        (b"hmtx", hmtx()),
        (b"loca", loca(glyf.len())),
        (b"maxp", maxp()),
        (b"name", name()),
        (b"post", post()),
    ];
    tables.sort_by_key(|(tag, _)| **tag);
    let count = tables.len() as u16;
    let mut power = 1u16;
    let mut selector = 0u16;
    while power * 2 <= count {
        power *= 2;
        selector += 1;
    }
    let mut out = Vec::new();
    u32s(&mut out, &[0x0001_0000]);
    u16s(
        &mut out,
        &[count, power * 16, selector, count * 16 - power * 16],
    );
    let mut offset = 12 + 16 * tables.len();
    let mut body = Vec::new();
    let mut head_at = 0;
    for (tag, data) in &tables {
        if *tag == b"head" {
            head_at = offset;
        }
        out.extend_from_slice(*tag);
        u32s(
            &mut out,
            &[checksum(data), offset as u32, data.len() as u32],
        );
        body.extend_from_slice(data);
        let padded = data.len().next_multiple_of(4);
        body.resize(body.len() + padded - data.len(), 0);
        offset += padded;
    }
    out.extend(body);
    let adjustment = 0xb1b0_afbau32.wrapping_sub(checksum(&out));
    if let Some(slot) = out.get_mut(head_at + 8..head_at + 12) {
        slot.copy_from_slice(&adjustment.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shadow_face_reads_back_and_maps_no_code_point() {
        let bytes = shadow_font();
        let faces = slidock_fonts::read_sfnt(&bytes).unwrap();
        assert_eq!(faces.len(), 1);
        assert_eq!(faces[0].postscript_name, SHADOW_POSTSCRIPT_NAME);
        assert_eq!(faces[0].family, FAMILY);
        assert_eq!(slidock_fonts::sfnt::sanitizer_refusal(&bytes, 0), None);
        // No code point the deck could draw has a glyph here.
        assert_eq!(
            slidock_fonts::resolve::missing_code_points(
                &bytes,
                0,
                [0x20, 0x41, 0xe9, 0x4e00, 0x1f600]
            ),
            5
        );
        assert_eq!(checksum(&bytes), 0xb1b0_afba);
    }
}
