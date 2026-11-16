//! LZCOMP, the compressor of MicroType Express's three blocks (W3C MTX submission §3.3 and appendix C): LZ77 copies
//! against a history that starts with 7168 preloaded bytes, coded with three adaptive Huffman trees, and an
//! optional run-length layer on top.

use super::{MtxError, MtxStream};
use crate::container::{ContainerError, ContainerLimits, Part, check_expansion};

/// The bytes the history starts with: every pair (k, j) for k < 32, j < 96, then every byte value four times.
pub(super) const PRELOAD: usize = 2 * 32 * 96 + 4 * 256;
/// Copies at least this far back are one byte longer than their coded length.
const MAX_2_BYTE_DISTANCE: u32 = 512;
const DIST_WIDTH: u32 = 3;
const LEN_STOP: u16 = 1 << 2;
const ROOT: usize = 1;

/// The preloaded history.
pub(super) fn preload() -> Vec<u8> {
    let mut history = Vec::with_capacity(PRELOAD);
    for k in 0..32u8 {
        for j in 0..96u8 {
            history.extend_from_slice(&[k, j]);
        }
    }
    for j in 0..=255u8 {
        history.extend_from_slice(&[j; 4]);
    }
    history
}

/// The number of 3-bit distance symbols needed to reach `length` bytes back.
pub(super) fn distance_ranges(length: usize) -> u16 {
    let mut ranges: u16 = 1;
    while (1u64 << (DIST_WIDTH * u32::from(ranges))) < length as u64 {
        ranges += 1;
    }
    ranges
}

/// The symbols of the main tree for a block of `length` bytes: 256 literals, 8 copy codes per distance range, then
/// the one-byte copies from 2, 4 and 6 bytes back.
#[derive(Clone, Copy, Debug)]
pub(super) struct Symbols {
    pub(super) ranges: u16,
    pub(super) dup2: u16,
    pub(super) count: u16,
}

impl Symbols {
    pub(super) fn for_length(length: usize) -> Self {
        let ranges = distance_ranges(length);
        let dup2 = 256 + 8 * ranges;
        Symbols {
            ranges,
            dup2,
            count: dup2 + 3,
        }
    }
}

/// An MSB-first bit reader.
pub(super) struct BitReader<'a> {
    data: &'a [u8],
    at: usize,
    byte: u8,
    left: u8,
    block: u8,
}

impl<'a> BitReader<'a> {
    pub(super) fn new(data: &'a [u8], block: u8) -> Self {
        BitReader {
            data,
            at: 0,
            byte: 0,
            left: 0,
            block,
        }
    }

    pub(super) fn bit(&mut self) -> Result<bool, ContainerError> {
        if self.left == 0 {
            self.byte = *self
                .data
                .get(self.at)
                .ok_or(ContainerError::Truncated(Part::Mtx(MtxStream::Lzcomp(
                    self.block,
                ))))?;
            self.at += 1;
            self.left = 8;
        }
        self.left -= 1;
        Ok(self.byte >> self.left & 1 != 0)
    }

    pub(super) fn value(&mut self, bits: u32) -> Result<u32, ContainerError> {
        let mut value = 0;
        for _ in 0..bits {
            value = value << 1 | u32::from(self.bit()?);
        }
        Ok(value)
    }
}

#[derive(Clone, Copy, Default)]
struct Node {
    up: usize,
    left: usize,
    right: usize,
    symbol: Option<u16>,
    weight: u32,
}

/// An adaptive Huffman tree over symbols `0..range` (appendix C `AHUFF`). Nodes are numbered from the root (1) in
/// non-increasing weight order; each coded symbol adds one to its weight and swaps nodes to keep that order.
pub(super) struct Ahuff {
    nodes: Vec<Node>,
    /// The node of each symbol.
    leaves: Vec<usize>,
    block: u8,
}

impl Ahuff {
    pub(super) fn new(range: u16, block: u8) -> Result<Self, ContainerError> {
        let range = usize::from(range);
        let mut nodes = vec![Node::default(); 2 * range];
        let mut leaves = vec![0; range];
        for (i, node) in nodes.iter_mut().enumerate().skip(2) {
            node.up = i / 2;
            node.weight = 1;
        }
        for (i, node) in nodes.iter_mut().enumerate().take(range).skip(1) {
            node.left = 2 * i;
            node.right = 2 * i + 1;
        }
        for (symbol, (node, leaf)) in nodes
            .iter_mut()
            .skip(range)
            .zip(leaves.iter_mut())
            .enumerate()
        {
            node.symbol = u16::try_from(symbol).ok();
            *leaf = range + symbol;
        }
        let mut tree = Ahuff {
            nodes,
            leaves,
            block,
        };
        for i in (ROOT..range).rev() {
            let (left, right) = (tree.node(i)?.left, tree.node(i)?.right);
            tree.node_mut(i)?.weight = tree.node(left)?.weight + tree.node(right)?.weight;
        }
        if range > 256 && range < 512 {
            // The main tree starts out favouring the shortest copies and the one-byte copies from 2 and 4 back.
            tree.bump(256)?;
            tree.bump(257)?;
            for _ in 0..12 {
                tree.bump(range - 3)?;
            }
            for _ in 0..6 {
                tree.bump(range - 2)?;
            }
        } else {
            for _ in 0..2 {
                for symbol in 0..range {
                    tree.bump(symbol)?;
                }
            }
        }
        Ok(tree)
    }

    fn invalid(&self) -> ContainerError {
        MtxError::Lzcomp { block: self.block }.into()
    }

    fn node(&self, i: usize) -> Result<&Node, ContainerError> {
        self.nodes.get(i).ok_or_else(|| self.invalid())
    }

    fn node_mut(&mut self, i: usize) -> Result<&mut Node, ContainerError> {
        let invalid = self.invalid();
        self.nodes.get_mut(i).ok_or(invalid)
    }

    fn bump(&mut self, symbol: usize) -> Result<(), ContainerError> {
        let leaf = *self.leaves.get(symbol).ok_or_else(|| self.invalid())?;
        self.update(leaf)
    }

    /// Swaps the subtrees at `a` and `b`; each position keeps its parent.
    fn swap(&mut self, a: usize, b: usize) -> Result<(), ContainerError> {
        let (up_a, up_b) = (self.node(a)?.up, self.node(b)?.up);
        if a >= self.nodes.len() || b >= self.nodes.len() {
            return Err(self.invalid());
        }
        self.nodes.swap(a, b);
        self.node_mut(a)?.up = up_a;
        self.node_mut(b)?.up = up_b;
        for at in [a, b] {
            let node = *self.node(at)?;
            match node.symbol {
                Some(symbol) => {
                    let invalid = self.invalid();
                    *self.leaves.get_mut(usize::from(symbol)).ok_or(invalid)? = at;
                }
                None => {
                    self.node_mut(node.left)?.up = at;
                    self.node_mut(node.right)?.up = at;
                }
            }
        }
        Ok(())
    }

    /// Adds one to the weight of node `a` and its ancestors. Before each increment the node moves to the first
    /// position of its weight, so the order stays non-increasing.
    fn update(&mut self, mut a: usize) -> Result<(), ContainerError> {
        while a != ROOT {
            let weight = self.node(a)?.weight;
            let mut b = a.checked_sub(1).ok_or_else(|| self.invalid())?;
            if self.node(b)?.weight == weight {
                while self.node(b)?.weight == weight {
                    b = b.checked_sub(1).ok_or_else(|| self.invalid())?;
                }
                b += 1;
                if b > ROOT {
                    self.swap(a, b)?;
                    a = b;
                }
            }
            let node = self.node_mut(a)?;
            node.weight = weight + 1;
            a = node.up;
        }
        self.node_mut(ROOT)?.weight += 1;
        Ok(())
    }

    pub(super) fn read(&mut self, bits: &mut BitReader<'_>) -> Result<u16, ContainerError> {
        let mut a = ROOT;
        loop {
            let node = self.node(a)?;
            a = if bits.bit()? { node.right } else { node.left };
            if let Some(symbol) = self.node(a)?.symbol {
                self.update(a)?;
                return Ok(symbol);
            }
        }
    }

    /// Writes `symbol`'s code, root first, and updates the tree as [`Ahuff::read`] does.
    #[cfg(test)]
    pub(super) fn write(&mut self, symbol: u16, bits: &mut Vec<bool>) {
        let leaf = self.leaves[usize::from(symbol)];
        let mut path = Vec::new();
        let mut a = leaf;
        while a != ROOT {
            let up = self.nodes[a].up;
            path.push(self.nodes[up].right == a);
            a = up;
        }
        bits.extend(path.iter().rev());
        self.update(leaf).unwrap();
    }
}

/// The run-length layer: the first byte is the escape; `escape, 0` is a literal escape byte and `escape, n, b` is
/// `n` copies of `b`.
#[derive(Clone, Copy)]
enum Rle {
    Start,
    Normal(u8),
    Escape(u8),
    Count(u8, u8),
}

struct Output<'l> {
    bytes: Vec<u8>,
    rle: Option<Rle>,
    spent: &'l mut u64,
    input: usize,
    limits: &'l ContainerLimits,
}

impl Output<'_> {
    fn push(&mut self, byte: u8) -> Result<(), ContainerError> {
        let Some(state) = self.rle else {
            return Ok(());
        };
        let (next, run) = match state {
            Rle::Start => (Rle::Normal(byte), None),
            Rle::Normal(escape) if byte == escape => (Rle::Escape(escape), None),
            Rle::Normal(escape) => (Rle::Normal(escape), Some((byte, 1))),
            Rle::Escape(escape) if byte == 0 => (Rle::Normal(escape), Some((escape, 1))),
            Rle::Escape(escape) => (Rle::Count(escape, byte), None),
            Rle::Count(escape, count) => (Rle::Normal(escape), Some((byte, count))),
        };
        self.rle = Some(next);
        if let Some((byte, count)) = run {
            *self.spent += u64::from(count);
            check_expansion(*self.spent, self.input, self.limits)?;
            self.bytes
                .resize(self.bytes.len() + usize::from(count), byte);
        }
        Ok(())
    }
}

/// Decompresses LZCOMP block `block` (1 to 3) of an MTX font. `spent` counts the bytes every block has produced so
/// far, checked against the limits for an `input`-byte font.
pub(super) fn decompress(
    data: &[u8],
    block: u8,
    spent: &mut u64,
    input: usize,
    limits: &ContainerLimits,
) -> Result<Vec<u8>, ContainerError> {
    let invalid = || ContainerError::from(MtxError::Lzcomp { block });
    let mut bits = BitReader::new(data, block);
    let run_length = bits.bit()?;
    let mut distances = Ahuff::new(1 << DIST_WIDTH, block)?;
    let mut lengths = Ahuff::new(8, block)?;
    let length = bits.value(24)? as usize;
    *spent += length as u64;
    check_expansion(*spent, input, limits)?;
    let symbols = Symbols::for_length(length);
    let mut main = Ahuff::new(symbols.count, block)?;

    let mut history = preload();
    history.reserve(length);
    let end = PRELOAD + length;
    let mut output = Output {
        bytes: Vec::new(),
        rle: run_length.then_some(Rle::Start),
        spent,
        input,
        limits,
    };
    while history.len() < end {
        let symbol = main.read(&mut bits)?;
        let back = match symbol.checked_sub(symbols.dup2) {
            Some(0) => Some(2),
            Some(1) => Some(4),
            Some(2) => Some(6),
            _ => None,
        };
        if symbol < 256 || back.is_some() {
            let byte = match back {
                Some(back) => *history.get(history.len() - back).ok_or_else(invalid)?,
                None => u8::try_from(symbol).map_err(|_| invalid())?,
            };
            history.push(byte);
            output.push(byte)?;
            continue;
        }
        let code = symbol - 256;
        let ranges = code / 8 + 1;
        if ranges > symbols.ranges {
            return Err(invalid());
        }
        let mut chunk = code % 8;
        let mut copy: usize = 0;
        loop {
            copy = copy << 2 | usize::from(chunk & 3);
            if chunk & LEN_STOP == 0 {
                break;
            }
            if copy > length {
                return Err(invalid());
            }
            chunk = lengths.read(&mut bits)?;
        }
        copy += 2;
        let mut distance: u32 = 0;
        for _ in 0..ranges {
            distance = distance << DIST_WIDTH | u32::from(distances.read(&mut bits)?);
        }
        distance += 1;
        if distance >= MAX_2_BYTE_DISTANCE {
            copy += 1;
        }
        // The distance counts back to the copy's last byte.
        let start = history
            .len()
            .checked_sub(distance as usize + copy - 1)
            .ok_or_else(invalid)?;
        if copy > end - history.len() {
            return Err(invalid());
        }
        history.extend_from_within(start..start + copy);
        if output.rle.is_some() {
            for at in history.len() - copy..history.len() {
                let byte = *history.get(at).ok_or_else(invalid)?;
                output.push(byte)?;
            }
        }
    }
    if output.rle.is_some() {
        Ok(output.bytes)
    } else {
        Ok(history.split_off(PRELOAD))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_preload_is_the_specified_7168_bytes() {
        let history = preload();
        assert_eq!(history.len(), 7168);
        assert_eq!(&history[..4], [0, 0, 0, 1]);
        assert_eq!(&history[190..194], [0, 95, 1, 0]);
        assert_eq!(&history[6144..6152], [0, 0, 0, 0, 1, 1, 1, 1]);
        assert_eq!(&history[7164..], [255; 4]);
    }

    #[test]
    fn distance_ranges_cover_the_block_length() {
        assert_eq!(distance_ranges(0), 1);
        assert_eq!(distance_ranges(8), 1);
        assert_eq!(distance_ranges(9), 2);
        assert_eq!(distance_ranges(64), 2);
        assert_eq!(distance_ranges(65), 3);
        assert_eq!(distance_ranges(0xff_ffff), 8);
        let symbols = Symbols::for_length(100_000);
        assert_eq!(symbols.ranges, 6);
        assert_eq!(symbols.dup2, 256 + 48);
        assert_eq!(symbols.count, 256 + 48 + 3);
    }

    #[test]
    fn bits_are_read_msb_first_and_never_past_the_end() {
        let mut bits = BitReader::new(&[0b1010_0000, 0xff], 1);
        assert!(bits.bit().unwrap());
        assert!(!bits.bit().unwrap());
        assert_eq!(bits.value(6).unwrap(), 0b10_0000);
        assert_eq!(bits.value(8).unwrap(), 0xff);
        assert_eq!(
            bits.bit(),
            Err(ContainerError::Truncated(Part::Mtx(MtxStream::Lzcomp(1))))
        );
    }

    /// Walks the tree checking the invariants appendix C's `check_tree` asserts.
    fn check(tree: &Ahuff) {
        let last = tree.nodes.len() - 1;
        for i in ROOT..last {
            assert!(
                tree.nodes[i].weight >= tree.nodes[i + 1].weight,
                "order at {i}"
            );
        }
        for i in ROOT..tree.nodes.len() {
            let node = tree.nodes[i];
            match node.symbol {
                None => {
                    assert_eq!(tree.nodes[node.left].up, i);
                    assert_eq!(tree.nodes[node.right].up, i);
                    assert_eq!(
                        node.weight,
                        tree.nodes[node.left].weight + tree.nodes[node.right].weight
                    );
                    if i > ROOT {
                        assert_eq!(node.left.abs_diff(node.right), 1, "siblings at {i}");
                    }
                }
                Some(symbol) => assert_eq!(tree.leaves[usize::from(symbol)], i),
            }
        }
    }

    #[test]
    fn the_tree_keeps_its_invariants_and_decodes_what_it_encodes() {
        for range in [8u16, 267, 323] {
            let mut encoder = Ahuff::new(range, 1).unwrap();
            check(&encoder);
            let symbols: Vec<u16> = (0..2000u32)
                .map(|i| u16::try_from((i * i + 7 * i) % u32::from(range)).unwrap())
                .collect();
            let mut bits = Vec::new();
            for &symbol in &symbols {
                encoder.write(symbol, &mut bits);
            }
            check(&encoder);
            let bytes: Vec<u8> = bits
                .chunks(8)
                .map(|chunk| {
                    chunk
                        .iter()
                        .enumerate()
                        .fold(0u8, |byte, (i, &bit)| byte | u8::from(bit) << (7 - i))
                })
                .collect();
            let mut decoder = Ahuff::new(range, 1).unwrap();
            let mut reader = BitReader::new(&bytes, 1);
            for &symbol in &symbols {
                assert_eq!(decoder.read(&mut reader).unwrap(), symbol);
            }
        }
    }

    #[test]
    fn a_fresh_main_tree_gives_the_favoured_symbols_short_codes() {
        let tree = Ahuff::new(267, 1).unwrap();
        let depth = |symbol: usize| {
            let mut a = tree.leaves[symbol];
            let mut depth = 0;
            while a != ROOT {
                a = tree.nodes[a].up;
                depth += 1;
            }
            depth
        };
        // DUP2 was bumped twelve times, DUP4 six, copy code 256 once, DUP6 never.
        assert!(depth(264) < depth(265));
        assert!(depth(265) < depth(256));
        assert!(depth(265) < depth(266));
        assert!(depth(264) < depth(100));
    }
}
