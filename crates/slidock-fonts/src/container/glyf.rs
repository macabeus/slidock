//! Writes TrueType `glyf` glyph records and the `loca` table from decoded glyph data: short coordinates where they
//! fit, and flags as [`PointEncoding`] says.

use super::ContainerError;
use super::triplet::Point;

const ON_CURVE: u8 = 1 << 0;
const X_SHORT: u8 = 1 << 1;
const Y_SHORT: u8 = 1 << 2;
const REPEAT: u8 = 1 << 3;
const X_SAME_OR_POSITIVE: u8 = 1 << 4;
const Y_SAME_OR_POSITIVE: u8 = 1 << 5;
const OVERLAP_SIMPLE: u8 = 1 << 6;

/// How a simple glyph's flags and coordinates are written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PointEncoding {
    /// As the WOFF2 reference decoder writes them: repeat runs, and a zero delta written as "same" on every point.
    Woff2,
    /// As libeot writes MicroType Express glyphs: no repeat runs, and the first point's deltas always written.
    Mtx,
}

/// A simple glyph's parts.
pub(super) struct SimpleGlyph<'a> {
    /// The number of points of each contour.
    pub(super) contour_points: &'a [u16],
    pub(super) points: &'a [Point],
    pub(super) instructions: &'a [u8],
    /// `xMin, yMin, xMax, yMax`; computed from the points when absent.
    pub(super) bbox: Option<[i16; 4]>,
    /// Sets `OVERLAP_SIMPLE` on the first point.
    pub(super) overlap: bool,
}

fn coordinate(value: i32) -> Result<i16, ContainerError> {
    i16::try_from(value).map_err(|_| ContainerError::GlyphCoordinate)
}

/// The box of the points, `[0; 4]` without points.
fn bbox_of(points: &[Point]) -> Result<[i16; 4], ContainerError> {
    let Some(first) = points.first() else {
        return Ok([0; 4]);
    };
    let (mut x_min, mut y_min, mut x_max, mut y_max) = (first.x, first.y, first.x, first.y);
    for point in points {
        x_min = x_min.min(point.x);
        y_min = y_min.min(point.y);
        x_max = x_max.max(point.x);
        y_max = y_max.max(point.y);
    }
    Ok([
        coordinate(x_min)?,
        coordinate(y_min)?,
        coordinate(x_max)?,
        coordinate(y_max)?,
    ])
}

/// How one coordinate delta is written: its flag bits and its bytes.
fn encode_delta(
    delta: i32,
    short: u8,
    same_or_positive: u8,
    explicit: bool,
    out: &mut Vec<u8>,
) -> Result<u8, ContainerError> {
    if delta == 0 && !explicit {
        return Ok(same_or_positive);
    }
    if (-255..=255).contains(&delta) {
        out.push(u8::try_from(delta.unsigned_abs()).unwrap_or(u8::MAX));
        return Ok(short | if delta >= 0 { same_or_positive } else { 0 });
    }
    out.extend_from_slice(&coordinate(delta)?.to_be_bytes());
    Ok(0)
}

/// Appends a simple glyph record (unpadded) to `out`. Returns its `xMin`.
pub(super) fn write_simple(
    out: &mut Vec<u8>,
    glyph: &SimpleGlyph<'_>,
    encoding: PointEncoding,
) -> Result<i16, ContainerError> {
    let contours =
        i16::try_from(glyph.contour_points.len()).map_err(|_| ContainerError::GlyphCoordinate)?;
    let bbox = match glyph.bbox {
        Some(bbox) => bbox,
        None => bbox_of(glyph.points)?,
    };
    out.extend_from_slice(&contours.to_be_bytes());
    bbox.iter()
        .for_each(|v| out.extend_from_slice(&v.to_be_bytes()));
    let mut end_point: u32 = 0;
    for &count in glyph.contour_points {
        end_point += u32::from(count);
        // A contour ends at its last point; leading empty contours end at -1, written as 0xffff.
        let end = match end_point.checked_sub(1) {
            Some(end) => u16::try_from(end).map_err(|_| ContainerError::GlyphCoordinate)?,
            None => u16::MAX,
        };
        out.extend_from_slice(&end.to_be_bytes());
    }
    let instruction_length =
        u16::try_from(glyph.instructions.len()).map_err(|_| ContainerError::GlyphCoordinate)?;
    out.extend_from_slice(&instruction_length.to_be_bytes());
    out.extend_from_slice(glyph.instructions);

    let mut flags = Vec::with_capacity(glyph.points.len());
    let mut xs = Vec::with_capacity(glyph.points.len());
    let mut ys = Vec::with_capacity(glyph.points.len());
    let (mut last_x, mut last_y) = (0i32, 0i32);
    for (i, point) in glyph.points.iter().enumerate() {
        coordinate(point.x)?;
        coordinate(point.y)?;
        let mut flag = if point.on_curve { ON_CURVE } else { 0 };
        if glyph.overlap && i == 0 {
            flag |= OVERLAP_SIMPLE;
        }
        let explicit = encoding == PointEncoding::Mtx && i == 0;
        flag |= encode_delta(
            point.x - last_x,
            X_SHORT,
            X_SAME_OR_POSITIVE,
            explicit,
            &mut xs,
        )?;
        flag |= encode_delta(
            point.y - last_y,
            Y_SHORT,
            Y_SAME_OR_POSITIVE,
            explicit,
            &mut ys,
        )?;
        flags.push(flag);
        last_x = point.x;
        last_y = point.y;
    }
    if encoding == PointEncoding::Mtx {
        out.extend_from_slice(&flags);
        out.extend_from_slice(&xs);
        out.extend_from_slice(&ys);
        return Ok(bbox[0]);
    }
    let mut repeat: u8 = 0;
    let mut last_flag = None;
    for flag in flags {
        if last_flag == Some(flag) && repeat != u8::MAX {
            if repeat == 0
                && let Some(previous) = out.last_mut()
            {
                *previous |= REPEAT;
            }
            repeat += 1;
            continue;
        }
        if repeat != 0 {
            out.push(repeat);
        }
        out.push(flag);
        repeat = 0;
        last_flag = Some(flag);
    }
    if repeat != 0 {
        out.push(repeat);
    }
    out.extend_from_slice(&xs);
    out.extend_from_slice(&ys);
    Ok(bbox[0])
}

/// Pads `out` with zeros to a multiple of four bytes.
pub(super) fn pad4(out: &mut Vec<u8>) {
    out.resize(out.len().next_multiple_of(4), 0);
}

/// The `loca` table for glyph offsets `offsets` (one more than the glyphs), short (offset / 2) or long.
pub(super) fn loca(offsets: &[u32], long: bool) -> Result<Vec<u8>, ContainerError> {
    let mut out = Vec::with_capacity(offsets.len() * if long { 4 } else { 2 });
    for &offset in offsets {
        if long {
            out.extend_from_slice(&offset.to_be_bytes());
        } else {
            let half = u16::try_from(offset / 2).map_err(|_| ContainerError::LocaOverflow)?;
            if offset % 2 != 0 {
                return Err(ContainerError::LocaOverflow);
            }
            out.extend_from_slice(&half.to_be_bytes());
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: i32, y: i32, on_curve: bool) -> Point {
        Point { x, y, on_curve }
    }

    #[test]
    fn writes_flags_with_repeats_and_short_coordinates() {
        let points = [
            point(0, 0, true),
            point(100, 0, true),
            point(200, 0, true),
            point(200, -300, false),
        ];
        let mut out = Vec::new();
        let x_min = write_simple(
            &mut out,
            &SimpleGlyph {
                contour_points: &[4],
                points: &points,
                instructions: &[0xb0, 0x01],
                bbox: None,
                overlap: true,
            },
            PointEncoding::Woff2,
        )
        .unwrap();
        assert_eq!(x_min, 0);
        let expected: Vec<u8> = [
            &1i16.to_be_bytes()[..],
            &0i16.to_be_bytes(),
            &(-300i16).to_be_bytes(),
            &200i16.to_be_bytes(),
            &0i16.to_be_bytes(),
            &3u16.to_be_bytes(),
            &2u16.to_be_bytes(),
            &[0xb0, 0x01],
            // Point 0: on, overlap, x same, y same. Points 1-2: on, x short positive, y same (repeated once).
            // Point 3: off, x same, y long.
            &[
                ON_CURVE | OVERLAP_SIMPLE | X_SAME_OR_POSITIVE | Y_SAME_OR_POSITIVE,
                ON_CURVE | X_SHORT | X_SAME_OR_POSITIVE | Y_SAME_OR_POSITIVE | REPEAT,
                1,
                X_SAME_OR_POSITIVE,
            ],
            &[100, 100],
            &(-300i16).to_be_bytes(),
        ]
        .concat();
        assert_eq!(out, expected);
    }

    #[test]
    fn mtx_encoding_writes_no_repeats_and_the_first_point_always() {
        let points = [
            point(0, 0, true),
            point(100, 0, true),
            point(200, 0, true),
            point(200, -300, false),
        ];
        let mut out = Vec::new();
        write_simple(
            &mut out,
            &SimpleGlyph {
                contour_points: &[4],
                points: &points,
                instructions: &[],
                bbox: None,
                overlap: false,
            },
            PointEncoding::Mtx,
        )
        .unwrap();
        let expected: Vec<u8> = [
            &1i16.to_be_bytes()[..],
            &0i16.to_be_bytes(),
            &(-300i16).to_be_bytes(),
            &200i16.to_be_bytes(),
            &0i16.to_be_bytes(),
            &3u16.to_be_bytes(),
            &0u16.to_be_bytes(),
            // Point 0: zero deltas written as short positive bytes. Points 1-2: no repeat flag.
            &[
                ON_CURVE | X_SHORT | X_SAME_OR_POSITIVE | Y_SHORT | Y_SAME_OR_POSITIVE,
                ON_CURVE | X_SHORT | X_SAME_OR_POSITIVE | Y_SAME_OR_POSITIVE,
                ON_CURVE | X_SHORT | X_SAME_OR_POSITIVE | Y_SAME_OR_POSITIVE,
                X_SAME_OR_POSITIVE,
            ],
            &[0, 100, 100],
            &[0],
            &(-300i16).to_be_bytes(),
        ]
        .concat();
        assert_eq!(out, expected);
    }

    #[test]
    fn refuses_coordinates_outside_16_bits() {
        let points = [point(40_000, 0, true)];
        let glyph = SimpleGlyph {
            contour_points: &[1],
            points: &points,
            instructions: &[],
            bbox: Some([0; 4]),
            overlap: false,
        };
        assert_eq!(
            write_simple(&mut Vec::new(), &glyph, PointEncoding::Woff2),
            Err(ContainerError::GlyphCoordinate)
        );
    }

    #[test]
    fn short_loca_halves_offsets_and_refuses_what_does_not_fit() {
        assert_eq!(loca(&[0, 4, 8], false).unwrap(), [0, 0, 0, 2, 0, 4]);
        assert_eq!(loca(&[0, 4], true).unwrap(), [0, 0, 0, 0, 0, 0, 0, 4]);
        assert_eq!(
            loca(&[0, 0x2_0000], false),
            Err(ContainerError::LocaOverflow)
        );
        assert_eq!(loca(&[0, 3], false), Err(ContainerError::LocaOverflow));
    }
}
