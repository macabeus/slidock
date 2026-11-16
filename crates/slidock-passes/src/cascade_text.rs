//! cascade-text: complete paragraph and run properties for every text body a container renders. Per paragraph level the
//! layers are the presentation defaults, the master's text styles for the placeholder's class (none for other shapes
//! and for table cells, whatever the table's placeholder), the body's list style (a shape's already merged down its
//! chain), the paragraph's own properties and, for runs, the run's. Every paragraph also gets complete paragraph-mark
//! properties (`endProps`), which size an empty paragraph and give list labels their run defaults. A raised or lowered
//! run takes its script scale into its size. Sizes stay unscaled by a shrink: a stored one applies later, and one with
//! no stored scale is fitted by text-metrics.

use slidock_core_api::visit::TextSite;
use slidock_core_api::{ContainerView, OwnedContainer, PassCtx, PassError, PassInfo, SlidePass};
use slidock_ir::source::{Container, Element, Master};
use slidock_ir::text::{
    Autofit, BodyProps, Inline, LevelStyles, PartialParaProps, PartialRunProps, SourceParagraph,
    TextBody,
};
use slidock_ir::units::{Size, UnitRatio};
use slidock_ir::verify::{Invariant, complete_body};

use crate::container::{try_map_text_bodies, unit_error, with_mapped_elements};
use crate::layers::{Layer, partial_body, partial_para, partial_run};
use crate::roles::{TextStyleClass, text_style_class};
use crate::theme_refs::{ThemeContext, resolved, theme_context};

pub struct CascadeText;

static INFO: PassInfo = PassInfo::new(
    "cascade-text",
    1,
    &[Invariant::ThemeResolved],
    &[Invariant::TextCascaded],
);

impl SlidePass for CascadeText {
    fn info(&self) -> &PassInfo {
        &INFO
    }

    fn run(
        &self,
        view: &ContainerView<'_>,
        _ctx: &mut PassCtx<'_>,
    ) -> Result<OwnedContainer, PassError> {
        let theme = theme_context(view);
        with_mapped_elements(view.container, |element| {
            try_map_text_bodies(element, |text, site| {
                cascade_body(text, site, element, view, &theme)
            })
        })
    }
}

fn master_of<'a>(view: &ContainerView<'a>) -> Option<&'a Master> {
    match view.container {
        Container::Master(master) => Some(master),
        _ => view.master,
    }
}

/// The layers a body's paragraphs cascade through between the defaults and their own properties.
struct Cascade<'a> {
    styles: [Option<&'a LevelStyles>; 2],
    view: &'a ContainerView<'a>,
    theme: &'a ThemeContext,
}

/// A run's script scale folded into the size it draws at, where its baseline is shifted; the scale itself goes.
fn script_sized(mut props: PartialRunProps) -> Result<PartialRunProps, PassError> {
    let scale = props.script_scale.take();
    let shifted = props.baseline.is_some_and(|b| b.steps() != 0);
    if let (Some(scale), Some(size), true) = (scale, props.size, shifted) {
        props.size = Some(
            Size::quantise(size.to_f64() * scale.to_f64())
                .map_err(|e| unit_error("A script size", e))?,
        );
    }
    Ok(props)
}

fn cascade_paragraph(
    paragraph: &SourceParagraph,
    cascade: &Cascade<'_>,
) -> Result<SourceParagraph, PassError> {
    let level = paragraph.level as usize;
    let defaults = cascade.view.defaults.text.get(level);
    let styles = cascade.styles.map(|s| s.and_then(|s| s.get(level)));
    let default_para = partial_para(&defaults.para);
    let props = PartialParaProps::layered(
        std::iter::once(Some(&default_para))
            .chain(styles.iter().map(|s| s.and_then(|s| s.para.as_ref())))
            .chain(std::iter::once(Some(&paragraph.props))),
    );
    let default_run = partial_run(&defaults.run);
    let base = PartialRunProps::layered(
        std::iter::once(Some(&default_run))
            .chain(styles.iter().map(|s| s.and_then(|s| s.run.as_ref()))),
    );
    let run = |own: Option<&PartialRunProps>| -> Result<PartialRunProps, PassError> {
        let layered = PartialRunProps::layered([Some(&base), own]);
        script_sized(resolved(layered, cascade.theme)?)
    };
    let inlines = paragraph
        .inlines
        .iter()
        .map(|inline| {
            let mut out = inline.clone();
            match &mut out {
                Inline::Tab {} | Inline::Break { props: None } => {}
                Inline::Break { props: Some(props) } => *props = run(Some(props))?,
                Inline::Run { props, .. } | Inline::Field { props, .. } => {
                    *props = run(Some(props))?;
                }
            }
            Ok(out)
        })
        .collect::<Result<Vec<_>, PassError>>()?;
    Ok(SourceParagraph {
        level: paragraph.level,
        props: resolved(props, cascade.theme)?,
        inlines,
        end_props: Some(run(paragraph.end_props.as_ref())?),
    })
}

/// A shrink with a stored scale or line spacing reduction completes the other and is fitted (the source app fitted
/// it), its steps no longer needed; one with neither stays without until text-metrics fits it by its steps.
fn complete_autofit(autofit: Autofit) -> Autofit {
    match autofit {
        Autofit::Shrink {
            font_scale,
            line_spacing_reduction,
            fitted,
            ..
        } if font_scale.is_some() || line_spacing_reduction.is_some() => Autofit::Shrink {
            font_scale: Some(font_scale.unwrap_or(UnitRatio::from_steps(100_000))),
            line_spacing_reduction: Some(line_spacing_reduction.unwrap_or(UnitRatio::ZERO)),
            fitted: Some(fitted.unwrap_or(true)),
            steps: None,
        },
        other => other,
    }
}

fn cascade_body(
    text: &TextBody,
    site: TextSite,
    element: &Element,
    view: &ContainerView<'_>,
    theme: &ThemeContext,
) -> Result<TextBody, PassError> {
    let class = match site {
        TextSite::Shape => text_style_class(element.placeholder().map(|p| p.role)),
        TextSite::Cell => None,
    };
    let master = class.and_then(|class| {
        let styles = master_of(view)?.text_styles.as_ref()?;
        match class {
            TextStyleClass::Title => styles.title.as_ref(),
            TextStyleClass::Body => styles.body.as_ref(),
            TextStyleClass::Other => styles.other.as_ref(),
        }
    });
    let cascade = Cascade {
        styles: [master, text.list_style.as_ref()],
        view,
        theme,
    };
    let body = complete_body(&text.body, element.id())?;
    let autofit = complete_autofit(body.autofit.clone());
    let paragraphs = text
        .paragraphs
        .iter()
        .map(|paragraph| cascade_paragraph(paragraph, &cascade))
        .collect::<Result<Vec<_>, PassError>>()?;
    let mut out = text.clone();
    out.body = partial_body(&BodyProps { autofit, ..body });
    out.paragraphs = paragraphs;
    Ok(out)
}
