//! The pass order of a compile, split where the passes start to need fonts. Passes are injected:
//! `slidock-core` knows their names and order, never their code.

use indexmap::IndexMap;
use slidock_core_api::Pass;
use thiserror::Error;

use crate::manager::{ConfigError, PassManager};

/// The passes `Session::open` runs, in order: none of them reads a font.
pub const BEFORE_FONTS: [&str; 6] = [
    "normalize-frames",
    "bind-placeholders",
    "resolve-inheritance",
    "resolve-theme",
    "cascade-text",
    "number-lists",
];

/// The passes `Session::finish` runs, in order, from the fonts the host added.
pub const AFTER_FONTS: [&str; 7] = [
    "bind-fonts",
    "text-metrics",
    "lower-geometry",
    "plan-media",
    "normalize-timeline",
    "a11y",
    "collect-glyphs",
];

/// Passes of the standard order a pipeline may leave out: slidock's own additions, which change no IR byte.
pub const OPTIONAL: [&str; 1] = ["collect-glyphs"];

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum PipelineError {
    #[error("Pass {0} appears twice")]
    Duplicate(String),
    #[error("The pipeline needs pass {0}, which is not registered")]
    Missing(String),
    #[error("Pass {0} is not part of the pipeline")]
    Unknown(String),
    #[error(transparent)]
    Config(#[from] ConfigError),
}

/// Passes by name, as a host registers them.
#[derive(Debug, Default)]
pub struct PassRegistry {
    passes: IndexMap<String, Pass>,
}

impl PassRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, pass: Pass) -> Result<(), PipelineError> {
        let name = pass.name().to_owned();
        if self.passes.contains_key(&name) {
            return Err(PipelineError::Duplicate(name));
        }
        self.passes.insert(name, pass);
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.passes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.passes.is_empty()
    }
}

/// The passes of a compile and where fonts come in: the passes before the boundary run once per session, those
/// after it run again whenever the fonts change.
#[derive(Debug)]
pub struct Pipeline {
    manager: PassManager,
    font_boundary: usize,
}

impl Pipeline {
    /// Any pass lists; the manager checks their order as one list.
    pub fn new(before_fonts: Vec<Pass>, after_fonts: Vec<Pass>) -> Result<Self, ConfigError> {
        let font_boundary = before_fonts.len();
        let mut passes = before_fonts;
        passes.extend(after_fonts);
        Ok(Pipeline {
            manager: PassManager::new(passes)?,
            font_boundary,
        })
    }

    /// The standard order ([`BEFORE_FONTS`], then [`AFTER_FONTS`]) from the registered passes. Every pass of the
    /// order must be registered except an [`OPTIONAL`] one, and every registered pass must be part of the order.
    pub fn standard(mut registry: PassRegistry) -> Result<Self, PipelineError> {
        let mut take = |names: &[&str]| -> Result<Vec<Pass>, PipelineError> {
            let mut passes = Vec::new();
            for name in names {
                match registry.passes.shift_remove(*name) {
                    Some(pass) => passes.push(pass),
                    None if OPTIONAL.contains(name) => {}
                    None => return Err(PipelineError::Missing((*name).to_owned())),
                }
            }
            Ok(passes)
        };
        let before = take(&BEFORE_FONTS)?;
        let after = take(&AFTER_FONTS)?;
        if let Some(name) = registry.passes.keys().next() {
            return Err(PipelineError::Unknown(name.clone()));
        }
        Ok(Pipeline::new(before, after)?)
    }

    pub fn manager(&self) -> &PassManager {
        &self.manager
    }

    /// How many passes run before fonts are needed.
    pub fn font_boundary(&self) -> usize {
        self.font_boundary
    }
}
