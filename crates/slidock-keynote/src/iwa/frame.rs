//! An IWA file's framing: chunks `[0x00][uint24 LE length][block]`, each block one raw Snappy block (the format's
//! "block" layer alone: no stream identifier, no CRC). Blocks decompress with `snap::raw`; their concatenation is
//! the object stream.

use crate::iwa::error::{IwaError, corrupt};

/// A raw block's uncompressed length, the varint it starts with (at most 5 bytes), and where its body starts.
pub fn snappy_length(block: &[u8]) -> Result<(u64, usize), IwaError> {
    let mut length: u64 = 0;
    for (pos, shift) in (0..=28).step_by(7).enumerate() {
        let byte = *block.get(pos).ok_or_else(|| snappy_corrupt("bad length"))?;
        length += u64::from(byte & 0x7f) << shift;
        if byte < 0x80 {
            return Ok((length, pos + 1));
        }
    }
    Err(snappy_corrupt("bad length"))
}

fn snappy_corrupt(message: &str) -> IwaError {
    corrupt(format!("Snappy: {message}"))
}

/// One raw block decompressed into `out`, which is exactly its uncompressed length.
pub fn decompress_snappy(block: &[u8], out: &mut [u8]) -> Result<(), IwaError> {
    let (length, start) = snappy_length(block)?;
    if length != out.len() as u64 {
        return Err(snappy_corrupt(&format!(
            "a block of {length} bytes, expected {}",
            out.len()
        )));
    }
    snap::raw::Decoder::new()
        .decompress(block, out)
        .map(|_| ())
        .map_err(|_| snappy_corrupt(&refusal(block.get(start..).unwrap_or_default(), length)))
}

/// Why a block `snap` refused is damaged, in the reference decoder's words: its elements walked as that decoder
/// walks them, without writing the output.
fn refusal(body: &[u8], length: u64) -> String {
    let byte = |at: usize| u64::from(body.get(at).copied().unwrap_or(0));
    let little_endian = |at: usize, n: usize| (0..n).fold(0u64, |v, i| v | byte(at + i) << (8 * i));
    let end = body.len();
    let (mut pos, mut op) = (0usize, 0u64);
    while pos < end {
        let tag = byte(pos);
        pos += 1;
        let (copy, offset, width) = match tag & 3 {
            0 => {
                let mut literal = tag >> 2;
                if literal >= 60 {
                    let width = (literal - 59) as usize;
                    if pos + width > end {
                        return "truncated literal length".into();
                    }
                    literal = little_endian(pos, width);
                    pos += width;
                }
                let literal = literal + 1;
                if pos as u64 + literal > end as u64 || op + literal > length {
                    return "literal out of bounds".into();
                }
                pos += literal as usize;
                op += literal;
                continue;
            }
            1 => (((tag >> 2) & 7) + 4, (tag >> 5) << 8 | byte(pos), 1),
            2 => ((tag >> 2) + 1, little_endian(pos, 2), 2),
            _ => ((tag >> 2) + 1, little_endian(pos, 4), 4),
        };
        if pos + width > end {
            return "truncated copy".into();
        }
        pos += width;
        if offset == 0 || offset > op || op + copy > length {
            return "copy out of bounds".into();
        }
        op += copy;
    }
    format!("wrote {op} bytes, expected {length}")
}

/// The concatenated uncompressed chunks of an IWA file, at most `max_bytes`. The chunks' declared lengths are
/// summed and checked before any is decompressed, and the output is allocated once.
pub fn unframe_iwa(bytes: &[u8], file: &str, max_bytes: u64) -> Result<Vec<u8>, IwaError> {
    let mut total: u64 = 0;
    for block in blocks(bytes, file) {
        total += snappy_length(block?)?.0;
        if total > max_bytes {
            return Err(IwaError::TooLarge(format!(
                "{file} decompresses past the {max_bytes} bytes left for the index"
            )));
        }
    }
    let total = usize::try_from(total).map_err(|_| {
        IwaError::TooLarge(format!(
            "{file} decompresses past the {max_bytes} bytes left for the index"
        ))
    })?;
    let mut out = vec![0u8; total];
    let mut offset = 0;
    for block in blocks(bytes, file) {
        let block = block?;
        let (length, _) = snappy_length(block)?;
        // Summed above, so every length fits in `total`.
        let end = offset + length as usize;
        let target = out
            .get_mut(offset..end)
            .ok_or_else(|| corrupt(format!("{file}: a chunk past the declared total")))?;
        decompress_snappy(block, target)?;
        offset = end;
    }
    Ok(out)
}

/// Each chunk's Snappy block, or why the framing is damaged.
fn blocks<'a>(bytes: &'a [u8], file: &'a str) -> impl Iterator<Item = Result<&'a [u8], IwaError>> {
    let mut at = 0usize;
    let mut failed = false;
    std::iter::from_fn(move || {
        if failed || at >= bytes.len() {
            return None;
        }
        let block = chunk_at(bytes, at, file);
        match &block {
            Ok(body) => at += 4 + body.len(),
            Err(_) => failed = true,
        }
        Some(block)
    })
}

fn chunk_at<'a>(bytes: &'a [u8], at: usize, file: &str) -> Result<&'a [u8], IwaError> {
    let Some(&[kind, b0, b1, b2]) = bytes.get(at..at + 4) else {
        return Err(corrupt(format!("{file}: truncated chunk header at {at}")));
    };
    if kind != 0 {
        return Err(corrupt(format!("{file}: chunk type {kind} at {at}")));
    }
    let length = usize::from(b0) | (usize::from(b1) << 8) | (usize::from(b2) << 16);
    bytes
        .get(at + 4..at + 4 + length)
        .ok_or_else(|| corrupt(format!("{file}: truncated chunk at {at}")))
}
