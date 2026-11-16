//! A table's cells as its tiles store them (storage version 5, the layout numbers-parser documents). Each row of a
//! tile packs its cells into one buffer, found through a uint16 offset per column. A cell is a version byte, its type,
//! six reserved bytes and a uint32 of flags; the fields the flags name follow in flag order, each a uint32 key into one
//! of the table's lists except the values: a decimal128, a double and a date's seconds.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::proto::{tsk, tst};

/// The cell storage this reader knows, in the rows and in each cell.
pub const CELL_STORAGE_VERSION: u32 = 5;

/// TST cell types, as a cell's second byte holds them.
pub mod cell_types {
    pub const EMPTY: u8 = 0;
    pub const NUMBER: u8 = 2;
    pub const TEXT: u8 = 3;
    pub const DATE: u8 = 5;
    pub const BOOL: u8 = 6;
    pub const DURATION: u8 = 7;
    pub const ERROR: u8 = 8;
    pub const RICH_TEXT: u8 = 9;
    pub const CURRENCY: u8 = 10;
}

use cell_types::*;

/// A decimal as Numbers keeps it: `digits` × 10^`exponent`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decimal {
    pub digits: u128,
    pub exponent: i32,
    pub negative: bool,
}

/// One cell's fields, each present when its flag is set.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CellRecord {
    pub cell_type: u8,
    pub decimal: Option<Decimal>,
    pub double: Option<f64>,
    /// A date: seconds since 2001-01-01 UTC.
    pub seconds: Option<f64>,
    pub string: Option<u32>,
    pub rich_text: Option<u32>,
    pub cell_style: Option<u32>,
    pub text_style: Option<u32>,
    pub conditional_style: Option<u32>,
    pub number_format: Option<u32>,
    pub currency_format: Option<u32>,
    pub date_format: Option<u32>,
    pub duration_format: Option<u32>,
    pub bool_format: Option<u32>,
}

impl CellRecord {
    pub fn empty() -> Self {
        CellRecord::default()
    }
}

/// Where a flag's field goes.
#[derive(Clone, Copy, Debug)]
enum Field {
    Decimal,
    Double,
    Seconds,
    Key(fn(&mut CellRecord) -> &mut Option<u32>),
    Skipped,
}

/// Each flag's field and its width, in the order the fields follow the header.
const FIELDS: [(u32, Field, usize); 19] = [
    (0x1, Field::Decimal, 16),
    (0x2, Field::Double, 8),
    (0x4, Field::Seconds, 8),
    (0x8, Field::Key(|c| &mut c.string), 4),
    (0x10, Field::Key(|c| &mut c.rich_text), 4),
    (0x20, Field::Key(|c| &mut c.cell_style), 4),
    (0x40, Field::Key(|c| &mut c.text_style), 4),
    (0x80, Field::Key(|c| &mut c.conditional_style), 4),
    // The applied conditional rule, a formula, a control, a formula error and a suggestion.
    (0x100, Field::Skipped, 4),
    (0x200, Field::Skipped, 4),
    (0x400, Field::Skipped, 4),
    (0x800, Field::Skipped, 4),
    (0x1000, Field::Skipped, 4),
    (0x2000, Field::Key(|c| &mut c.number_format), 4),
    (0x4000, Field::Key(|c| &mut c.currency_format), 4),
    (0x8000, Field::Key(|c| &mut c.date_format), 4),
    (0x10000, Field::Key(|c| &mut c.duration_format), 4),
    // The text format.
    (0x20000, Field::Skipped, 4),
    (0x40000, Field::Key(|c| &mut c.bool_format), 4),
];

const HEADER_BYTES: usize = 12;
const NO_CELL: u16 = 0xffff;
const WIDE_OFFSET_UNIT: usize = 4;

/// A tile row that does not read as cells: the table cannot be shown.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("{0}")]
pub struct CellStorageError(pub String);

/// IEEE 754 decimal128 as Numbers writes it: a 113-bit little-endian coefficient, a 14-bit exponent and a sign.
pub fn decimal128(bytes: &[u8; 16]) -> Decimal {
    let [
        b0,
        b1,
        b2,
        b3,
        b4,
        b5,
        b6,
        b7,
        b8,
        b9,
        b10,
        b11,
        b12,
        b13,
        b14,
        b15,
    ] = *bytes;
    let mut digits = u128::from(b14 & 1);
    for byte in [b13, b12, b11, b10, b9, b8, b7, b6, b5, b4, b3, b2, b1, b0] {
        digits = (digits << 8) | u128::from(byte);
    }
    let exponent = ((i32::from(b15 & 0x7f) << 7) | i32::from(b14 >> 1)) - 0x1820;
    Decimal {
        digits,
        exponent,
        negative: b15 & 0x80 != 0,
    }
}

fn bytes_at<const N: usize>(buffer: &[u8], at: usize) -> Option<[u8; N]> {
    buffer.get(at..at.checked_add(N)?)?.try_into().ok()
}

fn cell_at(buffer: &[u8], at: usize) -> Result<CellRecord, CellStorageError> {
    let past = || CellStorageError(format!("a cell at {at} runs past its row"));
    let header: [u8; HEADER_BYTES] = bytes_at(buffer, at).ok_or_else(past)?;
    let [version, cell_type, _, _, _, _, _, _, f0, f1, f2, f3] = header;
    if u32::from(version) != CELL_STORAGE_VERSION {
        return Err(CellStorageError(format!("cell storage version {version}")));
    }
    let flags = u32::from_le_bytes([f0, f1, f2, f3]);
    let mut cell = CellRecord {
        cell_type,
        ..CellRecord::default()
    };
    let mut pos = at + HEADER_BYTES;
    for (flag, field, width) in FIELDS {
        if flags & flag == 0 {
            continue;
        }
        let double = || bytes_at::<8>(buffer, pos).map(f64::from_le_bytes);
        match field {
            Field::Decimal => {
                cell.decimal = Some(decimal128(&bytes_at(buffer, pos).ok_or_else(past)?))
            }
            Field::Double => cell.double = Some(double().ok_or_else(past)?),
            Field::Seconds => cell.seconds = Some(double().ok_or_else(past)?),
            Field::Key(slot) => {
                *slot(&mut cell) =
                    Some(u32::from_le_bytes(bytes_at(buffer, pos).ok_or_else(past)?));
            }
            Field::Skipped => {
                bytes_at::<4>(buffer, pos).ok_or_else(past)?;
            }
        }
        pos += width;
    }
    Ok(cell)
}

/// A tile row's cells by column.
pub fn row_cells(info: &tst::TileRowInfo) -> Result<BTreeMap<usize, CellRecord>, CellStorageError> {
    let (Some(buffer), Some(offsets)) = (
        info.cell_storage_buffer.as_deref(),
        info.cell_offsets.as_deref(),
    ) else {
        return Err(version_error(info.storage_version));
    };
    if info.storage_version != Some(CELL_STORAGE_VERSION) {
        return Err(version_error(info.storage_version));
    }
    let unit = if info.has_wide_offsets == Some(true) {
        WIDE_OFFSET_UNIT
    } else {
        1
    };
    let mut cells = BTreeMap::new();
    for (column, pair) in offsets.as_chunks::<2>().0.iter().enumerate() {
        let offset = u16::from_le_bytes(*pair);
        if offset != NO_CELL {
            cells.insert(column, cell_at(buffer, usize::from(offset) * unit)?);
        }
    }
    Ok(cells)
}

fn version_error(version: Option<u32>) -> CellStorageError {
    CellStorageError(format!(
        "cell storage version {}",
        version.map_or_else(|| "none".to_owned(), |v| v.to_string())
    ))
}

/// A TSK format's type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FormatType {
    Decimal,
    Currency,
    Percentage,
    Scientific,
    Text,
    Date,
    Fraction,
    Checkbox,
    Rating,
    Duration,
    Base,
    Custom,
}

impl FormatType {
    fn of(value: u32) -> Self {
        match value {
            256 => FormatType::Decimal,
            257 => FormatType::Currency,
            258 => FormatType::Percentage,
            259 => FormatType::Scientific,
            260 => FormatType::Text,
            261 => FormatType::Date,
            262 => FormatType::Fraction,
            263 => FormatType::Checkbox,
            267 => FormatType::Rating,
            268 => FormatType::Duration,
            269 => FormatType::Base,
            _ => FormatType::Custom,
        }
    }

    /// The format's type, decimal when it states none.
    fn of_format(format: Option<&tsk::FormatStructArchive>) -> Self {
        format
            .and_then(|f| f.format_type)
            .map_or(FormatType::Decimal, FormatType::of)
    }

    /// The name a value shown plain reports.
    fn name(self) -> &'static str {
        match self {
            FormatType::Decimal => "decimal",
            FormatType::Currency => "currency",
            FormatType::Percentage => "percentage",
            FormatType::Scientific => "scientific",
            FormatType::Text => "text",
            FormatType::Date => "date",
            FormatType::Fraction => "fraction",
            FormatType::Checkbox => "checkbox",
            FormatType::Rating => "rating",
            FormatType::Duration => "duration",
            FormatType::Base => "base",
            FormatType::Custom => "custom",
        }
    }
}

const AUTOMATIC_PLACES: u32 = 253;
const MAX_PLACES: i64 = 20;
const MAX_EXPONENT: i64 = 308;

/// A value's text, and the format it leaves unapplied when it is shown plain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellValue {
    pub text: String,
    pub approximated: Option<String>,
}

impl CellValue {
    fn plain(text: String) -> Self {
        CellValue {
            text,
            approximated: None,
        }
    }

    fn approximated(text: String, format: &str) -> Self {
        CellValue {
            text,
            approximated: Some(format.to_owned()),
        }
    }
}

/// The decimal of a double: its shortest round-trip digits; zero for a value that is not finite.
fn double_decimal(value: f64) -> Decimal {
    if !value.is_finite() {
        return Decimal {
            digits: 0,
            exponent: 0,
            negative: false,
        };
    }
    // `{:e}` prints the shortest round-trip digits, as `toExponential()` does.
    let text = format!("{:e}", value.abs());
    let (mantissa, power) = text.split_once('e').unwrap_or((text.as_str(), "0"));
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = format!("{whole}{fraction}").parse().unwrap_or(0);
    let power: i32 = power.parse().unwrap_or(0);
    Decimal {
        digits,
        exponent: power - fraction.len() as i32,
        negative: value < 0.0,
    }
}

/// `10^n`, when it fits.
fn pow10(n: i64) -> Option<u128> {
    10u128.checked_pow(u32::try_from(n).ok()?)
}

/// A decimal written out with `places` decimals, rounded half away from zero, or exactly with its trailing zeros
/// dropped when `places` is `None` (automatic). Numbers holds values in a double's range: an exponent past it is
/// clamped, and automatic decimals stop at MAX_PLACES.
fn write_decimal(decimal: Decimal, places: Option<i64>, grouped: bool) -> String {
    let kept = places.unwrap_or(MAX_PLACES);
    let mut shown = decimal.digits;
    let mut at = i64::from(decimal.exponent).min(MAX_EXPONENT);
    if at < -kept {
        // A divisor past u128 is more than twice any coefficient: the value rounds to 0.
        shown = match pow10(-kept - at) {
            Some(drop) => shown.saturating_add(drop / 2) / drop,
            None => 0,
        };
        at = -kept;
    }
    let decimals = (-at).max(0) as usize;
    let mut text = shown.to_string();
    text.push_str(&"0".repeat(at.max(0) as usize));
    if text.len() < decimals + 1 {
        text = format!("{}{text}", "0".repeat(decimals + 1 - text.len()));
    }
    let split = text.len() - decimals;
    let (whole, written) = text.split_at(split);
    let fraction = match places {
        None => written.trim_end_matches('0').to_owned(),
        Some(places) => format!("{written:0<width$}", width = places.max(0) as usize),
    };
    let whole = if grouped {
        group(whole)
    } else {
        whole.to_owned()
    };
    if fraction.is_empty() {
        whole
    } else {
        format!("{whole}.{fraction}")
    }
}

/// Digits with a comma before each group of three from the right.
fn group(digits: &str) -> String {
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A number in its format: decimal and percentage formats apply; any other shows the plain number.
fn number_text(value: Decimal, format: Option<&tsk::FormatStructArchive>) -> CellValue {
    let kind = FormatType::of_format(format);
    let percent = kind == FormatType::Percentage;
    let known = matches!(
        kind,
        FormatType::Decimal | FormatType::Percentage | FormatType::Text
    );
    let places = format
        .and_then(|f| f.decimal_places)
        .filter(|places| known && *places != AUTOMATIC_PLACES)
        .map(|places| i64::from(places).min(MAX_PLACES));
    let scaled = if percent {
        Decimal {
            exponent: value.exponent.saturating_add(2),
            ..value
        }
    } else {
        value
    };
    let grouped = known && format.and_then(|f| f.show_thousands_separator) == Some(true);
    let written = write_decimal(scaled, places, grouped);
    let negative = value.negative && written.bytes().any(|b| (b'1'..=b'9').contains(&b));
    let text = format!(
        "{}{written}{}",
        if negative { "-" } else { "" },
        if percent { "%" } else { "" }
    );
    if !known {
        return CellValue::approximated(text, kind.name());
    }
    if negative && format.and_then(|f| f.negative_style).unwrap_or(0) != 0 {
        return CellValue::approximated(text, "negative number");
    }
    CellValue::plain(text)
}

/// Days from 1970-01-01 to 2001-01-01, Numbers' epoch.
const EPOCH_DAYS: i64 = 11_323;
const MAX_DAYS: f64 = 100_000_000.0;

/// A day count since 1970-01-01 as an ISO date (the proleptic Gregorian calendar, by eras of 400 years).
fn iso_date(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{}{:04}-{:02}-{:02}",
        if year < 0 { "-" } else { "" },
        year.abs(),
        month,
        day
    )
}

/// The text a cell shows for its value: a number in its decimal or percentage format, a currency as its plain number,
/// a date as an ISO date, a duration in seconds, a boolean as TRUE or FALSE, an error as #ERROR; `None` for a type
/// this reader does not know. Text and rich text are read by the caller.
pub fn value_text<'f>(
    cell: &CellRecord,
    format: impl Fn(Option<u32>) -> Option<&'f tsk::FormatStructArchive>,
) -> Option<CellValue> {
    let value = || {
        cell.decimal
            .unwrap_or_else(|| double_decimal(cell.double.unwrap_or(0.0)))
    };
    Some(match cell.cell_type {
        EMPTY => CellValue::plain(String::new()),
        NUMBER => number_text(
            value(),
            format(cell.number_format).or_else(|| format(cell.currency_format)),
        ),
        CURRENCY => {
            let shown = number_text(
                value(),
                format(cell.currency_format).or_else(|| format(cell.number_format)),
            );
            CellValue {
                approximated: Some(
                    shown
                        .approximated
                        .unwrap_or_else(|| FormatType::Currency.name().to_owned()),
                ),
                text: shown.text,
            }
        }
        DATE => {
            let days = (cell.seconds.unwrap_or(0.0) / 86_400.0).floor();
            let text = if days.is_finite() && days.abs() < MAX_DAYS {
                // Within ±10^8, so it fits.
                iso_date(EPOCH_DAYS + days as i64)
            } else {
                String::new()
            };
            CellValue::approximated(text, FormatType::Date.name())
        }
        DURATION => CellValue::approximated(
            format!(
                "{}s",
                write_decimal(double_decimal(cell.double.unwrap_or(0.0)), None, false)
            ),
            FormatType::Duration.name(),
        ),
        BOOL => {
            let text = if cell.double.unwrap_or(0.0) != 0.0 {
                "TRUE"
            } else {
                "FALSE"
            };
            if FormatType::of_format(format(cell.bool_format)) == FormatType::Checkbox {
                CellValue::approximated(text.to_owned(), FormatType::Checkbox.name())
            } else {
                CellValue::plain(text.to_owned())
            }
        }
        ERROR => CellValue::approximated("#ERROR".to_owned(), "formula error"),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_digits_in_threes_from_the_right() {
        assert_eq!(group("1"), "1");
        assert_eq!(group("123"), "123");
        assert_eq!(group("1234"), "1,234");
        assert_eq!(group("1234567"), "1,234,567");
    }

    #[test]
    fn writes_dates_across_eras() {
        assert_eq!(iso_date(0), "1970-01-01");
        assert_eq!(iso_date(EPOCH_DAYS), "2001-01-01");
        assert_eq!(iso_date(-719_468 - 1), "0000-02-29");
        assert_eq!(iso_date(-719_468 - 366), "-0001-03-01");
        assert_eq!(iso_date(EPOCH_DAYS - 50_000_000), "-134895-08-27");
    }
}
