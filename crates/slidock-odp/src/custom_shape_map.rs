//! Custom shapes (`draw:custom-shape`) by their `draw:enhanced-geometry draw:type`, as ECMA presets. Shapes imported
//! from OOXML keep their preset as `ooxml-<name>`, with the preset's adjustments in `draw:modifiers` in its own order
//! and units; Microsoft's binary shape types arrive as `mso-spt<n>`; LibreOffice's own shapes have names of their own,
//! whose adjustments use another coordinate system and are dropped. Shapes map by type alone.

use indexmap::IndexMap;
use slidock_canon::js_number;
use slidock_geometry::adjust_names;
use slidock_ir::geometry::PresetName;

use crate::units::is_space;

/// MS-ODRAW's shape types (MSOSPT) that have an ECMA preset.
fn mso_shape_type(n: u32) -> Option<&'static str> {
    Some(match n {
        1 => "rect",
        2 => "roundRect",
        3 => "ellipse",
        4 => "diamond",
        5 => "triangle",
        6 => "rtTriangle",
        7 => "parallelogram",
        9 => "hexagon",
        10 => "octagon",
        11 => "plus",
        12 => "star5",
        13 => "rightArrow",
        15 => "homePlate",
        16 => "cube",
        20 => "line",
        22 => "can",
        23 => "donut",
        32 => "straightConnector1",
        33 => "bentConnector2",
        34 => "bentConnector3",
        35 => "bentConnector4",
        36 => "bentConnector5",
        37 => "curvedConnector2",
        38 => "curvedConnector3",
        39 => "curvedConnector4",
        40 => "curvedConnector5",
        55 => "chevron",
        56 => "pentagon",
        66 => "leftArrow",
        67 => "downArrow",
        68 => "upArrow",
        69 => "leftRightArrow",
        70 => "upDownArrow",
        73 => "lightningBolt",
        74 => "heart",
        96 => "smileyFace",
        183 => "sun",
        184 => "moon",
        _ => return None,
    })
}

/// LibreOffice's own shape names that draw an ECMA preset.
fn libreoffice_shape(name: &str) -> Option<&'static str> {
    Some(match name {
        "rectangle" => "rect",
        "round-rectangle" => "roundRect",
        "ellipse" | "circle" => "ellipse",
        "isosceles-triangle" => "triangle",
        "right-triangle" => "rtTriangle",
        "diamond" => "diamond",
        "parallelogram" => "parallelogram",
        "hexagon" => "hexagon",
        "octagon" => "octagon",
        "pentagon" => "pentagon",
        "cross" => "plus",
        "star4" => "star4",
        "star5" => "star5",
        "star8" => "star8",
        "star12" => "star12",
        "star24" => "star24",
        "right-arrow" => "rightArrow",
        "left-arrow" => "leftArrow",
        "up-arrow" => "upArrow",
        "down-arrow" => "downArrow",
        "left-right-arrow" => "leftRightArrow",
        "up-down-arrow" => "upDownArrow",
        "pentagon-right" => "homePlate",
        "chevron" => "chevron",
        "heart" => "heart",
        "smiley" => "smileyFace",
        "sun" => "sun",
        "moon" => "moon",
        "lightning" => "lightningBolt",
        "can" => "can",
        "cube" => "cube",
        "ring" => "donut",
        "frame" => "frame",
        "paper" => "foldedCorner",
        "flowchart-process" => "flowChartProcess",
        "flowchart-decision" => "flowChartDecision",
        "flowchart-terminator" => "flowChartTerminator",
        "flowchart-connector" => "flowChartConnector",
        "flowchart-document" => "flowChartDocument",
        "flowchart-data" => "flowChartInputOutput",
        _ => return None,
    })
}

/// A custom shape's preset, with the adjustments it carries over.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MappedShape {
    pub preset: PresetName,
    pub adjust: IndexMap<String, f64>,
    pub dropped_modifiers: bool,
}

/// `mso-spt<1 to 3 digits>`.
fn mso_number(kind: &str) -> Option<u32> {
    let digits = kind.strip_prefix("mso-spt")?;
    ((1..=3).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_digit()))
        .then(|| digits.parse().ok())
        .flatten()
}

/// The preset a `draw:type` draws, its `draw:modifiers` named by the preset's adjustments when they share units.
pub(crate) fn map_custom_shape(kind: &str, modifiers: Option<&str>) -> Option<MappedShape> {
    let values: Vec<f64> = slidock_canon::js_trim(modifiers.unwrap_or(""))
        .split(is_space)
        .filter(|token| !token.is_empty())
        .map(js_number)
        .filter(|value| value.is_finite() && value.abs() <= 1e9)
        .collect();
    if let Some(name) = kind.strip_prefix("ooxml-") {
        let preset = PresetName::parse(name)?;
        let adjust = adjust_names(preset)
            .into_iter()
            .zip(&values)
            .map(|(name, value)| (name.to_owned(), *value))
            .collect();
        return Some(MappedShape {
            preset,
            adjust,
            dropped_modifiers: false,
        });
    }
    let name = match mso_number(kind) {
        Some(n) => mso_shape_type(n),
        None if kind.starts_with("mso-spt") => None,
        None => libreoffice_shape(kind),
    }?;
    Some(MappedShape {
        preset: PresetName::parse(name)?,
        adjust: IndexMap::new(),
        dropped_modifiers: !values.is_empty(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_custom_shape_types_to_presets_and_carries_ooxml_adjustments_by_name() {
        let round = map_custom_shape("ooxml-roundRect", Some("30000")).unwrap();
        assert_eq!(round.preset.as_str(), "roundRect");
        assert_eq!(round.adjust.get("adj"), Some(&30000.0));
        assert!(!round.dropped_modifiers);
        let bent = map_custom_shape("ooxml-bentConnector3", Some("25000 1")).unwrap();
        assert_eq!(
            bent.adjust.into_iter().collect::<Vec<_>>(),
            [("adj1".to_owned(), 25000.0)]
        );
        let spt = map_custom_shape("mso-spt32", None).unwrap();
        assert_eq!(spt.preset.as_str(), "straightConnector1");
        let native = map_custom_shape("round-rectangle", Some("3600")).unwrap();
        assert_eq!(native.preset.as_str(), "roundRect");
        assert!(native.dropped_modifiers);
        assert!(map_custom_shape("ooxml-fancyStar", Some("1")).is_none());
        assert!(map_custom_shape("non-primitive", None).is_none());
        assert!(map_custom_shape("mso-spt1000", None).is_none());
        assert_eq!(
            map_custom_shape("mso-spt032", None)
                .unwrap()
                .preset
                .as_str(),
            "straightConnector1"
        );
    }

    /// Every `mso-spt0` to `mso-spt999`, every LibreOffice shape name, every `ooxml-<preset>` and a few near misses,
    /// as the reference table maps them with `draw:modifiers="1 2 … 9"`.
    #[test]
    fn maps_every_shape_type_as_the_reference_table() {
        let vector = include_str!("../tests/vectors/custom-shapes.tsv");
        let mut checked = 0;
        for line in vector.lines().filter(|line| !line.starts_with('#')) {
            let kind = line.split('\t').next().unwrap();
            let actual = match map_custom_shape(kind, Some("1 2 3 4 5 6 7 8 9")) {
                None => format!("{kind}\t-"),
                Some(shape) => {
                    let adjust: Vec<String> = shape
                        .adjust
                        .iter()
                        .map(|(name, value)| format!("{name}={value}"))
                        .collect();
                    let adjust = if adjust.is_empty() {
                        "-".to_owned()
                    } else {
                        adjust.join(",")
                    };
                    format!(
                        "{kind}\t{}\t{adjust}\t{}",
                        shape.preset.as_str(),
                        shape.dropped_modifiers
                    )
                }
            };
            assert_eq!(actual, line);
            checked += 1;
        }
        assert_eq!(checked, 1241);
    }
}
