//! The engine: the frontend registry, the [`PassManager`], the pipeline stages and the two-phase [`Session`]
//! (`open` → `font_needs` / `add_font` and `media_needs` / `add_media` → `finish`, which also ships the faces the deck
//! draws), with deadlines from an injected clock. Passes and frontends are injected by the host as trait objects; this crate knows only their
//! contracts (`slidock-core-api`).

pub mod compile;
pub mod foreign;
pub mod host_media;
pub mod manager;
pub mod pipeline;
pub mod registry;
pub mod session;
pub mod shipping;
pub mod verify;

pub use compile::{
    AnalyzeResult, CompileOptions, CompileResult, FrontendChoice, InputOptions, Phase, SNIFF_BYTES,
    Stats, analyze, compile, stage_stopped,
};
pub use foreign::foreign_diagnostics;
pub use host_media::{MediaNeed, MediaRefusal, check_decoded, media_needs};
pub use manager::{
    ConfigError, FailureDump, PassManager, PassSnapshot, PassTiming, ReplayError, RunOptions,
    RunResult, VerifyMode, replay_failure,
};
pub use pipeline::{AFTER_FONTS, BEFORE_FONTS, PassRegistry, Pipeline, PipelineError};
pub use registry::{FrontendRegistry, RegistryError};
pub use session::{
    AddedFont, Containers, EmbeddedFontInfo, FaceRefusal, Finished, FontIssue, FontNeed,
    FontSource, FontUnwrap, OpenError, Session, SessionError, SessionOptions, SfntOnly,
    SubstituteFace, UnwrappedFont, is_bundled_family,
};
pub use shipping::{CollectGlyphs, DemotedSlots, GlyphUse, ServedFace, ShippedFace};
pub use verify::{VerifiedSource, verify_source_presentation};
