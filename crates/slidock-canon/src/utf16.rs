//! JavaScript string semantics over UTF-16 code units: `length`, `<` comparison, and `slice` indices.
//!
//! Rust strings cannot hold a lone surrogate, so an index that falls between the two halves of a surrogate pair has
//! no Rust equivalent: [`byte_offset`] and [`slice()`] refuse it, and [`clip`] stops before the pair.

use std::cmp::Ordering;

/// `s.length`: the number of UTF-16 code units.
pub fn len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// `a < b` as JavaScript compares strings: code unit by code unit. Byte (code point) order differs from it only
/// between U+E000–U+FFFF and astral characters, which UTF-16 orders below them.
pub fn cmp(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// The byte offset of UTF-16 index `units`, or `None` when it splits a surrogate pair or lies past the end.
pub fn byte_offset(s: &str, units: usize) -> Option<usize> {
    let mut seen = 0;
    for (offset, c) in s.char_indices() {
        if seen == units {
            return Some(offset);
        }
        seen += c.len_utf16();
        if seen > units {
            return None;
        }
    }
    (seen == units).then_some(s.len())
}

/// The longest prefix of at most `max_units` code units that ends on a character boundary. `s.slice(0, n)` would
/// split a surrogate pair at the limit; this drops the whole pair instead.
pub fn clip(s: &str, max_units: usize) -> &str {
    let mut seen = 0;
    for (offset, c) in s.char_indices() {
        seen += c.len_utf16();
        if seen > max_units {
            return s.get(..offset).unwrap_or(s);
        }
    }
    s
}

/// `s.slice(start, end)` for non-negative indices: both are clamped to the length, and `start ≥ end` gives `""`.
/// `None` when an index splits a surrogate pair.
pub fn slice(s: &str, start: usize, end: usize) -> Option<&str> {
    let total = len(s);
    let (start, end) = (start.min(total), end.min(total));
    if start >= end {
        return Some("");
    }
    s.get(byte_offset(s, start)?..byte_offset(s, end)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_code_units() {
        assert_eq!(len(""), 0);
        assert_eq!(len("abc"), 3);
        assert_eq!(len("é"), 1);
        assert_eq!(len("😀"), 2);
        assert_eq!(len("a😀b"), 4);
    }

    #[test]
    fn compares_code_units_not_code_points() {
        // U+FF5E is above U+1F600 in code point (and UTF-8 byte) order, below it in UTF-16.
        assert_eq!("\u{FF5E}".cmp("😀"), Ordering::Less);
        assert_eq!(cmp("\u{FF5E}", "😀"), Ordering::Greater);
        assert_eq!(cmp("ab", "abc"), Ordering::Less);
        assert_eq!(cmp("b", "abc"), Ordering::Greater);
        assert_eq!(cmp("x", "x"), Ordering::Equal);
    }

    #[test]
    fn offsets_refuse_split_pairs() {
        let s = "a😀b";
        assert_eq!(byte_offset(s, 0), Some(0));
        assert_eq!(byte_offset(s, 1), Some(1));
        assert_eq!(byte_offset(s, 2), None);
        assert_eq!(byte_offset(s, 3), Some(5));
        assert_eq!(byte_offset(s, 4), Some(6));
        assert_eq!(byte_offset(s, 5), None);
        assert_eq!(byte_offset("", 0), Some(0));
    }

    #[test]
    fn clip_keeps_whole_characters() {
        assert_eq!(clip("a😀b", 0), "");
        assert_eq!(clip("a😀b", 1), "a");
        assert_eq!(clip("a😀b", 2), "a");
        assert_eq!(clip("a😀b", 3), "a😀");
        assert_eq!(clip("a😀b", 9), "a😀b");
    }

    #[test]
    fn slice_clamps_like_javascript() {
        assert_eq!(slice("héllo", 1, 3), Some("él"));
        assert_eq!(slice("héllo", 3, 1), Some(""));
        assert_eq!(slice("héllo", 2, 99), Some("llo"));
        assert_eq!(slice("a😀b", 1, 3), Some("😀"));
        assert_eq!(slice("a😀b", 2, 4), None);
    }
}
