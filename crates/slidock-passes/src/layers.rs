//! Layering of declared properties: a later layer's present fields win, field by field, and absent fields leave
//! earlier values (`Object.assign` over the layers). Complete defaults enter the layers as subsets with every field.

use slidock_ir::text::{
    Autofit, BodyProps, LevelStyle, LevelStyles, ParaProps, PartialBodyProps, PartialInsets,
    PartialParaProps, PartialRunProps, RunProps,
};

/// A declared subset that a later layer overrides field by field.
pub(crate) trait Layer: Clone + Default {
    /// Copies every field `over` states onto `self`.
    fn overlay(&mut self, over: &Self);

    /// The layers merged, later ones winning.
    fn layered<'a>(layers: impl IntoIterator<Item = Option<&'a Self>>) -> Self
    where
        Self: 'a,
    {
        let mut out = Self::default();
        for layer in layers.into_iter().flatten() {
            out.overlay(layer);
        }
        out
    }
}

macro_rules! overlay_fields {
    ($ty:ty { $($field:ident),* $(,)? }) => {
        impl Layer for $ty {
            fn overlay(&mut self, over: &Self) {
                $(
                    if over.$field.is_some() {
                        self.$field.clone_from(&over.$field);
                    }
                )*
            }
        }
    };
}

overlay_fields!(PartialRunProps {
    font,
    size,
    weight,
    italic,
    underline,
    strike,
    color,
    highlight,
    baseline,
    caps,
    letter_spacing,
    link,
    synthesize_bold,
    script_scale,
    shadow,
});

overlay_fields!(PartialParaProps {
    align,
    margin_left,
    margin_right,
    indent,
    space_before,
    space_after,
    line_spacing,
    tab_size,
    bullet,
    rtl,
    line_height,
    space_before_pt,
    space_after_pt,
    leading_skew,
    line_tail_pt,
});

overlay_fields!(PartialInsets { l, t, r, b });

/// Body layers overlay field by field, and each side of the insets inherits on its own: a layer's insets add their
/// sides to those below.
impl Layer for PartialBodyProps {
    fn overlay(&mut self, over: &Self) {
        let insets = match (&self.insets, &over.insets) {
            (Some(below), Some(above)) => {
                let mut merged = below.clone();
                merged.overlay(above);
                Some(merged)
            }
            (below, above) => above.clone().or_else(|| below.clone()),
        };
        self.insets = insets;
        if over.anchor.is_some() {
            self.anchor = over.anchor;
        }
        if over.wrap.is_some() {
            self.wrap = over.wrap;
        }
        if over.autofit.is_some() {
            self.autofit.clone_from(&over.autofit);
        }
        if over.overflow.is_some() {
            self.overflow = over.overflow;
        }
        if over.max_width.is_some() {
            self.max_width = over.max_width;
        }
    }
}

/// Complete run properties as a subset with every field.
pub(crate) fn partial_run(props: &RunProps) -> PartialRunProps {
    PartialRunProps {
        font: Some(props.font.clone()),
        size: Some(props.size),
        weight: Some(props.weight),
        italic: Some(props.italic),
        underline: Some(props.underline),
        strike: Some(props.strike),
        color: Some(props.color.clone()),
        highlight: Some(props.highlight.clone()),
        baseline: Some(props.baseline),
        caps: Some(props.caps),
        letter_spacing: Some(props.letter_spacing),
        link: Some(props.link.clone()),
        synthesize_bold: Some(props.synthesize_bold),
        script_scale: Some(props.script_scale),
        // A default without a shadow states none, so the layers above it decide.
        shadow: props.shadow.clone().map(Some),
    }
}

/// Complete paragraph properties as a subset with every field they hold.
pub(crate) fn partial_para(props: &ParaProps) -> PartialParaProps {
    PartialParaProps {
        align: Some(props.align),
        margin_left: Some(props.margin_left),
        margin_right: Some(props.margin_right),
        indent: Some(props.indent),
        space_before: Some(props.space_before.clone()),
        space_after: Some(props.space_after.clone()),
        line_spacing: Some(props.line_spacing.clone()),
        tab_size: Some(props.tab_size),
        bullet: Some(props.bullet.clone()),
        rtl: Some(props.rtl),
        line_height: props.line_height.clone(),
        space_before_pt: props.space_before_pt,
        space_after_pt: props.space_after_pt,
        leading_skew: props.leading_skew,
        line_tail_pt: props.line_tail_pt,
    }
}

/// Complete body properties as a subset with every field they hold.
pub(crate) fn partial_body(body: &BodyProps<Autofit>) -> PartialBodyProps {
    PartialBodyProps {
        insets: Some(PartialInsets {
            l: Some(body.insets.l),
            t: Some(body.insets.t),
            r: Some(body.insets.r),
            b: Some(body.insets.b),
        }),
        anchor: Some(body.anchor),
        wrap: Some(body.wrap),
        autofit: Some(body.autofit.clone()),
        overflow: Some(body.overflow),
        max_width: body.max_width,
    }
}

/// List styles merged per level, para and run each field by field, later layers winning. A level any layer has
/// is kept, even with nothing in it.
pub(crate) fn merge_list_styles<'a>(
    layers: impl IntoIterator<Item = Option<&'a LevelStyles>>,
) -> Option<LevelStyles> {
    let present: Vec<&LevelStyles> = layers.into_iter().flatten().collect();
    if present.is_empty() {
        return None;
    }
    let mut out = LevelStyles::default();
    for (index, slot) in out.slots_mut().into_iter().enumerate() {
        let styles: Vec<&LevelStyle> = present
            .iter()
            .filter_map(|layer| layer.get(index))
            .collect();
        if styles.is_empty() {
            continue;
        }
        let paras: Vec<&PartialParaProps> = styles.iter().filter_map(|s| s.para.as_ref()).collect();
        let runs: Vec<&PartialRunProps> = styles.iter().filter_map(|s| s.run.as_ref()).collect();
        *slot = Some(Box::new(LevelStyle {
            para: (!paras.is_empty()).then(|| Layer::layered(paras.into_iter().map(Some))),
            run: (!runs.is_empty()).then(|| Layer::layered(runs.into_iter().map(Some))),
        }));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use slidock_ir::text::{Align, Anchor};
    use slidock_ir::units::Pt;

    fn pt(x: f64) -> Pt {
        Pt::exact(x).unwrap()
    }

    #[test]
    fn later_layers_win_field_by_field() {
        let low = PartialParaProps {
            align: Some(Align::Left),
            indent: Some(pt(4.0)),
            ..Default::default()
        };
        let high = PartialParaProps {
            align: Some(Align::Right),
            ..Default::default()
        };
        let out = PartialParaProps::layered([Some(&low), None, Some(&high)]);
        assert_eq!(out.align, Some(Align::Right));
        assert_eq!(out.indent, Some(pt(4.0)));
    }

    #[test]
    fn an_explicit_null_overrides_and_an_absent_value_does_not() {
        let low = PartialRunProps {
            highlight: Some(Some(slidock_ir::color::Color::Rgb(
                slidock_ir::color::RgbColor::new(
                    slidock_ir::color::ColorSpace::Srgb,
                    Default::default(),
                    Default::default(),
                    Default::default(),
                    Default::default(),
                ),
            ))),
            ..Default::default()
        };
        let null = PartialRunProps {
            highlight: Some(None),
            ..Default::default()
        };
        assert_eq!(
            PartialRunProps::layered([Some(&low), Some(&null)]).highlight,
            Some(None)
        );
        assert!(
            PartialRunProps::layered([Some(&low), Some(&PartialRunProps::default())])
                .highlight
                .is_some_and(|h| h.is_some())
        );
    }

    #[test]
    fn inset_sides_inherit_one_by_one() {
        let low = PartialBodyProps {
            insets: Some(PartialInsets {
                l: Some(pt(1.0)),
                t: Some(pt(2.0)),
                ..Default::default()
            }),
            anchor: Some(Anchor::Top),
            ..Default::default()
        };
        let high = PartialBodyProps {
            insets: Some(PartialInsets {
                t: Some(pt(9.0)),
                ..Default::default()
            }),
            ..Default::default()
        };
        let out = PartialBodyProps::layered([Some(&low), Some(&high)]);
        let insets = out.insets.unwrap();
        assert_eq!(
            (insets.l, insets.t, insets.r),
            (Some(pt(1.0)), Some(pt(9.0)), None)
        );
        assert_eq!(out.anchor, Some(Anchor::Top));
        let empty = PartialBodyProps {
            insets: Some(PartialInsets::default()),
            ..Default::default()
        };
        assert_eq!(
            PartialBodyProps::layered([Some(&empty)]).insets,
            Some(PartialInsets::default())
        );
    }

    #[test]
    fn list_styles_merge_per_level_and_keep_empty_levels() {
        let a = LevelStyles {
            l0: Some(Box::new(LevelStyle {
                para: Some(PartialParaProps {
                    align: Some(Align::Right),
                    ..Default::default()
                }),
                run: None,
            })),
            l2: Some(Box::default()),
            ..Default::default()
        };
        let b = LevelStyles {
            l0: Some(Box::new(LevelStyle {
                para: Some(PartialParaProps {
                    indent: Some(pt(3.0)),
                    ..Default::default()
                }),
                run: None,
            })),
            ..Default::default()
        };
        let out = merge_list_styles([Some(&a), None, Some(&b)]).unwrap();
        let level = out.l0.unwrap().para.unwrap();
        assert_eq!(
            (level.align, level.indent),
            (Some(Align::Right), Some(pt(3.0)))
        );
        assert_eq!(out.l2, Some(Box::default()));
        assert_eq!(out.l1, None);
        assert_eq!(merge_list_styles([None, None]), None);
    }
}
