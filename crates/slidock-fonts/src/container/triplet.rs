//! The triplet encoding of glyph points, shared by WOFF2's transformed `glyf` table and MicroType Express's
//! compact TrueType glyph data.
//!
//! Each point has one flag byte. Its top bit clear means the point is on the curve; its low seven bits pick one of
//! 128 ways to read the point's (dx, dy) from the next one to four data bytes. Coordinates accumulate from (0, 0).

/// One glyph point, with absolute coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
    pub on_curve: bool,
}

/// Why triplet data can't be decoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TripletError {
    #[error("The glyph data ends inside a point")]
    Truncated,
    #[error("A glyph coordinate overflows")]
    Overflow,
}

/// The number of data bytes of a flag (its top bit cleared).
fn data_bytes(flag: u8) -> usize {
    match flag {
        0..84 => 1,
        84..120 => 2,
        120..124 => 3,
        _ => 4,
    }
}

/// `value` negated unless the flag bit is set.
fn with_sign(flag: u8, value: i32) -> i32 {
    if flag & 1 != 0 { value } else { -value }
}

/// The (dx, dy) of one point from its flag (top bit cleared) and data bytes.
fn delta(flag: u8, data: &[u8]) -> Result<(i32, i32), TripletError> {
    let byte = |i: usize| {
        data.get(i)
            .map(|&b| i32::from(b))
            .ok_or(TripletError::Truncated)
    };
    let flag_value = i32::from(flag);
    Ok(match flag {
        0..10 => (0, with_sign(flag, ((flag_value & 14) << 7) + byte(0)?)),
        10..20 => (
            with_sign(flag, (((flag_value - 10) & 14) << 7) + byte(0)?),
            0,
        ),
        20..84 => {
            let b0 = flag_value - 20;
            let b1 = byte(0)?;
            (
                with_sign(flag, 1 + (b0 & 0x30) + (b1 >> 4)),
                with_sign(flag >> 1, 1 + ((b0 & 0x0c) << 2) + (b1 & 0x0f)),
            )
        }
        84..120 => {
            let b0 = flag_value - 84;
            (
                with_sign(flag, 1 + ((b0 / 12) << 8) + byte(0)?),
                with_sign(flag >> 1, 1 + (((b0 % 12) >> 2) << 8) + byte(1)?),
            )
        }
        120..124 => {
            let b1 = byte(1)?;
            (
                with_sign(flag, (byte(0)? << 4) + (b1 >> 4)),
                with_sign(flag >> 1, ((b1 & 0x0f) << 8) + byte(2)?),
            )
        }
        _ => (
            with_sign(flag, (byte(0)? << 8) + byte(1)?),
            with_sign(flag >> 1, (byte(2)? << 8) + byte(3)?),
        ),
    })
}

/// Decodes one point per flag from `data`, appending them to `points`. Returns the data bytes read.
pub fn decode_into(
    flags: &[u8],
    data: &[u8],
    points: &mut Vec<Point>,
) -> Result<usize, TripletError> {
    // Every point reads at least one data byte, which bounds the allocation by the input.
    if flags.len() > data.len() {
        return Err(TripletError::Truncated);
    }
    points.reserve(flags.len());
    let (mut x, mut y, mut at) = (0i32, 0i32, 0usize);
    for &flag in flags {
        let on_curve = flag & 0x80 == 0;
        let flag = flag & 0x7f;
        let length = data_bytes(flag);
        let bytes = data.get(at..at + length).ok_or(TripletError::Truncated)?;
        let (dx, dy) = delta(flag, bytes)?;
        x = x.checked_add(dx).ok_or(TripletError::Overflow)?;
        y = y.checked_add(dy).ok_or(TripletError::Overflow)?;
        at += length;
        points.push(Point { x, y, on_curve });
    }
    Ok(at)
}

/// Decodes one point per flag from `data`: the points and the data bytes read.
pub fn decode(flags: &[u8], data: &[u8]) -> Result<(Vec<Point>, usize), TripletError> {
    let mut points = Vec::new();
    let read = decode_into(flags, data, &mut points)?;
    Ok((points, read))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(flag: u8, data: &[u8]) -> (i32, i32) {
        let (points, read) = decode(&[flag], data).unwrap();
        assert_eq!(read, data_bytes(flag & 0x7f));
        (points[0].x, points[0].y)
    }

    #[test]
    fn every_flag_range_decodes_as_the_woff2_table_says() {
        // Flags 0-9: dy only, 8 extra bits from the flag; bit 0 is the sign.
        assert_eq!(one(0, &[5]), (0, -5));
        assert_eq!(one(1, &[5]), (0, 5));
        assert_eq!(one(3, &[5]), (0, 256 + 5));
        assert_eq!(one(9, &[255]), (0, 1024 + 255));
        // Flags 10-19: dx only.
        assert_eq!(one(10, &[7]), (-7, 0));
        assert_eq!(one(13, &[7]), (256 + 7, 0));
        // Flags 20-83: four bits each from one byte.
        assert_eq!(one(20, &[0x00]), (-1, -1));
        assert_eq!(one(23, &[0x21]), (3, 2));
        assert_eq!(one(83, &[0xff]), (64, 64));
        // Flags 84-119: one byte each, with high bits from the flag.
        assert_eq!(one(84, &[0, 0]), (-1, -1));
        assert_eq!(one(119, &[255, 255]), (768, 768));
        // Flags 120-123: twelve bits each.
        assert_eq!(one(123, &[0xab, 0xcd, 0xef]), (0xabc, 0xdef));
        assert_eq!(one(120, &[0x00, 0x01, 0x02]), (0, -0x102));
        // Flags 124-127: sixteen bits each.
        assert_eq!(one(127, &[0x12, 0x34, 0x56, 0x78]), (0x1234, 0x5678));
        assert_eq!(one(124, &[0xff, 0xff, 0xff, 0xff]), (-65535, -65535));
    }

    #[test]
    fn points_accumulate_and_carry_the_on_curve_bit() {
        let (points, read) = decode(&[0x01, 0x8b], &[10, 20]).unwrap();
        assert_eq!(read, 2);
        assert_eq!(
            points,
            [
                Point {
                    x: 0,
                    y: 10,
                    on_curve: true
                },
                Point {
                    x: 20,
                    y: 10,
                    on_curve: false
                }
            ]
        );
    }

    #[test]
    fn short_data_is_refused_before_any_allocation() {
        assert_eq!(decode(&[0, 0], &[1]), Err(TripletError::Truncated));
        assert_eq!(decode(&[127], &[1, 2, 3]), Err(TripletError::Truncated));
    }

    #[test]
    fn coordinates_never_wrap() {
        let flags = vec![127u8; 40_000];
        let data = vec![0xffu8; 160_000];
        assert_eq!(decode(&flags, &data), Err(TripletError::Overflow));
    }
}
