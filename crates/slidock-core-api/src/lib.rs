//! The contracts frontends and passes are written against: the [`Frontend`] and pass traits, the diagnostics
//! sink, limits and policy, failure isolation helpers, lowering helpers and IR visitors. Frontends and passes
//! depend on this crate, never on `slidock-core`. Everything here is synchronous and does no I/O: inputs are read
//! through `slidock-io`'s [`RangeReader`](slidock_io::RangeReader), and time comes from an injected [`Clock`].

pub mod deadline;
pub mod diagnostics;
pub mod failure;
pub mod frontend;
pub mod limits;
pub mod lowering;
pub mod pass;
pub mod visit;

pub use deadline::{Clock, Deadline, Stopped};
pub use diagnostics::{ContainerOrder, Sink, has_errors, sort_diagnostics};
pub use failure::{Causes, FailOptions, Failable, FailureCause, fail_containers, with_dependents};
pub use frontend::{
    Analysis, EmbeddedFont, EmptyMediaStore, Frontend, FrontendCtx, FrontendError, HOST_DECODED,
    HostDecoded, Input, LowerOptions, LowerResult, MediaHandle, MediaSource, MediaStore,
    PresentationFacts, Provenance, SniffHints, Unverified,
};
pub use limits::{Limits, MediaPolicy, Output};
pub use pass::{
    ContainerView, OwnedContainer, Pass, PassCtx, PassEnv, PassError, PassInfo, PresentationPass,
    Scope, SlidePass, view_of,
};
