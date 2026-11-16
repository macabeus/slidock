//! Failed containers. A container that cannot compile fails alone: it keeps its id, place, links and background, loses
//! its elements, and records why. The presentation around it still compiles, and every printer shows it as a failed
//! document.

use std::collections::BTreeSet;

use indexmap::IndexMap;
use slidock_ir::diagnostics::{Diagnostic, make_diagnostic};
use slidock_ir::ids::{NodeId, ThemeId};
use slidock_ir::origin::Origin;
use slidock_ir::presentation::Failed;
use slidock_ir::source::{Container, Layout, Master, Presentation, Slide};

use crate::diagnostics::site_data;

/// The longest reason, in UTF-16 code units, a diagnostic carries from an error.
pub const REASON_MAX: usize = 200;

/// An error message as the reason a diagnostic carries: its first [`REASON_MAX`] code units.
pub fn describe_error(message: &str) -> String {
    slidock_canon::utf16::clip(message, REASON_MAX).to_owned()
}

/// Why a container failed: the diagnostic that names it, and the pass when a pass failed it.
#[derive(Clone, Debug, PartialEq)]
pub struct FailureCause {
    pub diagnostic: Diagnostic,
    pub pass: Option<String>,
}

impl FailureCause {
    pub fn new(diagnostic: Diagnostic) -> Self {
        FailureCause {
            diagnostic,
            pass: None,
        }
    }

    pub fn of_pass(diagnostic: Diagnostic, pass: impl Into<String>) -> Self {
        FailureCause {
            diagnostic,
            pass: Some(pass.into()),
        }
    }

    /// The marker a container failed for this cause records.
    pub fn failed(&self) -> Failed {
        Failed {
            code: self.diagnostic.code.into(),
            pass: self.pass.clone(),
        }
    }
}

/// Failure causes by container id, in the order they were found.
pub type Causes = IndexMap<NodeId, FailureCause>;

/// The origin of a container failed bare: synthesized by the pass that failed it.
fn bare_origin(failed: &Failed) -> Origin {
    Origin::synth(failed.pass.as_deref().unwrap_or("verify"))
}

/// A container kind's failed skeleton.
pub trait Failable: Sized {
    fn id(&self) -> &NodeId;

    fn failed(&self) -> Option<&Failed>;

    fn container(&self) -> Container<'_>;

    /// The skeleton of this container failed with `failed`: its id, origin, links and background, no elements.
    /// `bare` keeps only what the schema requires, with a synthesized origin, for a container whose kept fields are
    /// themselves what failed (an invalid background, a dangling link).
    fn failed_skeleton(&self, failed: Failed, bare: bool) -> Self;
}

impl Failable for Slide {
    fn id(&self) -> &NodeId {
        &self.id
    }

    fn container(&self) -> Container<'_> {
        Container::Slide(self)
    }

    fn failed(&self) -> Option<&Failed> {
        self.failed.as_ref()
    }

    fn failed_skeleton(&self, failed: Failed, bare: bool) -> Self {
        let keep = !bare;
        Slide {
            id: self.id.clone(),
            origin: if bare {
                bare_origin(&failed)
            } else {
                self.origin.clone()
            },
            layout: self.layout.clone().filter(|_| keep),
            hidden: self.hidden,
            show_layout: self.show_layout,
            color_map_override: self.color_map_override.clone().filter(|_| keep),
            notes: None,
            transition: None,
            timeline: Vec::new(),
            ext: self.ext.clone().filter(|_| keep),
            background: self.background.clone().filter(|_| keep),
            elements: Vec::new(),
            title: None,
            reading_order: None,
            failed: Some(failed),
        }
    }
}

impl Failable for Layout {
    fn id(&self) -> &NodeId {
        &self.id
    }

    fn container(&self) -> Container<'_> {
        Container::Layout(self)
    }

    fn failed(&self) -> Option<&Failed> {
        self.failed.as_ref()
    }

    fn failed_skeleton(&self, failed: Failed, bare: bool) -> Self {
        let keep = !bare;
        Layout {
            id: self.id.clone(),
            origin: if bare {
                bare_origin(&failed)
            } else {
                self.origin.clone()
            },
            name: self.name.clone().filter(|_| keep),
            layout_type: self.layout_type.clone().filter(|_| keep),
            master: self.master.clone().filter(|_| keep),
            show_master: self.show_master,
            color_map_override: self.color_map_override.clone().filter(|_| keep),
            ext: self.ext.clone().filter(|_| keep),
            background: self.background.clone().filter(|_| keep),
            elements: Vec::new(),
            failed: Some(failed),
        }
    }
}

impl Failable for Master {
    fn id(&self) -> &NodeId {
        &self.id
    }

    fn container(&self) -> Container<'_> {
        Container::Master(self)
    }

    fn failed(&self) -> Option<&Failed> {
        self.failed.as_ref()
    }

    fn failed_skeleton(&self, failed: Failed, bare: bool) -> Self {
        let keep = !bare;
        Master {
            id: self.id.clone(),
            origin: if bare {
                bare_origin(&failed)
            } else {
                self.origin.clone()
            },
            name: self.name.clone().filter(|_| keep),
            theme: self.theme.clone().filter(|_| keep),
            color_map: self.color_map.clone().filter(|_| keep),
            text_styles: None,
            ext: self.ext.clone().filter(|_| keep),
            background: self.background.clone().filter(|_| keep),
            elements: Vec::new(),
            failed: Some(failed),
        }
    }
}

/// How [`fail_containers`] fails the containers it is given.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FailOptions {
    /// Containers to fail bare (their kept fields failed too).
    pub bare: BTreeSet<NodeId>,
    /// Themes to drop with the masters failed bare for them.
    pub drop_themes: BTreeSet<ThemeId>,
}

fn fail_one<C: Failable>(container: &mut C, causes: &Causes, options: &FailOptions) {
    if let Some(cause) = causes.get(container.id()) {
        let bare = options.bare.contains(container.id());
        *container = container.failed_skeleton(cause.failed(), bare);
    }
}

/// Replaces each container named in `causes` by its failed skeleton.
pub fn fail_containers(presentation: &mut Presentation, causes: &Causes, options: &FailOptions) {
    if !options.drop_themes.is_empty() {
        presentation
            .themes
            .retain(|key, _| !options.drop_themes.contains(key));
    }
    for master in presentation.masters.values_mut() {
        fail_one(master, causes, options);
    }
    for layout in presentation.layouts.values_mut() {
        fail_one(layout, causes, options);
    }
    for slide in &mut presentation.slides {
        fail_one(slide, causes, options);
    }
}

fn is_failed(presentation: &Presentation, id: &NodeId) -> bool {
    presentation
        .masters
        .get(id)
        .map(|m| m.failed.is_some())
        .or_else(|| presentation.layouts.get(id).map(|l| l.failed.is_some()))
        .or_else(|| {
            presentation
                .slides
                .iter()
                .find(|s| &s.id == id)
                .map(|s| s.failed.is_some())
        })
        .unwrap_or(false)
}

/// The containers that show a failed one fail with it: layouts of a failed master, slides of a failed layout. Each
/// gets the cause's code with `data.via`. Returns the causes with the dependents added, and the dependents'
/// diagnostics.
pub fn with_dependents(presentation: &Presentation, causes: &Causes) -> (Causes, Vec<Diagnostic>) {
    let mut all = causes.clone();
    let mut diagnostics = Vec::new();
    let mut inherit = |all: &mut Causes, dependent: &NodeId, via: &NodeId| {
        let Some(cause) = all.get(via) else { return };
        if all.contains_key(dependent) || is_failed(presentation, dependent) {
            return;
        }
        let source = &cause.diagnostic;
        let site = site_data(source.data.as_ref())
            .container(dependent.clone())
            .data("via", via.as_str());
        let derived = make_diagnostic(source.code, source.stage, &source.emitter, site);
        let pass = cause.pass.clone();
        diagnostics.push(derived.clone());
        all.insert(
            dependent.clone(),
            FailureCause {
                diagnostic: derived,
                pass,
            },
        );
    };
    for (id, layout) in &presentation.layouts {
        if let Some(master) = &layout.master {
            inherit(&mut all, id, master);
        }
    }
    for slide in &presentation.slides {
        if let Some(layout) = &slide.layout {
            inherit(&mut all, &slide.id, layout);
        }
    }
    (all, diagnostics)
}
