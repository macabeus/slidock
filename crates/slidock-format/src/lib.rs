//! The deck-format contract between the HTML backend, the bundle and the player: the `sd` marker prefix, the reset
//! rule, output paths, escaping, the document-set manifest, the static-HTML lint (`html5ever` + `cssparser`) and the
//! slide-over-layout composer. It is independent of the IR.

pub mod compose;
pub mod dom;
pub mod escape;
pub mod lint;
pub mod manifest;
pub mod markers;
pub mod paths;
pub mod problems;
pub mod references;
pub mod reset;
pub mod style;

pub use compose::{
    ComposeError, ComposedSlide, MediaUrls, Sources, compose, compose_slide, compose_with_sources,
};
pub use escape::{css_string, escape_attribute, escape_text};
pub use lint::{PresentationFiles, lint_presentation};
pub use manifest::{Manifest, ManifestError, read_manifest, serialize_manifest};
pub use problems::{Problem, ProblemCode, ProblemSeverity};
pub use reset::RESET_STYLE;
