//! A test-only LZCOMP encoder and MTX container writer, to build streams the decoder must read: a greedy matcher
//! over the preloaded history that uses every symbol kind, with the optional run-length layer.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

use std::collections::BTreeMap;

use super::lzcomp::{Ahuff, PRELOAD, Symbols, preload};

/// What the encoder may emit, to steer streams through particular decoder paths.
#[derive(Clone, Copy, Debug)]
pub(super) struct Options {
    pub(super) run_length: bool,
    /// Use copies (otherwise only literals and one-byte copies).
    pub(super) copies: bool,
    /// Use the one-byte copies from 2, 4 and 6 back.
    pub(super) dups: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            run_length: false,
            copies: true,
            dups: true,
        }
    }
}

/// Counts of the symbols an encoding used.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Stats {
    pub(super) literals: usize,
    pub(super) dups: [usize; 3],
    pub(super) copies: usize,
    pub(super) far_copies: usize,
    pub(super) preload_copies: usize,
    pub(super) long_lengths: usize,
}

pub(super) struct Bits(pub(super) Vec<bool>);

impl Bits {
    pub(super) fn value(&mut self, value: u32, bits: u32) {
        for i in (0..bits).rev() {
            self.0.push(value >> i & 1 != 0);
        }
    }

    pub(super) fn bytes(&self) -> Vec<u8> {
        self.0
            .chunks(8)
            .map(|chunk| {
                chunk
                    .iter()
                    .enumerate()
                    .fold(0u8, |byte, (i, &bit)| byte | u8::from(bit) << (7 - i))
            })
            .collect()
    }
}

/// Appendix C's `MTX_RUNLENGTHCOMP_PackData`: the least frequent byte escapes runs longer than three.
pub(super) fn run_length_pack(data: &[u8]) -> Vec<u8> {
    let mut counts = [0usize; 256];
    data.iter().for_each(|&b| counts[usize::from(b)] += 1);
    let escape = (0..=255u8).min_by_key(|&b| counts[usize::from(b)]).unwrap();
    let mut out = vec![escape];
    let mut i = 0;
    while i < data.len() {
        let byte = data[i];
        let run = data[i..]
            .iter()
            .take(255)
            .take_while(|&&b| b == byte)
            .count();
        if run > 3 {
            out.extend_from_slice(&[escape, u8::try_from(run).unwrap(), byte]);
            i += run;
        } else {
            if byte == escape {
                out.extend_from_slice(&[escape, 0]);
            } else {
                out.push(byte);
            }
            i += 1;
        }
    }
    out
}

fn bits_used(value: u32) -> u32 {
    32 - value.max(1).leading_zeros()
}

pub(super) struct Encoder {
    pub(super) bits: Bits,
    distances: Ahuff,
    lengths: Ahuff,
    main: Ahuff,
    symbols: Symbols,
}

impl Encoder {
    /// Starts a block of `length` bytes.
    pub(super) fn new(run_length: bool, length: usize) -> Self {
        let mut bits = Bits(vec![run_length]);
        let distances = Ahuff::new(8, 1).unwrap();
        let lengths = Ahuff::new(8, 1).unwrap();
        bits.value(u32::try_from(length).unwrap(), 24);
        let symbols = Symbols::for_length(length);
        let main = Ahuff::new(symbols.count, 1).unwrap();
        Encoder {
            bits,
            distances,
            lengths,
            main,
            symbols,
        }
    }

    pub(super) fn literal(&mut self, byte: u8) {
        self.main.write(u16::from(byte), &mut self.bits.0);
    }

    /// The one-byte copy from 2, 4 or 6 back.
    pub(super) fn dup(&mut self, back: u16) {
        self.main
            .write(self.symbols.dup2 + back / 2 - 1, &mut self.bits.0);
    }

    /// A copy of `length` bytes whose last source byte is `distance` back.
    pub(super) fn copy(&mut self, length: usize, distance: u32) {
        let minimum = if distance >= 512 { 3 } else { 2 };
        let value = u32::try_from(length - minimum).unwrap();
        let chunks = bits_used(value).div_ceil(2);
        let ranges = bits_used(distance - 1).div_ceil(3);
        assert!(ranges <= u32::from(self.symbols.ranges));
        for k in 0..chunks {
            let more = if k + 1 < chunks { 4 } else { 0 };
            let symbol = u16::try_from(more | (value >> (2 * (chunks - 1 - k)) & 3)).unwrap();
            if k == 0 {
                let first = 256 + symbol + u16::try_from((ranges - 1) * 8).unwrap();
                self.main.write(first, &mut self.bits.0);
            } else {
                self.lengths.write(symbol, &mut self.bits.0);
            }
        }
        for i in (0..ranges).rev() {
            let symbol = u16::try_from((distance - 1) >> (3 * i) & 7).unwrap();
            self.distances.write(symbol, &mut self.bits.0);
        }
    }

    /// A copy's length symbols as given: the first through the main tree, the others through the length tree.
    pub(super) fn copy_length_chunks(&mut self, chunks: &[u16]) {
        for (k, &chunk) in chunks.iter().enumerate() {
            if k == 0 {
                self.main.write(256 + chunk, &mut self.bits.0);
            } else {
                self.lengths.write(chunk, &mut self.bits.0);
            }
        }
    }

    pub(super) fn finish(self) -> Vec<u8> {
        self.bits.bytes()
    }
}

/// LZCOMP-compresses `data`.
pub(super) fn lzcomp(data: &[u8], options: Options) -> (Vec<u8>, Stats) {
    let packed;
    let data = if options.run_length {
        packed = run_length_pack(data);
        &packed[..]
    } else {
        data
    };
    let mut encoder = Encoder::new(options.run_length, data.len());
    let max_distance = 1usize << (3 * u32::from(encoder.symbols.ranges));
    let mut history = preload();
    history.extend_from_slice(data);
    let mut index: BTreeMap<[u8; 2], Vec<usize>> = BTreeMap::new();
    let add = |index: &mut BTreeMap<[u8; 2], Vec<usize>>, history: &[u8], at: usize| {
        if at + 1 < history.len() {
            index
                .entry([history[at], history[at + 1]])
                .or_default()
                .push(at);
        }
    };
    for at in 0..PRELOAD - 1 {
        add(&mut index, &history, at);
    }
    let mut stats = Stats::default();
    let mut pos = PRELOAD;
    while pos < history.len() {
        let mut best: Option<(usize, usize)> = None;
        if options.copies && pos + 1 < history.len() {
            let key = [history[pos], history[pos + 1]];
            for &start in index.get(&key).into_iter().flatten().rev().take(64) {
                let mut length = 0;
                while pos + length < history.len()
                    && start + length < pos
                    && history[start + length] == history[pos + length]
                {
                    length += 1;
                }
                let distance = pos - (start + length - 1);
                let minimum = if distance >= 512 { 3 } else { 2 };
                if length >= minimum
                    && distance <= max_distance
                    && best.is_none_or(|(best_length, _)| length > best_length)
                {
                    best = Some((length, start));
                }
            }
        }
        let byte = history[pos];
        let dup = [2, 4, 6]
            .into_iter()
            .find(|&back| options.dups && history[pos - back] == byte);
        match best {
            Some((length, start)) if length >= 3 || dup.is_none() => {
                let distance = pos - (start + length - 1);
                encoder.copy(length, u32::try_from(distance).unwrap());
                stats.copies += 1;
                stats.far_copies += usize::from(distance >= 512);
                stats.preload_copies += usize::from(start < PRELOAD);
                stats.long_lengths += usize::from(length >= 6);
                for at in pos..pos + length {
                    add(&mut index, &history, at - 1);
                }
                pos += length;
                continue;
            }
            _ => {}
        }
        match dup {
            Some(back) => {
                encoder.dup(u16::try_from(back).unwrap());
                stats.dups[back / 2 - 1] += 1;
            }
            None => {
                encoder.literal(byte);
                stats.literals += 1;
            }
        }
        add(&mut index, &history, pos - 1);
        pos += 1;
    }
    (encoder.finish(), stats)
}

/// An MTX container around three compressed blocks.
pub(super) fn container(blocks: [&[u8]; 3]) -> Vec<u8> {
    let second = 10 + blocks[0].len();
    let third = second + blocks[1].len();
    let mut out = vec![super::VERSION];
    for value in [0x7f_ffff, second, third] {
        out.extend_from_slice(&u32::try_from(value).unwrap().to_be_bytes()[1..]);
    }
    blocks.iter().for_each(|block| out.extend_from_slice(block));
    out
}

/// An MTX font of three uncompressed blocks.
pub(super) fn mtx(blocks: [&[u8]; 3], options: Options) -> Vec<u8> {
    let compressed = blocks.map(|block| lzcomp(block, options).0);
    container([&compressed[0], &compressed[1], &compressed[2]])
}
