//! CSS `unicode-range` values (`U+0460-052F, U+20B4, U+4??`): parsed into sorted, merged code point ranges.

/// The highest code point.
const MAX: u32 = 0x10_FFFF;
/// A list longer than this is refused.
const MAX_RANGES: usize = 4096;

/// Code point ranges, inclusive, sorted and merged.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnicodeRange {
    ranges: Vec<(u32, u32)>,
}

impl UnicodeRange {
    /// Ranges from inclusive pairs, in any order; overlapping and touching ranges merge.
    pub fn from_ranges(ranges: impl IntoIterator<Item = (u32, u32)>) -> UnicodeRange {
        let mut sorted: Vec<(u32, u32)> = ranges.into_iter().filter(|(lo, hi)| lo <= hi).collect();
        sorted.sort_unstable();
        let mut merged: Vec<(u32, u32)> = Vec::with_capacity(sorted.len());
        for (lo, hi) in sorted {
            match merged.last_mut() {
                Some(last) if lo <= last.1.saturating_add(1) => last.1 = last.1.max(hi),
                _ => merged.push((lo, hi)),
            }
        }
        UnicodeRange { ranges: merged }
    }

    /// A comma-separated list of `U+X`, `U+X-Y` and `U+X??` (wildcard) items; `None` when one does not parse.
    pub fn parse(value: &str) -> Option<UnicodeRange> {
        let mut ranges = Vec::new();
        for item in value.split(',') {
            let item = item.trim();
            if item.is_empty() {
                continue;
            }
            if ranges.len() == MAX_RANGES {
                return None;
            }
            let hex = item
                .strip_prefix("U+")
                .or_else(|| item.strip_prefix("u+"))?;
            let range = match hex.split_once('-') {
                Some((lo, hi)) => (code_point(lo)?, code_point(hi)?),
                None if hex.contains('?') => {
                    let digits = hex.trim_end_matches('?');
                    let wild = hex.len() - digits.len();
                    if wild == 0 || hex.len() > 6 || digits.contains('?') {
                        return None;
                    }
                    let base = if digits.is_empty() {
                        0
                    } else {
                        u32::from_str_radix(digits, 16).ok()?
                    };
                    let shift = 4 * u32::try_from(wild).ok()?;
                    let lo = base.checked_shl(shift)?;
                    let hi = lo | ((1u32 << shift) - 1);
                    (lo, hi.min(MAX))
                }
                None => {
                    let point = code_point(hex)?;
                    (point, point)
                }
            };
            if range.0 > range.1 || range.0 > MAX {
                return None;
            }
            ranges.push(range);
        }
        Some(UnicodeRange::from_ranges(ranges))
    }

    pub fn ranges(&self) -> &[(u32, u32)] {
        &self.ranges
    }

    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    pub fn contains(&self, point: u32) -> bool {
        self.ranges
            .binary_search_by(|&(lo, hi)| {
                if hi < point {
                    std::cmp::Ordering::Less
                } else if lo > point {
                    std::cmp::Ordering::Greater
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .is_ok()
    }

    /// Whether any of these code points is in the range.
    pub fn intersects(&self, points: impl IntoIterator<Item = u32>) -> bool {
        points.into_iter().any(|point| self.contains(point))
    }
}

fn code_point(hex: &str) -> Option<u32> {
    if hex.is_empty() || hex.len() > 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(hex, 16)
        .ok()
        .filter(|&point| point <= MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_points_ranges_and_wildcards() {
        let range = UnicodeRange::parse("U+0460-052F, U+20B4, U+4??, u+1F00-1FFF").unwrap();
        assert_eq!(
            range.ranges(),
            &[(0x400, 0x52F), (0x1F00, 0x1FFF), (0x20B4, 0x20B4)]
        );
        assert!(range.contains(0x4FF) && range.contains(0x20B4) && !range.contains(0x20B5));
        assert!(range.intersects([0x41, 0x1F00]));
        assert!(!range.intersects([0x41, 0x42]));
    }

    #[test]
    fn refuses_what_is_not_a_range() {
        for value in [
            "U+",
            "0041",
            "U+41-",
            "U+110000",
            "U+0042-0041",
            "U+?4",
            "U+1234567",
            "U+zz",
        ] {
            assert_eq!(UnicodeRange::parse(value), None, "{value}");
        }
        assert_eq!(
            UnicodeRange::parse("U+??????").unwrap().ranges(),
            &[(0, MAX)]
        );
    }
}
