//! resolve-theme: every theme reference a container renders from resolves against its theme and effective colour map (a
//! slide's override, else its layout's, else the master's): colours in fills, strokes, shadows, gradient stops,
//! backgrounds, text and bullets, list styles and master text styles get their `resolved` value with the modifiers
//! applied, and theme fonts become `{family, theme}`. A container without a theme resolves against Office's defaults and warns
//! MID5301.

use slidock_core_api::{ContainerView, OwnedContainer, PassCtx, PassError, PassInfo, SlidePass};
use slidock_ir::diagnostics::{DiagnosticSite, codes};
use slidock_ir::verify::Invariant;

use crate::theme_refs::{ThemeRefs, theme_context};

pub struct ResolveTheme;

static INFO: PassInfo = PassInfo::new(
    "resolve-theme",
    1,
    &[Invariant::InheritanceResolved],
    &[Invariant::ThemeResolved],
);

impl SlidePass for ResolveTheme {
    fn info(&self) -> &PassInfo {
        &INFO
    }

    fn run(
        &self,
        view: &ContainerView<'_>,
        ctx: &mut PassCtx<'_>,
    ) -> Result<OwnedContainer, PassError> {
        let mut container = view.unchanged();
        if !container.has_refs() {
            return Ok(container);
        }
        let theme = theme_context(view);
        for what in &theme.missing {
            ctx.diag.report(
                codes::MID5301,
                DiagnosticSite::new()
                    .container(container.id().clone())
                    .data("what", *what),
            );
        }
        container.resolve_refs(&theme)?;
        Ok(container)
    }
}
