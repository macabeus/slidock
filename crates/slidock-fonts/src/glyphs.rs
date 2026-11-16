//! Sets of code points: what a face draws (collect-glyphs), what a subset keeps, and what a face covers, written
//! as a CSS `unicode-range`.

use std::fmt::Write as _;

/// The code points every shipped face keeps whatever the text draws: space, no-break space, hyphen-minus, hyphen
/// and ellipsis (text-overflow, line breaking and the browser's own hyphenation reach for them), and À, whose glyph
/// top CoreText's leading rule reads (so the shipped file keeps the `graveTop` the binding measured).
pub const ALWAYS_DRAWN: [u32; 6] = [0x20, 0x2d, 0xa0, 0xc0, 0x2010, 0x2026];

/// Code points a browser shows without a glyph from any font: the C0 and C1 controls and Unicode's
/// `Default_Ignorable_Code_Point`s (soft hyphen, joiners, bidi controls, variation selectors, …).
const DRAWN_WITHOUT_GLYPH: [(u32, u32); 18] = [
    (0x0, 0x1f),
    (0x7f, 0x9f),
    (0xad, 0xad),
    (0x34f, 0x34f),
    (0x61c, 0x61c),
    (0x115f, 0x1160),
    (0x17b4, 0x17b5),
    (0x180b, 0x180f),
    (0x200b, 0x200f),
    (0x202a, 0x202e),
    (0x2060, 0x206f),
    (0x3164, 0x3164),
    (0xfe00, 0xfe0f),
    (0xfeff, 0xfeff),
    (0xffa0, 0xffa0),
    (0xfff0, 0xfff8),
    (0x1bca0, 0x1bca3),
    (0x1d173, 0x1d17a),
];

/// The tags and variation selectors supplement, ignorable as the ranges above.
const IGNORABLE_TAGS: (u32, u32) = (0xe0000, 0xe0fff);

/// Whether a browser shows `code_point` without a glyph from any font, so a face that lacks it leaves no gap.
pub fn draws_no_glyph(code_point: u32) -> bool {
    DRAWN_WITHOUT_GLYPH
        .iter()
        .chain([&IGNORABLE_TAGS])
        .any(|&(first, last)| (first..=last).contains(&code_point))
}

/// Code points every bundled substitute maps (Liberation, Carlito, Caladea and DejaVu Sans, each face): a deck drawing
/// only these with a substitute whose file has not arrived yet needs no last-resort face.
pub const BUNDLED_COVERAGE: [(u32, u32); 11] = [
    (0x20, 0x7e),
    (0xa0, 0xac),
    (0xae, 0xff),
    (0x2013, 0x2014),
    (0x2018, 0x201e),
    (0x2020, 0x2022),
    (0x2026, 0x2026),
    (0x2030, 0x2030),
    (0x2039, 0x203a),
    (0x20ac, 0x20ac),
    (0x2122, 0x2122),
];

/// Whether every bundled substitute face maps `code_point` ([`BUNDLED_COVERAGE`]).
pub fn bundled_covers(code_point: u32) -> bool {
    BUNDLED_COVERAGE
        .iter()
        .any(|&(first, last)| (first..=last).contains(&code_point))
}

/// The largest Unicode scalar value.
const MAX_CODE_POINT: u32 = 0x10_ffff;

/// A set of Unicode code points, iterated in ascending order. It is held as runs, so a face that maps whole planes
/// costs a few words, not a million entries.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct CodepointSet {
    /// Disjoint, non-adjacent `(first, last)` runs, ascending.
    runs: Vec<(u32, u32)>,
}

impl CodepointSet {
    pub fn new() -> CodepointSet {
        CodepointSet::default()
    }

    /// Adds a code point; values above U+10FFFF are ignored.
    pub fn insert(&mut self, code_point: u32) {
        self.insert_range(code_point, code_point);
    }

    /// Adds every code point of `text`.
    pub fn insert_text(&mut self, text: &str) {
        for c in text.chars() {
            self.insert(u32::from(c));
        }
    }

    /// Adds every code point of `text` and of its full uppercase mapping (`ß` adds `S`), which `text-transform:
    /// uppercase` and synthesized small caps draw.
    pub fn insert_text_upper(&mut self, text: &str) {
        for c in text.chars() {
            self.insert(u32::from(c));
            for upper in c.to_uppercase() {
                self.insert(u32::from(upper));
            }
        }
    }

    /// Adds the code points from `first` to `last`, both included; the part above U+10FFFF is ignored.
    pub fn insert_range(&mut self, first: u32, last: u32) {
        let last = last.min(MAX_CODE_POINT);
        if first > last {
            return;
        }
        // The runs that touch or overlap [first, last] merge with it.
        let start = self
            .runs
            .partition_point(|&(_, run_last)| run_last.saturating_add(1) < first);
        let end = self
            .runs
            .partition_point(|&(run_first, _)| run_first <= last.saturating_add(1));
        let merged = match self.runs.get(start..end) {
            Some([first_run, .., last_run]) => (first.min(first_run.0), last.max(last_run.1)),
            Some([only]) => (first.min(only.0), last.max(only.1)),
            _ => (first, last),
        };
        self.runs.splice(start..end, [merged]);
    }

    pub fn extend(&mut self, other: &CodepointSet) {
        for &(first, last) in &other.runs {
            self.insert_range(first, last);
        }
    }

    pub fn contains(&self, code_point: u32) -> bool {
        let at = self.runs.partition_point(|&(_, last)| last < code_point);
        self.runs
            .get(at)
            .is_some_and(|&(first, _)| first <= code_point)
    }

    /// How many code points the set holds.
    pub fn len(&self) -> u64 {
        self.runs
            .iter()
            .map(|&(first, last)| u64::from(last - first) + 1)
            .sum()
    }

    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }

    pub fn first(&self) -> Option<u32> {
        self.runs.first().map(|&(first, _)| first)
    }

    /// Every code point, ascending.
    pub fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        self.runs.iter().flat_map(|&(first, last)| first..=last)
    }

    /// The code points in both sets.
    pub fn intersection(&self, other: &CodepointSet) -> CodepointSet {
        let mut out = CodepointSet::new();
        let (mut a, mut b) = (self.runs.iter().peekable(), other.runs.iter().peekable());
        while let (Some(&&(a_first, a_last)), Some(&&(b_first, b_last))) = (a.peek(), b.peek()) {
            let (first, last) = (a_first.max(b_first), a_last.min(b_last));
            if first <= last {
                out.runs.push((first, last));
            }
            if a_last < b_last {
                a.next();
            } else {
                b.next();
            }
        }
        out
    }

    /// The code points not in `other`.
    pub fn difference(&self, other: &CodepointSet) -> CodepointSet {
        let mut out = CodepointSet::new();
        for &(first, last) in &self.runs {
            let mut from = first;
            for &(other_first, other_last) in &other.runs {
                if other_last < from || other_first > last {
                    continue;
                }
                if other_first > from {
                    out.runs.push((from, other_first - 1));
                }
                match other_last.checked_add(1) {
                    Some(next) => from = next,
                    None => {
                        from = u32::MAX;
                        break;
                    }
                }
                if from > last {
                    break;
                }
            }
            if from <= last {
                out.runs.push((from, last));
            }
        }
        out
    }

    /// Runs of consecutive code points, each `(first, last)`, ascending.
    pub fn ranges(&self) -> &[(u32, u32)] {
        &self.runs
    }

    /// The set as a CSS `unicode-range` value (`U+20-7E,U+A0`), uppercase hexadecimal; `None` when empty.
    pub fn unicode_range(&self) -> Option<String> {
        let mut out = String::new();
        for &(first, last) in &self.runs {
            if !out.is_empty() {
                out.push(',');
            }
            let _ = if first == last {
                write!(out, "U+{first:X}")
            } else {
                write!(out, "U+{first:X}-{last:X}")
            };
        }
        (!out.is_empty()).then_some(out)
    }
}

impl FromIterator<u32> for CodepointSet {
    fn from_iter<I: IntoIterator<Item = u32>>(iter: I) -> Self {
        let mut set = CodepointSet::new();
        for point in iter {
            set.insert(point);
        }
        set
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_merge_runs_and_print_as_unicode_range() {
        let set: CodepointSet = [0x41, 0x42, 0x43, 0x20, 0xa0, 0x1f600]
            .into_iter()
            .collect();
        assert_eq!(
            set.ranges(),
            [(0x20, 0x20), (0x41, 0x43), (0xa0, 0xa0), (0x1f600, 0x1f600)]
        );
        assert_eq!(
            set.unicode_range().as_deref(),
            Some("U+20,U+41-43,U+A0,U+1F600")
        );
        assert_eq!(CodepointSet::new().unicode_range(), None);
    }

    #[test]
    fn uppercase_mapping_expands() {
        let mut set = CodepointSet::new();
        set.insert_text_upper("ßa");
        let points: Vec<u32> = set.iter().collect();
        assert_eq!(
            points,
            [u32::from('A'), u32::from('S'), u32::from('a'), 0xdf]
        );
    }

    #[test]
    fn runs_merge_on_overlap_and_adjacency() {
        let mut set = CodepointSet::new();
        set.insert_range(10, 20);
        set.insert_range(30, 40);
        set.insert(21);
        assert_eq!(set.ranges(), [(10, 21), (30, 40)]);
        set.insert_range(22, 29);
        assert_eq!(set.ranges(), [(10, 40)]);
        set.insert_range(0, 5);
        set.insert_range(50, 60);
        set.insert_range(3, 55);
        assert_eq!(set.ranges(), [(0, 60)]);
        assert_eq!(set.len(), 61);
        assert!(set.contains(0) && set.contains(60) && !set.contains(61));
    }

    #[test]
    fn intersection_and_difference_work_on_runs() {
        let mut a = CodepointSet::new();
        a.insert_range(0, 10);
        a.insert_range(20, 30);
        let mut b = CodepointSet::new();
        b.insert_range(5, 25);
        b.insert(30);
        assert_eq!(a.intersection(&b).ranges(), [(5, 10), (20, 25), (30, 30)]);
        assert_eq!(a.difference(&b).ranges(), [(0, 4), (26, 29)]);
        assert_eq!(b.difference(&a).ranges(), [(11, 19)]);
        let all: CodepointSet = {
            let mut s = CodepointSet::new();
            s.insert_range(0, u32::MAX);
            s
        };
        assert!(a.difference(&all).is_empty());
        assert_eq!(all.len(), 0x11_0000);
        assert_eq!(all.difference(&a).ranges(), [(11, 19), (31, 0x10_ffff)]);
    }

    #[test]
    fn values_past_unicode_are_ignored() {
        let mut set = CodepointSet::new();
        set.insert(0x11_0000);
        set.insert_range(0x10_fffe, u32::MAX);
        assert_eq!(set.ranges(), [(0x10_fffe, 0x10_ffff)]);
    }

    #[test]
    fn controls_and_default_ignorables_draw_no_glyph() {
        for point in [
            0x9, 0xa, 0x7f, 0xad, 0x200b, 0x200d, 0x2060, 0xfe0f, 0xfeff, 0xe0001,
        ] {
            assert!(draws_no_glyph(point), "U+{point:X}");
        }
        for point in [0x20, 0x41, 0xa0, 0x2010, 0x25cf, 0x2756, 0x263a, 0x1f600] {
            assert!(!draws_no_glyph(point), "U+{point:X}");
        }
    }

    #[test]
    fn bundled_coverage_holds_latin_1_and_common_punctuation_only() {
        assert!(bundled_covers(u32::from('A')) && bundled_covers(0xe9) && bundled_covers(0x2026));
        assert!(!bundled_covers(0xad) && !bundled_covers(0x2010) && !bundled_covers(0x25cf));
        assert!(!bundled_covers(0x2756) && !bundled_covers(0x1f600));
    }
}
