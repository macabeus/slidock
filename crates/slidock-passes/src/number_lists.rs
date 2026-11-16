//! number-lists: every bullet becomes a label with its own text, font, size and colour. Character bullets keep their
//! character; auto-numbered ones count with PowerPoint's rules, per text body and level: a counter continues while
//! consecutive paragraphs at that level share scheme and start; a shallower paragraph resets deeper counters; a
//! different scheme or an unnumbered paragraph at that level restarts. A paragraph that shows no text leaves every
//! counter as it was. A label takes what it leaves unstated from the paragraph's first run.

use std::collections::BTreeMap;

use slidock_core_api::{ContainerView, OwnedContainer, PassCtx, PassError, PassInfo, SlidePass};
use slidock_ir::color::Color;
use slidock_ir::ids::NodeId;
use slidock_ir::text::{
    AutoNumScheme, AutoNumbering, Bullet, BulletSize, FontRef, Inline, OrText, SourceParagraph,
    TextBody,
};
use slidock_ir::units::Size;
use slidock_ir::verify::{CascadedRun, Invariant, cascaded_para, cascaded_run, family_font};

use crate::container::{try_map_text_bodies, unit_error, with_mapped_elements};

pub struct NumberLists;

static INFO: PassInfo = PassInfo::new(
    "number-lists",
    1,
    &[Invariant::TextCascaded],
    &[Invariant::ListsLabelled],
);

impl SlidePass for NumberLists {
    fn info(&self) -> &PassInfo {
        &INFO
    }

    fn run(
        &self,
        view: &ContainerView<'_>,
        _ctx: &mut PassCtx<'_>,
    ) -> Result<OwnedContainer, PassError> {
        with_mapped_elements(view.container, |element| {
            try_map_text_bodies(element, |text, _| label_body(text, element.id()))
        })
    }
}

/// How a scheme writes its digits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Digits {
    Arabic,
    AlphaLc,
    AlphaUc,
    RomanLc,
    RomanUc,
}

/// What surrounds the digits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Form {
    Period,
    ParenR,
    ParenBoth,
    Plain,
}

fn scheme_parts(scheme: AutoNumScheme) -> (Digits, Form) {
    use AutoNumScheme::*;
    match scheme {
        ArabicPeriod => (Digits::Arabic, Form::Period),
        ArabicParenR => (Digits::Arabic, Form::ParenR),
        ArabicParenBoth => (Digits::Arabic, Form::ParenBoth),
        ArabicPlain => (Digits::Arabic, Form::Plain),
        AlphaLcPeriod => (Digits::AlphaLc, Form::Period),
        AlphaUcPeriod => (Digits::AlphaUc, Form::Period),
        AlphaLcParenR => (Digits::AlphaLc, Form::ParenR),
        AlphaUcParenR => (Digits::AlphaUc, Form::ParenR),
        AlphaLcParenBoth => (Digits::AlphaLc, Form::ParenBoth),
        AlphaUcParenBoth => (Digits::AlphaUc, Form::ParenBoth),
        RomanLcPeriod => (Digits::RomanLc, Form::Period),
        RomanUcPeriod => (Digits::RomanUc, Form::Period),
        RomanLcParenR => (Digits::RomanLc, Form::ParenR),
        RomanUcParenR => (Digits::RomanUc, Form::ParenR),
        RomanLcParenBoth => (Digits::RomanLc, Form::ParenBoth),
        RomanUcParenBoth => (Digits::RomanUc, Form::ParenBoth),
    }
}

const ROMAN: [(u64, &str); 13] = [
    (1000, "m"),
    (900, "cm"),
    (500, "d"),
    (400, "cd"),
    (100, "c"),
    (90, "xc"),
    (50, "l"),
    (40, "xl"),
    (10, "x"),
    (9, "ix"),
    (5, "v"),
    (4, "iv"),
    (1, "i"),
];

fn roman(n: u64) -> String {
    let mut rest = n;
    let mut out = String::new();
    for (value, digits) in ROMAN {
        while rest >= value {
            out.push_str(digits);
            rest -= value;
        }
    }
    out
}

/// a…z, then aa…zz, aaa…: the letter repeats, as PowerPoint numbers past z.
fn alpha(n: u64) -> String {
    let index = (n - 1) % 26;
    let letter = char::from(b'a' + index as u8);
    let times = (n - 1) / 26 + 1;
    std::iter::repeat_n(letter, times as usize).collect()
}

/// The label of number `n` in a scheme: `arabicPeriod` 3 → `3.`, `alphaUcParenBoth` 2 → `(B)`.
pub fn number_label(scheme: AutoNumScheme, n: u64) -> String {
    let (digits, form) = scheme_parts(scheme);
    let value = match digits {
        Digits::Arabic => n.to_string(),
        Digits::AlphaLc | Digits::AlphaUc => alpha(n.max(1)),
        Digits::RomanLc | Digits::RomanUc => roman(n.max(1)),
    };
    let text = match digits {
        Digits::AlphaUc | Digits::RomanUc => value.to_ascii_uppercase(),
        _ => value,
    };
    match form {
        Form::ParenR => format!("{text})"),
        Form::ParenBoth => format!("({text})"),
        Form::Plain => text,
        Form::Period => format!("{text}."),
    }
}

/// What numbers a bullet, whether written as an auto bullet or as a label this pass made on an earlier run.
fn numbering_of(bullet: &Bullet) -> Option<AutoNumbering> {
    match bullet {
        Bullet::Auto {
            scheme, start_at, ..
        } => Some(AutoNumbering {
            scheme: *scheme,
            start_at: *start_at,
        }),
        Bullet::Label { auto, .. } => auto.clone(),
        _ => None,
    }
}

/// The run a label follows: the paragraph's first run or field, else its paragraph mark.
fn first_run(paragraph: &SourceParagraph, node: &NodeId) -> Result<CascadedRun, PassError> {
    let props = paragraph.inlines.iter().find_map(|inline| match inline {
        Inline::Run { props, .. } | Inline::Field { props, .. } => Some(props),
        _ => None,
    });
    Ok(cascaded_run(props.or(paragraph.end_props.as_ref()), node)?)
}

fn label(
    bullet: &Bullet,
    text: String,
    run: &CascadedRun,
    node: &NodeId,
) -> Result<Bullet, PassError> {
    let (font, color, size, auto) = match bullet {
        Bullet::None {} => return Ok(Bullet::None {}),
        Bullet::Label { .. } => {
            let mut out = bullet.clone();
            if let Bullet::Label { text: own, .. } = &mut out {
                *own = text;
            }
            return Ok(out);
        }
        Bullet::Char {
            font, color, size, ..
        } => (font, color, size, None),
        Bullet::Auto {
            font,
            color,
            size,
            scheme,
            start_at,
        } => (
            font,
            color,
            size,
            Some(AutoNumbering {
                scheme: *scheme,
                start_at: *start_at,
            }),
        ),
    };
    let font: &FontRef = match font {
        Some(OrText::Value(font)) => font,
        _ => &run.font,
    };
    let color: &Color = match color {
        Some(OrText::Value(color)) => color,
        _ => &run.color,
    };
    let size = match size {
        Some(BulletSize::Pt { value }) => *value,
        Some(BulletSize::Pct { value }) => Size::quantise(value.to_f64() * run.size.to_f64())
            .map_err(|e| unit_error("A bullet size", e))?,
        _ => run.size,
    };
    Ok(Bullet::Label {
        text,
        font: family_font(font, Some(node))?.clone(),
        size,
        color: color.clone(),
        auto,
    })
}

/// An open counter: its numbering and the last number it gave.
struct Counter {
    numbering: AutoNumbering,
    n: u64,
}

fn label_body(text: &TextBody, node: &NodeId) -> Result<TextBody, PassError> {
    let mut counters: BTreeMap<u32, Counter> = BTreeMap::new();
    let mut out = text.clone();
    for paragraph in &mut out.paragraphs {
        let props = cascaded_para(&paragraph.props, node)?;
        let level = paragraph.level;
        // A paragraph that shows no text takes the label it would get, and leaves every counter as it was.
        let counts = !paragraph.shows_no_text();
        if counts {
            counters.retain(|&deeper, _| deeper <= level);
        }
        let mut content = match &props.bullet {
            Bullet::Char { char, .. } => char.clone(),
            Bullet::Label { text, .. } => text.clone(),
            _ => String::new(),
        };
        if let Some(numbering) = numbering_of(&props.bullet) {
            let n = match counters.get(&level) {
                Some(open) if open.numbering == numbering => open.n + 1,
                _ => u64::from(numbering.start_at),
            };
            content = number_label(numbering.scheme, n);
            if counts {
                counters.insert(level, Counter { numbering, n });
            }
        } else if counts {
            counters.remove(&level);
        }
        let bullet = label(&props.bullet, content, &first_run(paragraph, node)?, node)?;
        paragraph.props.bullet = Some(bullet);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_alphabetic_and_roman_numbers() {
        assert_eq!(alpha(1), "a");
        assert_eq!(alpha(26), "z");
        assert_eq!(alpha(27), "aa");
        assert_eq!(alpha(53), "aaa");
        assert_eq!(roman(1994), "mcmxciv");
        assert_eq!(roman(4), "iv");
        // Zero numbers as one in the letter and roman schemes, and as 0 in arabic.
        assert_eq!(number_label(AutoNumScheme::RomanLcPeriod, 0), "i.");
        assert_eq!(number_label(AutoNumScheme::ArabicPlain, 0), "0");
    }
}
