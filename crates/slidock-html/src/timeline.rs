//! The timeline template: off the parity path, a slide document's `<head>` lists the slide's build
//! steps as `<template data-sd-timeline><ol><li …></li></ol></template>`, one `<li>` per step, so the document
//! stays inert and fully built. The deck JSON carries the same steps and the transition for the player: a deck
//! page's slide templates hold the composed `<main>` alone, while a composed slide document keeps its head.
//!
//! Each `<li>` prints its step's fields as data attributes, in this order: `data-target`, `data-trigger`,
//! `data-kind`, `data-effect` and one attribute per effect parameter (`data-dir`, `data-distance`, …), `data-native`,
//! `data-fallback`, `data-delivery`, `data-paragraphs` (`first-last`), `data-background`, the stagger
//! (`data-stagger-ms` or `data-stagger-fraction`, and `data-stagger-order`), `data-offset-ms`, `data-delay-ms`,
//! `data-duration-ms`, `data-easing`, `data-repeat`, `data-auto-reverse` and `data-after`. Absent fields print no
//! attribute; a step without an effect in the common vocabulary prints no `data-effect` and names its opaque id in
//! `data-native`.
//!
//! Values print as the deck JSON writes them: a string as itself, a number as JavaScript prints it, a boolean
//! parameter as `true` or `false`, and anything structured (a motion path, an easing curve, a scale pair, a repeat
//! count) as compact canonical JSON; the step's own flags (`data-background`, `data-auto-reverse`) print bare when
//! set. A parameter takes `data-<name>` in kebab case, except one whose name a step attribute already takes, which
//! takes `data-effect-<name>` (blink's `repeat`). The tier-2 keyframes, which have no named parameters, print in one
//! `data-params`.

use serde_json::Value;
use slidock_canon::{Layout, to_js_string, value_to_canonical_json};
use slidock_ir::diagnostics::{Diagnostic, DiagnosticSite, Stage, codes, make_diagnostic};
use slidock_ir::effects::StaggerAmount;
use slidock_ir::resolved::Presentation;
use slidock_ir::timeline::TimelineStep;

use crate::context::EMITTER;
use crate::error::{HtmlError, Result};
use crate::paint_motion;
use crate::tree::{Attrs, Element, lines};

/// The template's marker attribute.
pub const TIMELINE: &str = "data-sd-timeline";

/// The attribute a parameter prints as, by its JSON name.
fn param_attribute(name: &str) -> Option<&'static str> {
    Some(match name {
        "alpha" => "data-alpha",
        "autoRotate" => "data-auto-rotate",
        "axis" => "data-axis",
        "bounce" => "data-bounce",
        "by" => "data-by",
        "color" => "data-color",
        "corner" => "data-corner",
        "cw" => "data-cw",
        "d" => "data-d",
        "deg" => "data-deg",
        "dir" => "data-dir",
        "distance" => "data-distance",
        "fade" => "data-fade",
        "fadeUnmatched" => "data-fade-unmatched",
        "from" => "data-from",
        "hold" => "data-hold",
        "inOut" => "data-in-out",
        "intensity" => "data-intensity",
        "orient" => "data-orient",
        "origin" => "data-origin",
        "overshoot" => "data-overshoot",
        "property" => "data-property",
        "repeat" => "data-effect-repeat",
        "rotate" => "data-rotate",
        "scale" => "data-scale",
        "shape" => "data-shape",
        "slight" => "data-slight",
        "slow" => "data-slow",
        "soft" => "data-soft",
        "spokes" => "data-spokes",
        "through" => "data-through",
        "throughBlack" => "data-through-black",
        "to" => "data-to",
        _ => return None,
    })
}

/// A JSON value as an attribute value.
fn value_text(value: &Value) -> Result<String> {
    Ok(match value {
        Value::String(text) => text.clone(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => number
            .as_f64()
            .map(to_js_string)
            .unwrap_or_else(|| number.to_string()),
        Value::Null | Value::Array(_) | Value::Object(_) => {
            value_to_canonical_json(value, Layout::Compact)?
        }
    })
}

fn json<T: serde::Serialize>(value: &T) -> Result<Value> {
    serde_json::to_value(value).map_err(|error| HtmlError::Ir(error.to_string()))
}

/// A value's JSON text when it prints as a string (an enum's token), else its canonical JSON.
pub(crate) fn token<T: serde::Serialize>(value: &T) -> Result<String> {
    value_text(&json(value)?)
}

/// The effect and parameter attributes of a step's native effect.
fn effect_attributes(step: &TimelineStep, mut attrs: Attrs) -> Result<Attrs> {
    let Some(native) = &step.effect.native else {
        return Ok(attrs);
    };
    let effect = &native.effect;
    attrs = attrs.val("data-effect", effect.name().to_owned());
    match json(effect)?.get("params") {
        None => {}
        Some(Value::Object(params)) => {
            for (name, value) in params {
                let attribute = param_attribute(name).ok_or_else(|| {
                    HtmlError::Ir(format!(
                        "the {} effect's parameter {name} has no attribute",
                        effect.name()
                    ))
                })?;
                attrs = attrs.val(attribute, value_text(value)?);
            }
        }
        Some(params) => attrs = attrs.val("data-params", value_text(params)?),
    }
    Ok(attrs)
}

/// One step as its `<li>`.
fn step_item(step: &TimelineStep) -> Result<Element> {
    let mut attrs = Attrs::new()
        .val("data-target", step.target.element.to_string())
        .val("data-trigger", token(&step.trigger)?)
        .val("data-kind", token(&step.kind)?);
    attrs = effect_attributes(step, attrs)?;
    let native = step.effect.native.as_ref().map_or_else(
        || step.effect.id.to_string(),
        |native| native.id.to_string(),
    );
    attrs = attrs
        .val("data-native", native)
        .val("data-fallback", token(&step.effect.fallback)?)
        .val("data-delivery", token(&step.delivery)?)
        .val(
            "data-paragraphs",
            step.target
                .paragraphs
                .map(|(first, last)| format!("{first}-{last}")),
        )
        .flag("data-background", step.target.background);
    let timing = step.timing.as_deref();
    if let Some(stagger) = timing.and_then(|timing| timing.stagger.as_ref()) {
        attrs = match stagger.per_unit {
            StaggerAmount::Ms(ms) => attrs.val("data-stagger-ms", ms.0.to_string()),
            StaggerAmount::Fraction(fraction) => {
                attrs.val("data-stagger-fraction", to_js_string(fraction.to_f64()))
            }
        };
        attrs = attrs.val("data-stagger-order", token(&stagger.order)?);
    }
    attrs = attrs
        .val(
            "data-offset-ms",
            timing
                .and_then(|timing| timing.offset_ms)
                .map(|ms| ms.0.to_string()),
        )
        .val("data-delay-ms", step.delay_ms.0.to_string())
        .val("data-duration-ms", step.duration_ms.0.to_string());
    if let Some(timing) = timing {
        attrs = attrs.val(
            "data-easing",
            timing.easing.as_ref().map(token).transpose()?,
        );
        attrs = attrs
            .val(
                "data-repeat",
                timing.repeat.as_ref().map(token).transpose()?,
            )
            .flag("data-auto-reverse", timing.auto_reverse)
            .val("data-after", timing.after.as_ref().map(token).transpose()?);
    }
    Ok(Element::new("li", attrs, Vec::new()))
}

/// The template that lists `steps`, or none for a slide without steps.
pub fn timeline_template(steps: &[TimelineStep]) -> Result<Option<Element>> {
    if steps.is_empty() {
        return Ok(None);
    }
    let items = steps.iter().map(step_item).collect::<Result<Vec<_>>>()?;
    let list = Element::new("ol", Attrs::new(), lines(items));
    Ok(Some(Element::new(
        "template",
        Attrs::new().flag(TIMELINE, true),
        vec![list.into()],
    )))
}

/// HTML6010 for each step and transition that prints only its fallback (no effect in the common vocabulary), in
/// slide order, a slide's steps before its transition: every fallback is noted once. A step that plays its fallback
/// because its target cannot expose the paint it animates is also noted with HTML6011, before its HTML6010.
pub(crate) fn fallback_notes(presentation: &Presentation) -> Result<Vec<Diagnostic>> {
    let mut notes = Vec::new();
    for slide in &presentation.slides {
        let site = |effect: String, fallback: String| {
            DiagnosticSite::new()
                .container(slide.id.clone())
                .data("effect", effect)
                .data("fallback", fallback)
        };
        for played in paint_motion::played_steps(slide, presentation) {
            let step = &played.step;
            if step.effect.native.is_none() {
                let fallback = token(&step.effect.fallback)?;
                let at =
                    site(step.effect.id.to_string(), fallback).node(step.target.element.clone());
                if let Some((hook, reason)) = played.unhooked {
                    let (property, reason) = paint_motion::unhooked_data(hook, reason);
                    notes.push((
                        codes::HTML6011,
                        at.clone().data("property", property).data("reason", reason),
                    ));
                }
                notes.push((codes::HTML6010, at));
            }
        }
        if let Some(transition) = &slide.transition
            && transition.effect.native.is_none()
        {
            let fallback = token(&transition.effect.fallback)?;
            notes.push((
                codes::HTML6010,
                site(transition.effect.id.to_string(), fallback),
            ));
        }
    }
    Ok(notes
        .into_iter()
        .map(|(code, site)| make_diagnostic(code, Stage::Backend, EMITTER, site))
        .collect())
}
