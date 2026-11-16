//! DEFLATE decoding into buffers of a known size, so a lying stream stops at what its container declared.

use miniz_oxide::inflate::TINFLStatus;
use miniz_oxide::inflate::core::{DecompressorOxide, decompress, inflate_flags};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum InflateError {
    #[error("Inflated {actual} bytes, expected {expected}")]
    Short { actual: usize, expected: usize },
    #[error("Inflated more than {expected} bytes")]
    Long { expected: usize },
    #[error("unexpected end of file")]
    Truncated,
    #[error("invalid deflate data")]
    Invalid,
    #[error("invalid zlib data")]
    Header,
}

/// Runs one decompression of all of `data` into `out`: the bytes written, or why it stopped.
fn run(data: &[u8], out: &mut [u8], flags: u32) -> (TINFLStatus, usize) {
    let mut state = Box::<DecompressorOxide>::default();
    let flags = flags | inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF;
    let (status, _, written) = decompress(&mut state, data, out, 0, flags);
    (status, written)
}

/// Raw DEFLATE `data` inflated to exactly `expected` bytes; any other length is an error.
pub fn inflate_raw_exact(data: &[u8], expected: usize) -> Result<Vec<u8>, InflateError> {
    // A stream with more to write than the buffer holds stops with output pending.
    let mut out = vec![0; expected];
    let (status, written) = run(data, &mut out, 0);
    match status {
        TINFLStatus::Done if written == expected => Ok(out),
        TINFLStatus::Done => Err(InflateError::Short {
            actual: written,
            expected,
        }),
        TINFLStatus::HasMoreOutput => Err(InflateError::Long { expected }),
        TINFLStatus::FailedCannotMakeProgress | TINFLStatus::NeedsMoreInput => {
            Err(InflateError::Truncated)
        }
        _ => Err(InflateError::Invalid),
    }
}

/// The DEFLATE window: the most output a back-reference reaches behind it.
const WINDOW: usize = 1 << 15;

/// A zlib stream inflated as fflate's `unzlibSync` inflates it into a buffer of `size` bytes: the bytes written,
/// at most `size`. Its last four bytes are the checksum, dropped unread, so a stream of nothing more than its
/// header and checksum is `size` zero bytes. A stored block past `size` is an error; compressed output past it is
/// dropped, and the stream is still read to its end. A body that is not valid DEFLATE is refused, where fflate may
/// read it.
pub fn unzlib_into(data: &[u8], size: usize) -> Result<Vec<u8>, InflateError> {
    let [cmf, flg, ..] = *data else {
        return Err(InflateError::Header);
    };
    // No preset dictionary.
    if cmf & 15 != 8 || cmf >> 4 > 7 || u16::from_be_bytes([cmf, flg]) % 31 != 0 || flg & 0x20 != 0
    {
        return Err(InflateError::Header);
    }
    let body = data
        .get(2..data.len().saturating_sub(4))
        .unwrap_or_default();
    let mut out = vec![0; size];
    if body.is_empty() {
        return Ok(out);
    }
    // Output runs through a buffer that keeps the window behind it, so a stream is read to its end in bounded
    // memory, and a back-reference before the first byte is still refused.
    let mut buffer = vec![0; 2 * WINDOW];
    let mut state = Box::<DecompressorOxide>::default();
    let flags = inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF
        | inflate_flags::TINFL_FLAG_STOP_ON_BLOCK_BOUNDARY;
    let (mut read, mut written, mut at) = (0, 0, 0);
    check_stored_block(body, 0, written, size)?;
    loop {
        let input = body.get(read..).unwrap_or_default();
        let (status, consumed, produced) = decompress(&mut state, input, &mut buffer, at, flags);
        if let (Some(target), Some(source)) = (
            out.get_mut(written.min(size)..(written + produced).min(size)),
            buffer.get(at..at + produced),
        ) {
            target.copy_from_slice(source.get(..target.len()).unwrap_or_default());
        }
        read += consumed;
        written += produced;
        at += produced;
        match status {
            TINFLStatus::Done => break,
            // The buffer is full: its last window moves to the front.
            TINFLStatus::HasMoreOutput => {
                buffer.copy_within(at.saturating_sub(WINDOW)..at, 0);
                at = at.min(WINDOW);
            }
            TINFLStatus::BlockBoundary => {
                // The next block's header starts in the bits left of the last byte read.
                let held = state
                    .block_boundary_state()
                    .map_or(0, |boundary| usize::from(boundary.num_bits));
                check_stored_block(body, (read * 8).saturating_sub(held), written, size)?;
            }
            TINFLStatus::FailedCannotMakeProgress | TINFLStatus::NeedsMoreInput => {
                return Err(InflateError::Truncated);
            }
            _ => return Err(InflateError::Invalid),
        }
    }
    out.truncate(written.min(size));
    Ok(out)
}

/// For a stored block whose header starts at bit `position` of `body`, with `written` bytes inflated before it: an
/// error when its bytes pass the end of `body` or of the `size`-byte buffer, which fflate refuses.
fn check_stored_block(
    body: &[u8],
    position: usize,
    written: usize,
    size: usize,
) -> Result<(), InflateError> {
    let byte = |at: usize| usize::from(body.get(at).copied().unwrap_or(0));
    let header = (byte(position / 8) | byte(position / 8 + 1) << 8) >> (position % 8);
    if (header >> 1) & 3 != 0 {
        return Ok(());
    }
    // The length after the header's three bits, at the next byte.
    let start = (position + 3).div_ceil(8) + 4;
    let length = byte(start - 4) | byte(start - 3) << 8;
    if start + length > body.len() {
        return Err(InflateError::Truncated);
    }
    if written + length > size {
        return Err(InflateError::Long { expected: size });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use miniz_oxide::deflate::{compress_to_vec, compress_to_vec_zlib};

    #[test]
    fn inflates_to_exactly_the_expected_size() {
        let data = compress_to_vec(&[b'y'; 5000], 6);
        assert_eq!(inflate_raw_exact(&data, 5000).unwrap(), vec![b'y'; 5000]);
        assert_eq!(
            inflate_raw_exact(&data, 4999),
            Err(InflateError::Long { expected: 4999 })
        );
        assert_eq!(
            inflate_raw_exact(&data, 5001),
            Err(InflateError::Short {
                actual: 5000,
                expected: 5001
            })
        );
        assert_eq!(
            inflate_raw_exact(&data[..data.len() - 2], 5000),
            Err(InflateError::Truncated)
        );
        assert_eq!(
            inflate_raw_exact(&[0xff; 8], 10),
            Err(InflateError::Invalid)
        );
        assert_eq!(inflate_raw_exact(&compress_to_vec(b"", 6), 0).unwrap(), b"");
    }

    #[test]
    fn reads_a_stream_to_its_end_past_a_full_buffer() {
        // A stored block of "abc", then an empty final stored block.
        let data = [
            0x00, 3, 0, 0xfc, 0xff, b'a', b'b', b'c', 0x01, 0, 0, 0xff, 0xff,
        ];
        assert_eq!(inflate_raw_exact(&data, 3).unwrap(), b"abc");
        assert_eq!(
            inflate_raw_exact(&data, 2),
            Err(InflateError::Long { expected: 2 })
        );
    }

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&text[at..at + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn inflates_a_zlib_stream_into_a_fixed_buffer_as_fflate_does() {
        let short = compress_to_vec_zlib(b"abc", 6);
        assert_eq!(unzlib_into(&short, 10).unwrap(), b"abc");
        // Compressed output past the buffer is dropped; the stream is still read to its end.
        let bomb = compress_to_vec_zlib(&vec![0; 1 << 20], 6);
        assert_eq!(unzlib_into(&bomb, 3).unwrap(), vec![0; 3]);
        let mut broken = bomb.clone();
        let at = broken.len() - 8;
        broken[at] ^= 0xff;
        assert!(unzlib_into(&broken, 3).is_err());
        // One stored block of 10 bytes must fit the buffer.
        let stored = hex("7801010a00f5ff0102030405060708090a00000000");
        assert_eq!(
            unzlib_into(&stored, 10).unwrap(),
            (1..=10).collect::<Vec<u8>>()
        );
        assert_eq!(
            unzlib_into(&stored, 9),
            Err(InflateError::Long { expected: 9 })
        );
        // 50 bytes of fixed codes, an empty stored block (a sync flush), then two more bytes of fixed codes.
        let mixed = hex("789caaa8201500000000ffffabac020086461864");
        assert_eq!(unzlib_into(&mixed, 60).unwrap().len(), 52);
        assert_eq!(
            unzlib_into(&mixed, 51).unwrap(),
            [&[b'x'; 50][..], b"y"].concat()
        );
        assert_eq!(unzlib_into(&mixed, 50).unwrap(), vec![b'x'; 50]);
        // Even an empty stored block cannot start past the buffer.
        assert_eq!(
            unzlib_into(&mixed, 49),
            Err(InflateError::Long { expected: 49 })
        );
        // Truncated stored bytes.
        assert_eq!(
            unzlib_into(&hex("7801010a00f5ff010200000000"), 20),
            Err(InflateError::Truncated)
        );
    }

    #[test]
    fn drops_the_last_four_bytes_unread_as_the_checksum() {
        let short = compress_to_vec_zlib(b"abc", 6);
        let mut wrong = short.clone();
        let last = wrong.len() - 1;
        wrong[last] ^= 0xff;
        assert_eq!(unzlib_into(&wrong, 10).unwrap(), b"abc");
        // Nothing but a header and four bytes: a buffer of zeros.
        assert_eq!(unzlib_into(&hex("789c63e70000"), 2).unwrap(), [0, 0]);
        assert_eq!(unzlib_into(&short[..5], 4).unwrap(), [0; 4]);
        // A stream short of its checksum loses its last deflate bytes.
        assert_eq!(
            unzlib_into(&short[..short.len() - 2], 10),
            Err(InflateError::Truncated)
        );
    }

    #[test]
    fn refuses_a_back_reference_before_the_first_byte() {
        // Fixed codes: length 3 at distance 1 with nothing written.
        let reference = [0x78, 0x01, 0b0000_0011, 0b0000_0010, 0, 0, 0, 0, 0];
        assert_eq!(unzlib_into(&reference, 10), Err(InflateError::Invalid));
        // A real stream reaching back across the buffer's window still inflates.
        let text: Vec<u8> = (0..200_000u32)
            .map(|i| (i % 251) as u8 ^ (i / 7000) as u8)
            .collect();
        let deflated = compress_to_vec_zlib(&text, 9);
        assert_eq!(unzlib_into(&deflated, text.len()).unwrap(), text);
        assert_eq!(unzlib_into(&deflated, 70_000).unwrap(), &text[..70_000]);
    }

    #[test]
    fn refuses_a_zlib_header_as_fflate_does() {
        let short = compress_to_vec_zlib(b"abc", 6);
        for header in [[0x78, 0x20], [0x88, 0x01], [0x79, 0x9c], [0x78, 0x9d]] {
            let mut data = short.clone();
            data[..2].copy_from_slice(&header);
            assert_eq!(
                unzlib_into(&data, 10),
                Err(InflateError::Header),
                "{header:x?}"
            );
        }
        assert_eq!(unzlib_into(&[0x78], 10), Err(InflateError::Header));
        assert_eq!(unzlib_into(&[], 10), Err(InflateError::Header));
    }
}
