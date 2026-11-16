//! Lists: a `text:list-style`'s level styles as bullets and indents. A bullet level shows its character in its own
//! font, colour and size; a number level its scheme (format, prefix and suffix as one of the IR's schemes) from its
//! start value; an image level a round bullet (OD9209). Indents come in either of ODF's two models: the label's
//! position and minimum width (`text:space-before`, `text:min-label-width`, the default), or the label alignment's
//! margin and indent.

use slidock_canon::utf16;
use slidock_ir::color::Color;
use slidock_ir::diagnostics::{DiagnosticCode, DiagnosticSite, codes};
use slidock_ir::text::{AutoNumScheme, Bullet, BulletSize, FamilyFont, FontRef, OrText};

use crate::names::{QName, qn};
use crate::odf::{Element, Ns};
use crate::styles::{CommonStyles, unquote};
use crate::units::{hex_color, length_pt, percent, ratio_of, size, to_pt};

/// What a list level gives a paragraph: its bullet, and where its label and its text start.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ListLevel {
    pub bullet: Bullet,
    /// The label's start and the text's start, from the paragraph's own left margin.
    pub label: f64,
    pub text: f64,
}

/// The level style for a 0-based level: the one written for it, else the deepest written above it.
pub(crate) fn level_style(list: Option<&Element>, level: u32) -> Option<&Element> {
    let wanted = i64::from(level) + 1;
    let mut best = None;
    let mut best_level = 0;
    for element in list.into_iter().flat_map(Element::kids) {
        let written = element.int_attr(qn!("text:level")).unwrap_or(0);
        if written == wanted {
            return Some(element);
        }
        if written <= wanted && written > best_level {
            best = Some(element);
            best_level = written;
        }
    }
    best
}

/// `style:num-format` with its prefix and suffix as an IR scheme; `None` when no scheme matches exactly.
pub(crate) fn number_scheme(format: &str, prefix: &str, suffix: &str) -> Option<AutoNumScheme> {
    use AutoNumScheme::*;
    let [period, paren_r, paren_both] = match format {
        "1" => [ArabicPeriod, ArabicParenR, ArabicParenBoth],
        "a" => [AlphaLcPeriod, AlphaLcParenR, AlphaLcParenBoth],
        "A" => [AlphaUcPeriod, AlphaUcParenR, AlphaUcParenBoth],
        "i" => [RomanLcPeriod, RomanLcParenR, RomanLcParenBoth],
        "I" => [RomanUcPeriod, RomanUcParenR, RomanUcParenBoth],
        _ => return None,
    };
    match (prefix, suffix) {
        ("", ".") => Some(period),
        ("", ")") => Some(paren_r),
        ("(", ")") => Some(paren_both),
        ("", "") if format == "1" => Some(ArabicPlain),
        _ => None,
    }
}

struct LabelProps {
    font: OrText<FontRef>,
    color: OrText<Color>,
    size: BulletSize,
}

/// A level's label properties from its text properties: font, colour (or the text's) and size (or the text's).
fn label_props(level: &Element, common: &CommonStyles<'_>) -> LabelProps {
    let text = level.child(Ns::Style, "text-properties");
    let attr = |name: QName| text.and_then(|t| t.attr(name));
    let family = match (
        attr(qn!("style:font-name")).filter(|s| !s.is_empty()),
        attr(qn!("fo:font-family")).filter(|s| !s.is_empty()),
    ) {
        (Some(name), _) => Some(common.font_family(name)),
        (None, Some(family)) => Some(unquote(family)),
        (None, None) => None,
    };
    let font = match family.filter(|f| !f.is_empty()) {
        Some(family) => OrText::Value(FontRef::Family(FamilyFont::new(
            utf16::clip(&family, 256).to_owned(),
        ))),
        None => OrText::Text,
    };
    let written = hex_color(attr(qn!("fo:color")));
    let color = match written {
        Some(color) if attr(qn!("style:use-window-font-color")) != Some("true") => {
            OrText::Value(Color::Rgb(color))
        }
        _ => OrText::Text,
    };
    let relative = percent(
        level
            .attr(qn!("text:bullet-relative-size"))
            .or_else(|| attr(qn!("fo:font-size"))),
    );
    let absolute = length_pt(attr(qn!("fo:font-size")));
    let size = match (relative, absolute) {
        (Some(relative), _) => BulletSize::Pct {
            value: ratio_of(relative.max(0.0)),
        },
        (None, Some(absolute)) => BulletSize::Pt {
            value: size(absolute.max(0.0)),
        },
        (None, None) => BulletSize::Text {},
    };
    LabelProps { font, color, size }
}

/// The label and text starts a level's properties give, in either positioning model.
fn indents(level: &Element) -> (f64, f64) {
    let props = level.child(Ns::Style, "list-level-properties");
    let alignment = props.and_then(|p| p.child(Ns::Style, "list-level-label-alignment"));
    let mode = props.and_then(|p| p.attr(qn!("text:list-level-position-and-space-mode")));
    if let (Some("label-alignment"), Some(alignment)) = (mode, alignment) {
        let margin = length_pt(alignment.attr(qn!("fo:margin-left"))).unwrap_or(0.0);
        let indent = length_pt(alignment.attr(qn!("fo:text-indent"))).unwrap_or(0.0);
        return (to_pt(margin + indent), margin);
    }
    let attr = |name: QName| props.and_then(|p| p.attr(name));
    let before = length_pt(attr(qn!("text:space-before"))).unwrap_or(0.0);
    let width = length_pt(attr(qn!("text:min-label-width"))).unwrap_or(0.0);
    (before, to_pt(before + width))
}

/// What a list style's level gives its paragraphs; an unstyled level gets the `none` bullet.
pub(crate) fn list_level(
    list: Option<&Element>,
    level: u32,
    start_at: Option<i64>,
    common: &CommonStyles<'_>,
    report: &dyn Fn(DiagnosticCode, DiagnosticSite),
) -> ListLevel {
    let Some(style) = level_style(list, level) else {
        return ListLevel {
            bullet: Bullet::None {},
            label: 0.0,
            text: 0.0,
        };
    };
    let (label, text) = indents(style);
    let props = label_props(style, common);
    let bullet = match style.local.as_str() {
        "list-level-style-bullet" => {
            let written = style
                .attr(qn!("text:bullet-char"))
                .filter(|s| !s.is_empty())
                .unwrap_or("•");
            Bullet::Char {
                char: written.chars().take(1).collect(),
                font: Some(props.font),
                color: Some(props.color),
                size: Some(props.size),
            }
        }
        "list-level-style-number" => {
            let format = style.attr(qn!("style:num-format")).unwrap_or("");
            if format.is_empty() {
                Bullet::None {}
            } else {
                let prefix = style.attr(qn!("style:num-prefix")).unwrap_or("");
                let suffix = style.attr(qn!("style:num-suffix")).unwrap_or("");
                let exact = number_scheme(format, prefix, suffix);
                if exact.is_none() {
                    let written = format!("{prefix}{format}{suffix}");
                    report(
                        codes::OD9203,
                        DiagnosticSite::new().data("format", utf16::clip(&written, 32).to_owned()),
                    );
                }
                let scheme = exact
                    .or_else(|| number_scheme(format, "", "."))
                    .unwrap_or(AutoNumScheme::ArabicPeriod);
                let start = start_at
                    .or_else(|| style.int_attr(qn!("text:start-value")))
                    .unwrap_or(1)
                    .clamp(1, 32_767);
                Bullet::Auto {
                    scheme,
                    start_at: u32::try_from(start).unwrap_or(1),
                    font: Some(props.font),
                    color: Some(props.color),
                    size: Some(props.size),
                }
            }
        }
        "list-level-style-image" => {
            report(codes::OD9209, DiagnosticSite::new());
            Bullet::Char {
                char: "•".to_owned(),
                font: Some(props.font),
                color: Some(props.color),
                size: Some(props.size),
            }
        }
        _ => Bullet::None {},
    };
    ListLevel {
        bullet,
        label,
        text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_numbering_formats_to_schemes() {
        assert_eq!(
            number_scheme("1", "", "."),
            Some(AutoNumScheme::ArabicPeriod)
        );
        assert_eq!(
            number_scheme("a", "", ")"),
            Some(AutoNumScheme::AlphaLcParenR)
        );
        assert_eq!(
            number_scheme("I", "(", ")"),
            Some(AutoNumScheme::RomanUcParenBoth)
        );
        assert_eq!(number_scheme("1", "", ""), Some(AutoNumScheme::ArabicPlain));
        assert_eq!(number_scheme("a", "", ""), None);
        assert_eq!(number_scheme("α", "", "."), None);
    }
}
