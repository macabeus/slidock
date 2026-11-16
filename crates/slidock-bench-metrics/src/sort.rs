//! `Array.prototype.sort` with a comparator, as JavaScriptCore runs it (`runtime/StableSort.h`): binary insertion
//! below 8 elements; otherwise powersort over natural runs (runs shorter than 8 extended to 64 by insertion), merged
//! with TimSort's galloping merge. Any stable sort gives the same result for a consistent order; this one also gives
//! JavaScriptCore's result when the comparator is not one (a NaN difference compares neither less nor greater), and
//! never panics.
//!
//! Every index stays inside its slice by construction; reads and writes go through [`at`] and [`put`] so that a
//! slip would give a wrong order, not a panic.

/// Below this length, and for runs shorter than this, insertion sort.
const EXTEND_RUN_CUTOFF: usize = 8;
/// A short run is extended by insertion to this many elements.
const FORCE_RUN_LENGTH: usize = 64;
const MIN_GALLOP_THRESHOLD: usize = 7;

fn at(values: &[f64], i: usize) -> f64 {
    values.get(i).copied().unwrap_or(f64::NAN)
}

fn put(values: &mut [f64], i: usize, value: f64) {
    if let Some(slot) = values.get_mut(i) {
        *slot = value;
    }
}

fn copy(dst: &mut [f64], dst_at: usize, src: &[f64], src_at: usize, len: usize) {
    if let (Some(to), Some(from)) = (
        dst.get_mut(dst_at..dst_at + len),
        src.get(src_at..src_at + len),
    ) {
        to.copy_from_slice(from);
    }
}

fn tail(values: &[f64], from: usize, to: usize) -> &[f64] {
    values.get(from..to).unwrap_or_default()
}

/// Sorts `values` ascending by `less` (the comparator's result `< 0`), stably.
pub fn stable_sort(values: &mut [f64], less: impl Fn(f64, f64) -> bool) {
    let n = values.len();
    if n < EXTEND_RUN_CUTOFF {
        insertion_sort(values, 0, &less);
        return;
    }
    let mut work = vec![0.0; n];
    let mut stack: Vec<(Run, u32)> = Vec::new();
    let mut min_gallop = MIN_GALLOP_THRESHOLD;
    let mut run1 = next_run(values, 0, &less);
    while run1.end + 1 < n {
        let run2 = next_run(values, run1.end + 1, &less);
        let p = power(run1.begin, run2.begin, run2.end, n);
        while let Some(&(top, top_power)) = stack.last()
            && top_power > p
        {
            stack.pop();
            merge_into_place(values, &mut work, top, run1, &less, &mut min_gallop);
            run1.begin = top.begin;
        }
        stack.push((run1, p));
        run1 = run2;
    }
    while let Some((top, _)) = stack.pop() {
        merge_into_place(values, &mut work, top, run1, &less, &mut min_gallop);
        run1.begin = top.begin;
    }
}

/// A sorted range, both ends inclusive.
#[derive(Clone, Copy, Debug)]
struct Run {
    begin: usize,
    end: usize,
}

/// The run starting at `begin`: the natural run (a strictly descending one reversed), extended by insertion to 64
/// elements when it is shorter than 8, then by any following elements already in order.
fn next_run(values: &mut [f64], begin: usize, less: &impl Fn(f64, f64) -> bool) -> Run {
    let n = values.len();
    let mut run = Run {
        begin,
        end: extend_and_normalise_run(values, begin, less),
    };
    if run.end - run.begin < EXTEND_RUN_CUTOFF {
        let size = FORCE_RUN_LENGTH.min(n - run.begin);
        if let Some(part) = values.get_mut(run.begin..run.begin + size) {
            insertion_sort(part, run.end - run.begin, less);
        }
        run.end = run.begin + size - 1;
    }
    while run.end + 1 < n && !less(at(values, run.end + 1), at(values, run.end)) {
        run.end += 1;
    }
    run
}

/// Merges two adjacent runs through `work` back into `values`.
fn merge_into_place(
    values: &mut [f64],
    work: &mut [f64],
    first: Run,
    second: Run,
    less: &impl Fn(f64, f64) -> bool,
    min_gallop: &mut usize,
) {
    merge_runs(
        work,
        values,
        (first.begin, first.end + 1),
        (second.begin, second.end + 1),
        less,
        min_gallop,
    );
    let len = second.end + 1 - first.begin;
    copy(values, first.begin, work, first.begin, len);
}

/// Binary insertion sort of `values`, whose first `sorted_header + 1` elements are already in order. An element goes
/// after every element it is not less than.
fn insertion_sort(values: &mut [f64], sorted_header: usize, less: &impl Fn(f64, f64) -> bool) {
    for i in sorted_header + 1..values.len() {
        let value = at(values, i);
        let (mut left, mut right) = (0, i);
        while left < right {
            let m = left + (right - left) / 2;
            if less(value, at(values, m)) {
                right = m;
            } else {
                left = m + 1;
            }
        }
        if let Some(part) = values.get_mut(left..=i) {
            part.rotate_right(1);
        }
    }
}

/// The end (inclusive) of the run starting at `begin`: non-descending, or strictly descending and then reversed.
fn extend_and_normalise_run(
    values: &mut [f64],
    begin: usize,
    less: &impl Fn(f64, f64) -> bool,
) -> usize {
    let n = values.len();
    let mut end = begin;
    if end + 1 >= n {
        return end;
    }
    let descending = less(at(values, end + 1), at(values, end));
    end += 1;
    while end + 1 < n && less(at(values, end + 1), at(values, end)) == descending {
        end += 1;
    }
    if descending && let Some(part) = values.get_mut(begin..=end) {
        part.reverse();
    }
    end
}

/// The next offset of an exponential search: `2·offset + 1`, capped at `max`.
fn grow(offset: usize, max: usize) -> usize {
    let next = (offset << 1) + 1;
    if next > offset { next.min(max) } else { max }
}

/// The leftmost insertion point of `key` in `base` (before equal elements), searched outward from `hint`.
fn gallop_left(key: f64, base: &[f64], hint: usize, less: &impl Fn(f64, f64) -> bool) -> usize {
    let length = base.len();
    let mut last = 0usize;
    let mut offset = 1usize;
    if less(at(base, hint), key) {
        let max = length - hint;
        while offset < max && less(at(base, hint + offset), key) {
            last = offset;
            offset = grow(offset, max);
        }
        last += hint;
        offset += hint;
    } else {
        let max = hint + 1;
        while offset < max && !less(at(base, hint - offset), key) {
            last = offset;
            offset = grow(offset, max);
        }
        let near = last;
        // One below 0 when the search reached the start; the increment below brings it back.
        last = hint.wrapping_sub(offset);
        offset = hint - near;
    }
    last = last.wrapping_add(1);
    while last < offset {
        let m = last + ((offset - last) >> 1);
        if less(at(base, m), key) {
            last = m + 1;
        } else {
            offset = m;
        }
    }
    offset
}

/// The rightmost insertion point of `key` in `base` (after equal elements), searched outward from `hint`.
fn gallop_right(key: f64, base: &[f64], hint: usize, less: &impl Fn(f64, f64) -> bool) -> usize {
    let length = base.len();
    let mut last = 0usize;
    let mut offset = 1usize;
    if less(key, at(base, hint)) {
        let max = hint + 1;
        while offset < max && less(key, at(base, hint - offset)) {
            last = offset;
            offset = grow(offset, max);
        }
        let near = last;
        last = hint.wrapping_sub(offset);
        offset = hint - near;
    } else {
        let max = length - hint;
        while offset < max && !less(key, at(base, hint + offset)) {
            last = offset;
            offset = grow(offset, max);
        }
        last += hint;
        offset += hint;
    }
    last = last.wrapping_add(1);
    while last < offset {
        let m = last + ((offset - last) >> 1);
        if less(key, at(base, m)) {
            offset = m;
        } else {
            last = m + 1;
        }
    }
    offset
}

/// Merges the adjacent sorted ranges `[a.0, a.1)` and `[b.0, b.1)` of `src` into the same positions of `dst`:
/// elements already in place at either end are trimmed by galloping, then a linear merge switches to galloping
/// while one side keeps winning `min_gallop` times (adapted across merges).
fn merge_runs(
    dst: &mut [f64],
    src: &[f64],
    (start, left_end): (usize, usize),
    (right_start, end): (usize, usize),
    less: &impl Fn(f64, f64) -> bool,
    min_gallop: &mut usize,
) {
    let mut left_length = left_end - start;
    let mut right_length = end - right_start;
    if left_length == 0 || right_length == 0 {
        copy(dst, start, src, start, end - start);
        return;
    }

    let skip_left = gallop_right(at(src, right_start), tail(src, start, left_end), 0, less);
    copy(dst, start, src, start, skip_left);
    let mut left = start + skip_left;
    left_length -= skip_left;
    if left_length == 0 {
        copy(dst, right_start, src, right_start, right_length);
        return;
    }

    let skip_right = right_length
        - gallop_left(
            at(src, left_end - 1),
            tail(src, right_start, end),
            right_length - 1,
            less,
        );
    copy(dst, end - skip_right, src, end - skip_right, skip_right);
    let right_end = end - skip_right;
    right_length -= skip_right;
    if right_length == 0 {
        copy(dst, left, src, left, left_length);
        return;
    }

    let mut right = right_start;
    let mut d = left;
    'merge: loop {
        let mut left_wins = 0;
        let mut right_wins = 0;
        while left_wins < *min_gallop && right_wins < *min_gallop {
            if !(right < right_end && left < left_end) {
                break 'merge;
            }
            if less(at(src, right), at(src, left)) {
                put(dst, d, at(src, right));
                right += 1;
                right_wins += 1;
                left_wins = 0;
            } else {
                put(dst, d, at(src, left));
                left += 1;
                left_wins += 1;
                right_wins = 0;
            }
            d += 1;
        }

        *min_gallop += 1;
        loop {
            if *min_gallop > 1 {
                *min_gallop -= 1;
            }
            if left >= left_end || right >= right_end {
                break 'merge;
            }

            let k = gallop_right(at(src, right), tail(src, left, left_end), 0, less);
            left_wins = k;
            copy(dst, d, src, left, k);
            d += k;
            left += k;
            put(dst, d, at(src, right));
            d += 1;
            right += 1;
            if left >= left_end || right >= right_end {
                break 'merge;
            }

            let k = gallop_left(at(src, left), tail(src, right, right_end), 0, less);
            right_wins = k;
            copy(dst, d, src, right, k);
            d += k;
            right += k;
            put(dst, d, at(src, left));
            d += 1;
            left += 1;
            if left >= left_end || right >= right_end {
                break 'merge;
            }

            if left_wins < MIN_GALLOP_THRESHOLD && right_wins < MIN_GALLOP_THRESHOLD {
                break;
            }
        }
        *min_gallop += 1;
    }

    copy(dst, d, src, left, left_end - left);
    d += left_end - left;
    copy(dst, d, src, right, right_end - right);
}

/// Powersort's node power of the boundary between the runs `[left, middle)` and `[middle, right]` of `n` elements.
fn power(left: usize, middle: usize, right: usize, n: usize) -> u32 {
    let n1 = (middle - left) as u128;
    let n2 = (right - middle + 1) as u128;
    let a = ((left as u128) * 2 + n1) << 62;
    let b = ((middle as u128) * 2 + n2) << 62;
    let n = n as u128;
    let differing = (a / n) ^ (b / n);
    (differing as u64).leading_zeros()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn by_difference(a: f64, b: f64) -> bool {
        a - b < 0.0
    }

    fn sorted(values: &[f64]) -> Vec<f64> {
        let mut v = values.to_vec();
        stable_sort(&mut v, by_difference);
        v
    }

    fn same(a: &[f64], b: &[f64]) -> bool {
        a.len() == b.len()
            && a.iter()
                .zip(b)
                .all(|(x, y)| x.to_bits() == y.to_bits() || (x.is_nan() && y.is_nan()))
    }

    #[test]
    fn sorts_a_consistent_order_stably() {
        let input: Vec<f64> = (0..500).map(|i| ((i * 7919) % 211) as f64 / 4.0).collect();
        let mut want = input.clone();
        want.sort_by(f64::total_cmp);
        assert!(same(&sorted(&input), &want));
        // Equal under the comparator, the zeros keep their order.
        let zeros: Vec<f64> = (0..300)
            .map(|i| if i % 3 == 0 { -0.0 } else { 0.0 })
            .collect();
        assert!(same(&sorted(&zeros), &zeros));
        let descending: Vec<f64> = (0..100).rev().map(f64::from).collect();
        let ascending: Vec<f64> = (0..100).map(f64::from).collect();
        assert!(same(&sorted(&descending), &ascending));
    }

    #[test]
    fn short_lists_sort_by_binary_insertion() {
        // JavaScriptCore: [3, NaN, 1] stays, [NaN, 3, 1] gives [NaN, 1, 3], [2, NaN, 1, NaN, 0] gives
        // [2, NaN, 0, 1, NaN].
        let nan = f64::NAN;
        assert!(same(&sorted(&[3.0, nan, 1.0]), &[3.0, nan, 1.0]));
        assert!(same(&sorted(&[nan, 3.0, 1.0]), &[nan, 1.0, 3.0]));
        assert!(same(
            &sorted(&[2.0, nan, 1.0, nan, 0.0]),
            &[2.0, nan, 0.0, 1.0, nan]
        ));
    }

    #[test]
    fn an_inconsistent_order_sorts_as_javascriptcore_does() {
        let nan = f64::NAN;
        let input = [
            3.0, nan, 1.0, 2.0, nan, 0.5, 9.0, 4.0, nan, 7.0, 6.0, 5.0, 8.0, nan, 2.5, 1.5, 0.25,
            3.5, 4.5, 5.5, 6.5,
        ];
        let want = [
            3.0, nan, 0.25, 1.0, 1.5, 2.0, nan, 0.5, 2.5, 3.5, 4.0, 4.5, 5.0, 5.5, 6.0, 6.5, 7.0,
            9.0, nan, 8.0, nan,
        ];
        assert!(same(&sorted(&input), &want));
    }

    #[test]
    fn power_of_run_boundaries() {
        // n = 8 split in halves: the boundary is the root (power 1); quarter boundaries have power 2.
        assert_eq!(power(0, 4, 7, 8), 1);
        assert_eq!(power(0, 2, 3, 8), 2);
        assert_eq!(power(4, 6, 7, 8), 2);
    }
}
