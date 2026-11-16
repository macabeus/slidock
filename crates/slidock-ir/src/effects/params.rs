//! The parameter types effects share: directions in the content-moves convention, orientations, shapes and
//! colours. Every enum reads only its own tokens; a reader maps the source's values onto them, and a value it does
//! not know falls back to a declared default with a diagnostic instead of being coerced here.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use ts_rs::TS;

/// A direction, always the one the content moves, in OOXML's tokens: "fly in from the left" moves right (`r`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[ts(export_to = "ir/")]
pub enum Dir {
    #[serde(rename = "l")]
    Left,
    #[serde(rename = "r")]
    Right,
    #[serde(rename = "u")]
    Up,
    #[serde(rename = "d")]
    Down,
    #[serde(rename = "lu")]
    LeftUp,
    #[serde(rename = "ru")]
    RightUp,
    #[serde(rename = "ld")]
    LeftDown,
    #[serde(rename = "rd")]
    RightDown,
}

impl Dir {
    pub const ALL: [Dir; 8] = [
        Dir::Left,
        Dir::Right,
        Dir::Up,
        Dir::Down,
        Dir::LeftUp,
        Dir::RightUp,
        Dir::LeftDown,
        Dir::RightDown,
    ];

    pub const fn token(self) -> &'static str {
        match self {
            Dir::Left => "l",
            Dir::Right => "r",
            Dir::Up => "u",
            Dir::Down => "d",
            Dir::LeftUp => "lu",
            Dir::RightUp => "ru",
            Dir::LeftDown => "ld",
            Dir::RightDown => "rd",
        }
    }

    pub fn from_token(token: &str) -> Option<Dir> {
        Dir::ALL.into_iter().find(|dir| dir.token() == token)
    }

    /// The direction turned around: what an exit plays its entrance recipe with.
    pub const fn opposite(self) -> Dir {
        match self {
            Dir::Left => Dir::Right,
            Dir::Right => Dir::Left,
            Dir::Up => Dir::Down,
            Dir::Down => Dir::Up,
            Dir::LeftUp => Dir::RightDown,
            Dir::RightUp => Dir::LeftDown,
            Dir::LeftDown => Dir::RightUp,
            Dir::RightDown => Dir::LeftUp,
        }
    }

    /// The motion away from a side or corner the content starts at ("from the top" moves down).
    pub const fn from_side(side: Dir) -> Dir {
        side.opposite()
    }

    pub const fn as_dir4(self) -> Option<Dir4> {
        match self {
            Dir::Left => Some(Dir4::Left),
            Dir::Right => Some(Dir4::Right),
            Dir::Up => Some(Dir4::Up),
            Dir::Down => Some(Dir4::Down),
            _ => None,
        }
    }

    pub const fn as_diagonal(self) -> Option<Diagonal> {
        match self {
            Dir::LeftUp => Some(Diagonal::LeftUp),
            Dir::RightUp => Some(Diagonal::RightUp),
            Dir::LeftDown => Some(Diagonal::LeftDown),
            Dir::RightDown => Some(Diagonal::RightDown),
            _ => None,
        }
    }
}

impl fmt::Display for Dir {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.token())
    }
}

/// One of the four straight directions the content moves (`l r u d`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[ts(export_to = "ir/")]
pub enum Dir4 {
    #[serde(rename = "l")]
    Left,
    #[serde(rename = "r")]
    Right,
    #[serde(rename = "u")]
    Up,
    #[serde(rename = "d")]
    Down,
}

impl Dir4 {
    pub const ALL: [Dir4; 4] = [Dir4::Left, Dir4::Right, Dir4::Up, Dir4::Down];

    pub const fn dir(self) -> Dir {
        match self {
            Dir4::Left => Dir::Left,
            Dir4::Right => Dir::Right,
            Dir4::Up => Dir::Up,
            Dir4::Down => Dir::Down,
        }
    }

    pub const fn opposite(self) -> Dir4 {
        match self {
            Dir4::Left => Dir4::Right,
            Dir4::Right => Dir4::Left,
            Dir4::Up => Dir4::Down,
            Dir4::Down => Dir4::Up,
        }
    }
}

/// One of the four diagonal directions the content moves (`lu ru ld rd`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[ts(export_to = "ir/")]
pub enum Diagonal {
    #[serde(rename = "lu")]
    LeftUp,
    #[serde(rename = "ru")]
    RightUp,
    #[serde(rename = "ld")]
    LeftDown,
    #[serde(rename = "rd")]
    RightDown,
}

impl Diagonal {
    pub const fn dir(self) -> Dir {
        match self {
            Diagonal::LeftUp => Dir::LeftUp,
            Diagonal::RightUp => Dir::RightUp,
            Diagonal::LeftDown => Dir::LeftDown,
            Diagonal::RightDown => Dir::RightDown,
        }
    }
}

/// Up or down: Float In rises (`u`) or descends (`d`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[ts(export_to = "ir/")]
pub enum Vertical {
    #[serde(rename = "u")]
    Up,
    #[serde(rename = "d")]
    Down,
}

/// The orientation of a line or of bars, in OOXML's tokens: `split{orient: horz}` opens along a horizontal line,
/// `blinds{orient: horz}` has horizontal bars.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[ts(export_to = "ir/")]
pub enum Orient {
    #[serde(rename = "horz")]
    Horizontal,
    #[serde(rename = "vert")]
    Vertical,
}

/// The axis a scale or a turn works along: horizontal (`scaleX`, `rotateY`), vertical, or both.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[ts(export_to = "ir/")]
pub enum Axis {
    #[serde(rename = "horz")]
    Horizontal,
    #[serde(rename = "vert")]
    Vertical,
    #[serde(rename = "both")]
    Both,
}

/// Inward (from the edges to the centre) or outward (from the centre to the edges).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "ir/")]
pub enum InOut {
    In,
    Out,
}

impl InOut {
    pub const fn opposite(self) -> InOut {
        match self {
            InOut::In => InOut::Out,
            InOut::Out => InOut::In,
        }
    }
}

/// The shape an iris grows or shrinks as.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "ir/")]
pub enum IrisShape {
    Circle,
    Diamond,
    Box,
    Plus,
}

/// Where a clock sweep starts, as the hour on a clock face.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[ts(export_to = "ir/")]
pub enum ClockStart {
    #[serde(rename = "12")]
    Twelve,
    #[serde(rename = "3")]
    Three,
    #[serde(rename = "6")]
    Six,
    #[serde(rename = "9")]
    Nine,
}

/// What a fade passes through between the two slides.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "ir/")]
pub enum Through {
    /// A crossfade.
    None,
    Black,
    White,
    /// The colour in the effect's `color`.
    Color,
}

/// How far a fly-in travels: from outside the slide, or a short way (25% of the element's box).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "ir/")]
pub enum Distance {
    OffSlide,
    Slight,
}

/// The point a zoom scales about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "ir/")]
pub enum ZoomOrigin {
    Object,
    SlideCenter,
}

/// What a Morph matches between the two slides.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "ir/")]
pub enum MorphBy {
    Object,
    Word,
    Char,
}

/// The colour a colour change animates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "ir/")]
pub enum ColorProperty {
    Fill,
    Text,
    Line,
}

/// An opaque sRGB colour, `#rrggbb` in lowercase.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, TS)]
#[ts(export_to = "ir/", type = "string")]
pub struct HexColor(u32);

/// A string that is not `#` and six hex digits.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("not a #rrggbb colour: {0:?}")]
pub struct InvalidColor(pub String);

impl HexColor {
    pub const BLACK: HexColor = HexColor(0);
    pub const WHITE: HexColor = HexColor(0x00ff_ffff);

    pub const fn rgb(r: u8, g: u8, b: u8) -> HexColor {
        HexColor(((r as u32) << 16) | ((g as u32) << 8) | b as u32)
    }

    /// `#rrggbb`, either case.
    pub fn parse(text: &str) -> Result<HexColor, InvalidColor> {
        let digits = text
            .strip_prefix('#')
            .filter(|digits| digits.len() == 6 && digits.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or_else(|| InvalidColor(text.to_owned()))?;
        u32::from_str_radix(digits, 16)
            .map(HexColor)
            .map_err(|_| InvalidColor(text.to_owned()))
    }

    pub const fn value(self) -> u32 {
        self.0
    }
}

impl fmt::Display for HexColor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:06x}", self.0)
    }
}

impl Serialize for HexColor {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for HexColor {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        match HexColor::parse(&text) {
            Ok(color) if color.to_string() == text => Ok(color),
            _ => Err(serde::de::Error::custom(InvalidColor(text))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directions_print_ooxml_tokens() {
        for dir in Dir::ALL {
            assert_eq!(Dir::from_token(dir.token()), Some(dir));
            assert_eq!(
                serde_json::to_string(&dir).unwrap(),
                format!("\"{}\"", dir.token())
            );
            assert_eq!(dir.opposite().opposite(), dir);
        }
        assert_eq!(Dir::from_side(Dir::Left), Dir::Right);
        assert_eq!(Dir::from_token("left"), None);
        assert!(serde_json::from_str::<Dir4>("\"lu\"").is_err());
        assert_eq!(Dir::Up.as_dir4(), Some(Dir4::Up));
        assert_eq!(Dir::Up.as_diagonal(), None);
    }

    #[test]
    fn colours_are_lowercase_hex() {
        let color = HexColor::parse("#FFaa00").unwrap();
        assert_eq!(color.to_string(), "#ffaa00");
        assert_eq!(serde_json::to_string(&color).unwrap(), "\"#ffaa00\"");
        assert!(serde_json::from_str::<HexColor>("\"#ffaa00\"").is_ok());
        // Only the canonical spelling reads back, so a colour has one wire form.
        assert!(serde_json::from_str::<HexColor>("\"#FFAA00\"").is_err());
        for bad in ["ffaa00", "#ffaa0", "#ffaa000", "#ggaa00", "#+faa00"] {
            assert!(HexColor::parse(bad).is_err(), "{bad}");
        }
        assert_eq!(HexColor::rgb(1, 2, 3).to_string(), "#010203");
    }
}
