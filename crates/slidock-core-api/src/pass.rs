//! The contract every pass implements. A pass is pure: its output depends only on its input and context. It reads the
//! source phase and hands on the source phase, recomputing the fields it provides from inputs it only reads, so running
//! it twice changes nothing. What it reads and writes is stated as the invariants it requires and provides.

use std::borrow::Cow;
use std::fmt;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use slidock_fonts::FontCatalog;
use slidock_ir::diagnostics::DiagnosticSink;
use slidock_ir::fonts::FontEntry;
use slidock_ir::ids::{NodeId, Sha256};
use slidock_ir::media::MediaEntry;
use slidock_ir::presentation::{Defaults, SlideSize, Theme};
use slidock_ir::source::{Container, Layout, Master, Presentation, Slide};
use slidock_ir::verify::{Invariant, InvariantError};
use thiserror::Error;

use crate::frontend::{HostDecoded, PresentationFacts};
use crate::limits::{Limits, MediaPolicy};

/// A pass's name, version and the invariants it requires and provides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PassInfo {
    pub name: Cow<'static, str>,
    /// Bumped whenever the pass's output changes for the same input.
    pub version: u32,
    pub requires: Cow<'static, [Invariant]>,
    pub provides: Cow<'static, [Invariant]>,
}

impl PassInfo {
    pub const fn new(
        name: &'static str,
        version: u32,
        requires: &'static [Invariant],
        provides: &'static [Invariant],
    ) -> Self {
        PassInfo {
            name: Cow::Borrowed(name),
            version,
            requires: Cow::Borrowed(requires),
            provides: Cow::Borrowed(provides),
        }
    }
}

/// What a pass runs over: the whole presentation, or each container on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Scope {
    Presentation,
    Slide,
}

impl Scope {
    pub const fn as_str(self) -> &'static str {
        match self {
            Scope::Presentation => "presentation",
            Scope::Slide => "slide",
        }
    }
}

/// How a pass fails.
#[derive(Clone, Debug, PartialEq, Error)]
pub enum PassError {
    /// A value an earlier pass should have provided is missing (a narrowing guard failed).
    #[error(transparent)]
    Invariant(#[from] InvariantError),
    /// Anything else.
    #[error("{0}")]
    Failed(String),
}

impl PassError {
    pub fn failed(message: impl Into<String>) -> Self {
        PassError::Failed(message.into())
    }
}

/// The data every pass reads besides its input: the context without the diagnostics sink.
#[derive(Clone, Copy, Debug)]
pub struct PassEnv<'a> {
    pub limits: &'a Limits,
    /// bind-fonts binds against it; text-metrics measures glyph advances with its faces' files.
    pub fonts: &'a FontCatalog,
    pub policy: &'a MediaPolicy,
    pub facts: &'a PresentationFacts,
    /// The media a host decoded (only plan-media reads it); none without a host that decodes.
    pub host_media: &'a [HostDecoded],
}

impl<'a> PassEnv<'a> {
    /// The context a pass runs in, reporting to `diag`.
    pub fn ctx<'b>(&self, diag: &'b mut dyn DiagnosticSink) -> PassCtx<'b>
    where
        'a: 'b,
    {
        PassCtx {
            limits: self.limits,
            fonts: self.fonts,
            policy: self.policy,
            facts: self.facts,
            host_media: self.host_media,
            diag,
        }
    }
}

pub struct PassCtx<'a> {
    pub limits: &'a Limits,
    /// bind-fonts binds against it; text-metrics measures glyph advances with its faces' files.
    pub fonts: &'a FontCatalog,
    pub policy: &'a MediaPolicy,
    pub facts: &'a PresentationFacts,
    /// The media a host decoded (only plan-media reads it).
    pub host_media: &'a [HostDecoded],
    /// Stamped with stage `pass` and the pass's name.
    pub diag: &'a mut dyn DiagnosticSink,
}

impl fmt::Debug for PassCtx<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PassCtx")
            .field("limits", &self.limits)
            .field("policy", &self.policy)
            .field("facts", &self.facts)
            .finish_non_exhaustive()
    }
}

/// A container a slide-scope pass returns.
#[derive(Clone, Debug, PartialEq)]
#[allow(
    clippy::large_enum_variant,
    reason = "a pass returns one container and the manager moves it once"
)]
pub enum OwnedContainer {
    Master(Master),
    Layout(Layout),
    Slide(Slide),
}

impl OwnedContainer {
    pub fn id(&self) -> &NodeId {
        match self {
            OwnedContainer::Master(c) => &c.id,
            OwnedContainer::Layout(c) => &c.id,
            OwnedContainer::Slide(c) => &c.id,
        }
    }

    pub fn kind(&self) -> &'static str {
        self.as_ref().kind()
    }

    pub fn as_ref(&self) -> Container<'_> {
        match self {
            OwnedContainer::Master(c) => Container::Master(c),
            OwnedContainer::Layout(c) => Container::Layout(c),
            OwnedContainer::Slide(c) => Container::Slide(c),
        }
    }
}

impl From<Container<'_>> for OwnedContainer {
    fn from(container: Container<'_>) -> Self {
        match container {
            Container::Master(c) => OwnedContainer::Master(c.clone()),
            Container::Layout(c) => OwnedContainer::Layout(c.clone()),
            Container::Slide(c) => OwnedContainer::Slide(c.clone()),
        }
    }
}

/// What a slide-scope pass sees of the presentation: least privilege. The container's own chain only (a slide sees
/// its layout, master and theme; a layout its master and theme), plus the presentation-wide records. Every view is
/// built from the pass's input presentation, so a view shows only that input and results are independent of
/// iteration order.
#[derive(Clone, Copy, Debug)]
pub struct ContainerView<'a> {
    pub container: Container<'a>,
    pub layout: Option<&'a Layout>,
    pub master: Option<&'a Master>,
    pub theme: Option<&'a Theme>,
    /// Slides only: the 1-based show position, for fields.
    pub show_index: Option<usize>,
    pub defaults: &'a Defaults,
    pub size: &'a SlideSize,
    pub fonts: &'a IndexMap<String, FontEntry>,
    pub media: &'a IndexMap<Sha256, MediaEntry>,
}

impl ContainerView<'_> {
    /// The container, unchanged, as a pass returns it.
    pub fn unchanged(&self) -> OwnedContainer {
        self.container.into()
    }
}

/// A view as JSON, for failure dumps and the hash a failure names; its keys and their order are part of that hash.
impl Serialize for ContainerView<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("kind", self.container.kind())?;
        match self.container {
            Container::Master(c) => map.serialize_entry("container", c)?,
            Container::Layout(c) => map.serialize_entry("container", c)?,
            Container::Slide(c) => map.serialize_entry("container", c)?,
        }
        if let Some(layout) = self.layout {
            map.serialize_entry("layout", layout)?;
        }
        if let Some(master) = self.master {
            map.serialize_entry("master", master)?;
        }
        if let Some(theme) = self.theme {
            map.serialize_entry("theme", theme)?;
        }
        if let Some(index) = self.show_index {
            map.serialize_entry("showIndex", &index)?;
        }
        map.serialize_entry("defaults", self.defaults)?;
        map.serialize_entry("size", self.size)?;
        map.serialize_entry("fonts", self.fonts)?;
        map.serialize_entry("media", self.media)?;
        map.end()
    }
}

/// The view of one container, built from `presentation`.
pub fn view_of<'a>(
    presentation: &'a Presentation,
    container: Container<'a>,
    facts: &PresentationFacts,
) -> ContainerView<'a> {
    let theme_of = |master: Option<&Master>| {
        master
            .and_then(|m| m.theme.as_ref())
            .and_then(|theme| presentation.themes.get(theme))
    };
    let (layout, master, show_index) = match container {
        Container::Master(master) => (None, Some(master), None),
        Container::Layout(layout) => {
            let master = layout
                .master
                .as_ref()
                .and_then(|id| presentation.masters.get(id));
            (None, master, None)
        }
        Container::Slide(slide) => {
            let layout = slide
                .layout
                .as_ref()
                .and_then(|id| presentation.layouts.get(id));
            let master = layout
                .and_then(|l| l.master.as_ref())
                .and_then(|id| presentation.masters.get(id));
            (layout, master, Some(facts.show_index(&slide.id)))
        }
    };
    let theme = theme_of(master);
    ContainerView {
        container,
        layout,
        master: if matches!(container, Container::Master(_)) {
            None
        } else {
            master
        },
        theme,
        show_index,
        defaults: &presentation.defaults,
        size: &presentation.size,
        fonts: &presentation.fonts,
        media: &presentation.media,
    }
}

/// A pass over the whole presentation.
pub trait PresentationPass {
    fn info(&self) -> &PassInfo;

    fn run(
        &self,
        presentation: &Presentation,
        ctx: &mut PassCtx<'_>,
    ) -> Result<Presentation, PassError>;
}

/// A pass run once per container; it returns that container (same id and kind), changed.
pub trait SlidePass {
    fn info(&self) -> &PassInfo;

    fn run(
        &self,
        view: &ContainerView<'_>,
        ctx: &mut PassCtx<'_>,
    ) -> Result<OwnedContainer, PassError>;
}

/// A pass as the manager holds it.
pub enum Pass {
    Presentation(Box<dyn PresentationPass>),
    Slide(Box<dyn SlidePass>),
}

impl fmt::Debug for Pass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pass")
            .field("scope", &self.scope())
            .field("info", self.info())
            .finish()
    }
}

impl Pass {
    pub fn presentation(pass: impl PresentationPass + 'static) -> Self {
        Pass::Presentation(Box::new(pass))
    }

    pub fn slide(pass: impl SlidePass + 'static) -> Self {
        Pass::Slide(Box::new(pass))
    }

    pub fn info(&self) -> &PassInfo {
        match self {
            Pass::Presentation(pass) => pass.info(),
            Pass::Slide(pass) => pass.info(),
        }
    }

    pub fn name(&self) -> &str {
        &self.info().name
    }

    pub fn scope(&self) -> Scope {
        match self {
            Pass::Presentation(_) => Scope::Presentation,
            Pass::Slide(_) => Scope::Slide,
        }
    }
}
