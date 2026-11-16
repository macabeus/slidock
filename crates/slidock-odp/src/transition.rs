//! A page's transition: the drawing-page style's SMIL filter (`smil:type`, `smil:subtype`,
//! `smil:direction`, `smil:fadeColor`) or an ODF 1.0 `presentation:transition-style`, the duration (the timing root's
//! `anim:transitionFilter smil:dur`, exact, else `presentation:transition-speed`) and the advance
//! (`presentation:transition-type`, `presentation:duration`). A transition style plays the filter LibreOffice migrates
//! it to. The filter maps through LibreOffice's transition table ([`slidock_presets::odp::transition`]); `dir` is the
//! direction the content moves (`pushWipe fromTop` moves down). A transition with no entry plays a dissolve and is
//! reported (OD9402); one the vocabulary only approximates is noted (OD9413).

use slidock_ir::diagnostics::{DiagnosticSite, codes};
use slidock_ir::effects::{NativeId, NativeTransition, TransitionEffect as Effect};
use slidock_ir::features::FeatureId;
use slidock_ir::formats::FileFormat;
use slidock_ir::timeline::{Advance, Transition, TransitionEffect, TransitionFallback};
use slidock_ir::units::Ms;
use slidock_presets::Unknown;

use crate::context::Lowering;
use crate::names::qn;
use crate::odf::{Element, Ns};
use crate::styles::{PropertySet, get};
use crate::timing::value::{self, Read};

/// The duration of each `presentation:transition-speed`, as LibreOffice plays it.
fn speed_ms(speed: &str) -> Option<u32> {
    match speed {
        "slow" => Some(2000),
        "medium" => Some(1000),
        "fast" => Some(500),
        _ => None,
    }
}

/// The speed a page that names none plays at.
const DEFAULT_SPEED: &str = "medium";

/// How a page advances (`presentation:transition-type`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AdvanceMode {
    Manual,
    /// The page advances on its own, after `presentation:duration` (at once without one).
    Automatic,
    /// The effects play on their own; the page still advances on a click.
    SemiAutomatic,
}

impl AdvanceMode {
    fn parse(token: &str) -> Option<AdvanceMode> {
        match token {
            "manual" => Some(AdvanceMode::Manual),
            "automatic" => Some(AdvanceMode::Automatic),
            "semi-automatic" => Some(AdvanceMode::SemiAutomatic),
            _ => None,
        }
    }
}

/// The SMIL filter of the page's transition node in its timing root: an `anim:par` holding an
/// `anim:transitionFilter` aimed at no shape (`smil:begin="<page>.begin"`).
pub(crate) fn timing_filter(page: &Element) -> Option<&Element> {
    let root = timing_root(page)?;
    root.children_of(Ns::Anim, "par").find_map(|par| {
        let filter = par.child(Ns::Anim, "transitionFilter")?;
        (filter.attr(qn!("smil:targetElement")).is_none()
            && par.attr(qn!("presentation:node-type")).is_none())
        .then_some(filter)
    })
}

/// A SMIL sub-type as the transition table reads it: `default` is the type's own default, as no sub-type is.
pub(crate) fn table_subtype(subtype: Option<&str>) -> Option<&str> {
    subtype.filter(|subtype| *subtype != "default")
}

/// The page's timing root: its `anim:par` of node type `timing-root`.
pub(crate) fn timing_root(page: &Element) -> Option<&Element> {
    page.children_of(Ns::Anim, "par")
        .find(|par| par.attr(qn!("presentation:node-type")) == Some("timing-root"))
}

/// Reports an option value read as its default, at a site naming the effect.
pub(crate) fn report_unknown(ctx: &Lowering<'_, '_>, site: DiagnosticSite, unknown: &Unknown) {
    ctx.report(
        codes::OD9412,
        site.data("option", unknown.option)
            .data("value", unknown.value.clone())
            .data("used", unknown.used),
    );
}

/// Reports a timing value that is not as written.
pub(crate) fn report_timing(
    ctx: &Lowering<'_, '_>,
    site: DiagnosticSite,
    what: &str,
    reason: String,
) {
    ctx.report(
        codes::OD9411,
        site.feature(FeatureId::Animation)
            .data("what", what.to_owned())
            .data("reason", reason),
    );
}

/// Why a timing value reads as it does.
pub(crate) fn timing_reason<T>(read: Read<T>, written: &str, used: &str) -> String {
    match read {
        Read::Clamped(_) => format!("{written} is past the limit; it reads as {used}"),
        _ => format!("{written} is not a valid time; it reads as {used}"),
    }
}

/// The page's transition, or `None` when it has neither an effect nor an automatic advance.
pub(crate) fn read(
    page: &Element,
    props: &PropertySet,
    ctx: &Lowering<'_, '_>,
) -> Option<Transition> {
    let filter = timing_filter(page);
    let from_filter = |name| filter.and_then(|filter| filter.attr(name));
    let smil_type = get(props, qn!("smil:type")).or_else(|| from_filter(qn!("smil:type")));
    let style = get(props, qn!("presentation:transition-style")).filter(|style| *style != "none");
    let mode_token = get(props, qn!("presentation:transition-type"));
    let mode = mode_token.map(|token| (token, AdvanceMode::parse(token)));
    let automatic = matches!(mode, Some((_, Some(AdvanceMode::Automatic))));
    if smil_type.is_none() && style.is_none() && !automatic {
        return None;
    }

    let (subtype, direction, fade_color) = if get(props, qn!("smil:type")).is_some() {
        (
            get(props, qn!("smil:subtype")),
            get(props, qn!("smil:direction")),
            get(props, qn!("smil:fadeColor")),
        )
    } else {
        (
            from_filter(qn!("smil:subtype")),
            from_filter(qn!("smil:direction")),
            from_filter(qn!("smil:fadeColor")),
        )
    };
    let mut unknown = Vec::new();
    let reverse = match direction {
        None | Some("forward") => false,
        Some("reverse") => true,
        Some(other) => {
            unknown.push(Unknown {
                option: "smil:direction",
                value: other.to_owned(),
                used: "forward",
            });
            false
        }
    };
    let format = FileFormat::Odp.into();
    let smil = match (smil_type, style) {
        (Some(smil_type), _) => Some((smil_type, table_subtype(subtype), reverse, fade_color)),
        // An ODF 1.0 transition style plays the preset LibreOffice migrates it to.
        (None, Some(style)) => slidock_presets::odp::legacy_transition(style).map(|preset| {
            (
                preset.smil_type,
                preset.subtype,
                preset.reverse,
                preset.fade_color,
            )
        }),
        (None, None) => None,
    };
    let native = match (smil_type, style) {
        (Some(smil_type), _) => slidock_presets::odp::transition_id(smil_type, subtype, reverse),
        (None, Some(style)) => NativeId::new(format, &format!("legacy/{style}")),
        (None, None) => NativeId::new(format, "none"),
    };
    let (effect, approximate) = match smil {
        Some((smil_type, subtype, reverse, fade_color)) => {
            match slidock_presets::odp::transition(smil_type, subtype, reverse, fade_color) {
                Some(lookup) => {
                    unknown.extend(lookup.unknown);
                    (lookup.row.effect, lookup.row.approximate)
                }
                None => (None, false),
            }
        }
        None if style.is_some() => (None, false),
        None => (Some(Effect::None), false),
    };
    let site = || DiagnosticSite::new().data("effect", native.to_string());
    for unknown in &unknown {
        report_unknown(ctx, site(), unknown);
    }
    match effect {
        None => ctx.report(
            codes::OD9402,
            DiagnosticSite::new()
                .feature(FeatureId::SlideTransition)
                .data("effect", native.to_string()),
        ),
        Some(effect) if approximate => ctx.report(
            codes::OD9413,
            site().data(
                "reason",
                format!("{} only approximates the transition", effect.name()),
            ),
        ),
        Some(_) => {}
    }

    let speed = get(props, qn!("presentation:transition-speed"));
    let speed_default = match speed {
        None => speed_ms(DEFAULT_SPEED).unwrap_or(1000),
        Some(speed) => speed_ms(speed).unwrap_or_else(|| {
            report_unknown(
                ctx,
                site(),
                &Unknown {
                    option: "presentation:transition-speed",
                    value: speed.to_owned(),
                    used: DEFAULT_SPEED,
                },
            );
            speed_ms(DEFAULT_SPEED).unwrap_or(1000)
        }),
    };
    let duration = match filter.and_then(|filter| filter.attr(qn!("smil:dur"))) {
        None => Ms(speed_default),
        Some(written) => {
            let read = value::clock(written);
            if read.approximated() {
                let used = read.value().unwrap_or(Ms(speed_default));
                report_timing(
                    ctx,
                    site(),
                    "transition duration",
                    timing_reason(read, written, &format!("{} ms", used.0)),
                );
            }
            read.value().unwrap_or(Ms(speed_default))
        }
    };

    let mode = match mode {
        None => AdvanceMode::Manual,
        Some((_, Some(mode))) => mode,
        Some((token, None)) => {
            report_unknown(
                ctx,
                site(),
                &Unknown {
                    option: "presentation:transition-type",
                    value: token.to_owned(),
                    used: "manual",
                },
            );
            AdvanceMode::Manual
        }
    };
    let advance = match mode {
        AdvanceMode::Manual | AdvanceMode::SemiAutomatic => Advance {
            on_click: true,
            after_ms: None,
        },
        AdvanceMode::Automatic => match get(props, qn!("presentation:duration")) {
            None => Advance {
                on_click: false,
                after_ms: Some(Ms(0)),
            },
            Some(written) => {
                let read = value::iso_duration(written);
                match read.value() {
                    Some(after) => {
                        if read.approximated() {
                            report_timing(
                                ctx,
                                site(),
                                "advance",
                                timing_reason(read, written, &format!("{} ms", after.0)),
                            );
                        }
                        Advance {
                            on_click: false,
                            after_ms: Some(after),
                        }
                    }
                    None => {
                        report_timing(
                            ctx,
                            site(),
                            "advance",
                            timing_reason(read, written, "a click"),
                        );
                        Advance {
                            on_click: true,
                            after_ms: None,
                        }
                    }
                }
            }
        },
    };

    let fallback = match effect {
        Some(effect) => effect.fallback(),
        None => TransitionFallback::Dissolve,
    };
    Some(Transition {
        effect: TransitionEffect {
            id: native.effect_id(),
            fallback,
            native: effect.map(|effect| {
                Box::new(NativeTransition {
                    id: native.clone(),
                    effect,
                })
            }),
        },
        duration_ms: duration,
        advance: Some(advance),
        easing: None,
    })
}
