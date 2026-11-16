//! The frontends a host offers. Sniffing asks them in registration order, so the host decides which formats compile.

use std::fmt;

use slidock_core_api::{Frontend, SniffHints};
use slidock_ir::formats::SourceFormat;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum RegistryError {
    #[error("Two frontends read {}", .0.as_str())]
    Duplicate(SourceFormat),
}

pub struct FrontendRegistry {
    frontends: Vec<Box<dyn Frontend>>,
}

impl fmt::Debug for FrontendRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.frontends.iter().map(|frontend| frontend.format()))
            .finish()
    }
}

impl FrontendRegistry {
    /// The frontends in sniffing order; two frontends of one format are refused.
    pub fn new(frontends: Vec<Box<dyn Frontend>>) -> Result<Self, RegistryError> {
        let mut formats: Vec<SourceFormat> = Vec::with_capacity(frontends.len());
        for frontend in &frontends {
            let format = frontend.format();
            if formats.contains(&format) {
                return Err(RegistryError::Duplicate(format));
            }
            formats.push(format);
        }
        Ok(FrontendRegistry { frontends })
    }

    pub fn frontends(&self) -> impl Iterator<Item = &dyn Frontend> {
        self.frontends.iter().map(|frontend| frontend.as_ref())
    }

    pub fn get(&self, format: SourceFormat) -> Option<&dyn Frontend> {
        self.frontends()
            .find(|frontend| frontend.format() == format)
    }

    /// The first frontend that reads the input.
    pub fn sniff(&self, head: &[u8], hints: &SniffHints<'_>) -> Option<&dyn Frontend> {
        self.frontends()
            .find(|frontend| frontend.sniff(head, hints))
    }
}
