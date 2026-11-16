//! The public Rust API of slidock: the frontends, the passes and the HTML backend wired together.
//!
//! - [`Compiler::compile`] is one input through its frontend, every pass, the backend, the loss check and the lint: the
//!   document set `--emit documents` writes, the resolved IR and the compile report.
//! - [`Session`] is the two-phase compile (`open` → `font_needs` / `add_font`, `media_needs` / `add_media` →
//!   `finish`) over [`pipeline`], the
//!   standard pass order; [`print_finished`] prints what it finished, and [`finish_deck`] makes it a deck: the
//!   deck JSON ([`deck`]) and the document set.
//! - [`bundle`] writes a deck as the product's `index.html` and zip; [`compile`] is one input from `open` to the
//!   bundle, with no font source; [`fonts`] is what a host that ships fonts adds.
//!
//! Everything is synchronous and reads only the [`Input`]'s range reader and the media files it is given.

pub mod alternates;
pub mod driver;
pub mod fonts;
pub mod images;
pub mod loss;
mod media;
mod package;
mod report;
pub mod videos;

use slidock_core::{
    AnalyzeResult, CompileOptions, ConfigError, PassRegistry, Phase, PipelineError, RegistryError,
    analyze,
};
use slidock_core_api::sort_diagnostics;
use slidock_ir::diagnostics::codes;
use slidock_ir::features::FeatureId;
use slidock_ir::resolved;
use thiserror::Error;

pub use driver::{
    DriverError, Emit, PrintInput, Printed, html_provenance, print_presentation,
    print_presentation_with, with_available_media,
};
pub use loss::{LossCheckInput, loss_check};
pub use media::{MediaError, write_media_file};
pub use package::{
    Bundled, CompileError, FinishedDeck, StoreMedia, added_font_files, bundle_within, compile,
    finish_deck,
};
pub use report::{Described, compile_report};
pub use slidock_bundle::{
    AlternateSource, BundleError, BundleOptions, BundleReport, BundleSink, BundledFile,
    CHUNK_BYTES, CompiledDeck, DEFAULT_SINGLE_FILE_THRESHOLD, DeckMedia, NotesMode, Packaging,
    PageDefaults, ReplacedMedia, ShippedFace, SiteUrlError, SlideTemplate, bundle, bundled_deck,
    embedded_player, lint_templates, page_defaults, site_url, size_notice, slide_templates,
};
pub use slidock_canon::{JsonError, Layout as JsonLayout, to_canonical_json};
pub use slidock_core::FailureDump;
pub use slidock_core::{
    AddedFont, CompileResult, Containers, EmbeddedFontInfo, FaceRefusal, Finished, FontIssue,
    FontNeed, FontSource, FontUnwrap, FrontendChoice, FrontendRegistry, MediaNeed, MediaRefusal,
    OpenError, PassManager, PassTiming, Pipeline, Session, SessionError, SessionOptions, Stats,
    SubstituteFace, VerifyMode, is_bundled_family, shipping,
};
pub use slidock_core_api::{
    Clock, ContainerOrder, Deadline, HostDecoded, Input, Limits, MediaHandle, MediaPolicy,
    MediaSource, MediaStore, Output, Provenance, has_errors,
};
pub use slidock_fonts::{CatalogFace, FontCatalog, SfntError, builtin_catalog, read_sfnt};
pub use slidock_format::manifest::Generator;
#[cfg(feature = "ir-input")]
pub use slidock_frontend_ir::{IR_FILE_SUFFIX, IrFrontend, MediaFiles, is_media_file_name};
pub use slidock_html::{
    Documents, EmitOptions, Emitted, HtmlError, MediaAction, MediaPlanRef, Motion, deck,
    slidock_generator, supported_features,
};
#[cfg(feature = "std-fs")]
pub use slidock_io::FileReader;
pub use slidock_io::{BytesReader, RangeError, RangeReader, check_range};
pub use slidock_ir::canonical::dump_presentation;
pub use slidock_ir::diagnostics::{Diagnostic, Severity};
pub use slidock_ir::formats::SourceFormat;
pub use slidock_ir::ids::{Sha256, SlideId};
pub use slidock_keynote::KeynoteFrontend;
pub use slidock_odp::OdpFrontend;
pub use slidock_pptx::PptxFrontend;

/// The schema URL every IR document's `$schema` holds.
pub const SCHEMA_URL: &str = slidock_ir::presentation::SchemaUrl::WIRE;

/// Whether this build reads IR documents (the `ir-input` feature). The wasm build asserts it does not.
pub const IR_INPUT: bool = cfg!(feature = "ir-input");

/// What the frontends need from the host. Built from [`Frontends::default`] and the `with_*` setters, so a new
/// frontend's input is one more setter.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct Frontends {
    /// The media files of an IR input (the CLI reads the `media/` folder beside it).
    #[cfg(feature = "ir-input")]
    pub ir_media: MediaFiles,
}

impl Frontends {
    /// These frontends with the IR input's media files.
    #[cfg(feature = "ir-input")]
    pub fn with_ir_media(mut self, media: MediaFiles) -> Self {
        self.ir_media = media;
        self
    }

    /// Every frontend, in sniffing order: PPTX, ODP, Keynote, then, with the `ir-input` feature, the IR, which also
    /// claims any input named `*.presentation-ir.json`. Without it an IR document is an input no frontend reads.
    pub fn registry(self) -> Result<FrontendRegistry, RegistryError> {
        #[cfg_attr(not(feature = "ir-input"), allow(unused_mut))]
        let mut frontends = binary_frontends();
        #[cfg(feature = "ir-input")]
        frontends.push(Box::new(IrFrontend::new(self.ir_media)));
        FrontendRegistry::new(frontends)
    }
}

/// The formats this build compiles, in sniffing order: PPTX, ODP, Keynote, then, with the `ir-input` feature, IR.
pub fn input_formats() -> Vec<SourceFormat> {
    let mut formats: Vec<SourceFormat> = binary_frontends().iter().map(|f| f.format()).collect();
    if IR_INPUT {
        formats.push(SourceFormat::Ir);
    }
    formats
}

/// The frontends every build has, in sniffing order: PPTX, ODP, Keynote.
fn binary_frontends() -> Vec<Box<dyn slidock_core_api::Frontend>> {
    vec![
        Box::new(PptxFrontend),
        Box::new(OdpFrontend),
        Box::new(KeynoteFrontend),
    ]
}

/// Why a [`Compiler`] cannot be built: a frontend list or pass list the core refuses.
#[derive(Clone, Debug, PartialEq, Error)]
pub enum SetupError {
    #[error(transparent)]
    Registry(#[from] RegistryError),
    #[error(transparent)]
    Passes(#[from] ConfigError),
    #[error(transparent)]
    Pipeline(#[from] PipelineError),
}

/// Every pass, in the order the pass manager runs them.
pub fn pass_manager() -> Result<PassManager, ConfigError> {
    PassManager::new(slidock_passes::passes())
}

/// The standard pipeline over every pass, split where the passes start to need fonts: what a [`Session`] runs.
pub fn pipeline() -> Result<Pipeline, PipelineError> {
    let mut registry = PassRegistry::new();
    for pass in slidock_passes::passes() {
        registry.register(pass)?;
    }
    Pipeline::standard(registry)
}

/// How one input compiles.
#[derive(Clone)]
pub struct Options<'a> {
    pub frontend: FrontendChoice,
    /// Lower and compile only these slides (with their layout, master and theme chain).
    pub only: Option<&'a [SlideId]>,
    pub verify: VerifyMode,
    pub limits: Limits,
    /// The fonts bind-fonts binds against.
    pub fonts: &'a FontCatalog,
    pub policy: MediaPolicy,
    /// For stage deadlines and pass timings; nothing times out without one.
    pub clock: Option<&'a dyn Clock>,
    /// Keeps the presentation after each pass.
    pub trace: bool,
    /// What the document set names as its writer.
    pub generator: Generator,
    /// Keep the native motion data (the effect vocabulary, timing and match keys) and print it: the deck JSON's
    /// effects, the timeline templates and `data-sd-match`. Off, the parity path, the outputs are those of the IR
    /// without it.
    pub native_motion: bool,
}

impl std::fmt::Debug for Options<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Options")
            .field("frontend", &self.frontend)
            .field("only", &self.only)
            .field("verify", &self.verify)
            .field("clock", &self.clock.is_some())
            .field("trace", &self.trace)
            .field("generator", &self.generator)
            .field("native_motion", &self.native_motion)
            .finish_non_exhaustive()
    }
}

impl Default for Options<'static> {
    /// The sniffed frontend, every slide, default limits and policy, the built-in fonts, verified at the boundaries,
    /// written by slidock, native motion left out.
    fn default() -> Self {
        Options {
            frontend: FrontendChoice::Auto,
            only: None,
            verify: VerifyMode::Boundaries,
            limits: Limits::default(),
            fonts: builtin_catalog(),
            policy: MediaPolicy::default(),
            clock: None,
            trace: false,
            generator: slidock_generator(),
            native_motion: false,
        }
    }
}

/// How far [`Compiler::run`] goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    /// After lowering and verify-source.
    Source,
    /// After every pass and verify-resolved.
    Resolved,
}

/// An input compiled to HTML.
pub struct Compiled {
    /// The compile through the passes.
    pub result: CompileResult,
    /// The presentation as printed; `None` when nothing compiled.
    pub printed: Option<Printed>,
    /// Every diagnostic, the compile's included, in output order.
    pub diagnostics: Vec<Diagnostic>,
}

impl std::fmt::Debug for Compiled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Compiled")
            .field("result", &self.result)
            .field("printed", &self.printed.is_some())
            .field("diagnostics", &self.diagnostics)
            .finish()
    }
}

impl Compiled {
    /// The presentation as printed: containers the driver failed are marked.
    pub fn presentation(&self) -> Option<&resolved::Presentation> {
        self.printed.as_ref().map(|printed| &printed.presentation)
    }

    /// The document set; `None` when the presentation failed as a whole.
    pub fn documents(&self) -> Option<&Documents> {
        self.printed.as_ref()?.documents.as_ref()
    }

    /// The compile report of the printed presentation, as indented canonical JSON.
    pub fn report(&self) -> Result<String, DriverError> {
        compile_report(
            &self.result,
            self.presentation().map(Described::Resolved),
            &self.diagnostics,
        )
    }

    /// The printed presentation as a canonical IR dump.
    pub fn ir(&self) -> Option<Result<String, DriverError>> {
        self.presentation()
            .map(|presentation| dump_presentation(presentation).map_err(DriverError::from))
    }

    /// The media store the compile's frontend holds.
    pub fn media(&self) -> Option<&dyn MediaStore> {
        self.result.media.as_deref()
    }
}

/// The frontends and passes of a compile, built once.
pub struct Compiler {
    registry: FrontendRegistry,
    passes: PassManager,
}

impl std::fmt::Debug for Compiler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Compiler")
            .field("registry", &self.registry)
            .field("passes", &self.passes)
            .finish()
    }
}

impl Compiler {
    pub fn new(frontends: Frontends) -> Result<Compiler, SetupError> {
        Ok(Compiler {
            registry: frontends.registry()?,
            passes: pass_manager()?,
        })
    }

    pub fn registry(&self) -> &FrontendRegistry {
        &self.registry
    }

    pub fn passes(&self) -> &PassManager {
        &self.passes
    }

    fn core_options<'a>(&'a self, options: &Options<'a>, stop: Stop) -> CompileOptions<'a> {
        CompileOptions {
            registry: &self.registry,
            frontend: options.frontend,
            only: options.only,
            phase: match stop {
                Stop::Source => Phase::Source,
                Stop::Resolved => Phase::Resolved(&self.passes),
            },
            verify: options.verify,
            limits: options.limits,
            fonts: options.fonts,
            policy: options.policy,
            clock: options.clock,
            trace: options.trace,
            native_motion: options.native_motion,
        }
    }

    /// The frontend's analysis of an input: metadata only, before anything is lowered.
    pub fn analyze(&self, input: &Input<'_>, options: &Options<'_>) -> AnalyzeResult {
        analyze(input, &self.core_options(options, Stop::Source).input())
    }

    /// One input through its frontend and, to [`Stop::Resolved`], the passes; nothing is printed.
    pub fn run(&self, input: &Input<'_>, options: &Options<'_>, stop: Stop) -> CompileResult {
        slidock_core::compile(input, &self.core_options(options, stop))
    }

    /// compile → print → loss check → lint, for one input.
    pub fn compile(
        &self,
        input: &Input<'_>,
        options: &Options<'_>,
    ) -> Result<Compiled, DriverError> {
        let deadline = Deadline::start(options.clock, options.limits.compile_ms);
        let result = self.run(input, options, Stop::Resolved);
        let (Some(presentation), Some(source), Some(provenance), Some(media)) = (
            &result.presentation,
            &result.source,
            &result.provenance,
            &result.media,
        ) else {
            let diagnostics = result.diagnostics.clone();
            return Ok(Compiled {
                result,
                printed: None,
                diagnostics,
            });
        };
        let emit = EmitOptions {
            provenance: html_provenance(provenance),
            generator: options.generator.clone(),
        };
        let printed = print_presentation(&PrintInput {
            presentation,
            source,
            emit: &emit,
            features: &result.features,
            diagnostics: &result.diagnostics,
            media: media.as_ref(),
        })?;
        let mut diagnostics = merged(&result.diagnostics, &printed);
        if deadline.expired() {
            diagnostics.push(slidock_core::stage_stopped("print", &deadline, false));
            return Ok(Compiled {
                result,
                printed: None,
                diagnostics,
            });
        }
        Ok(Compiled {
            result,
            printed: Some(printed),
            diagnostics,
        })
    }
}

/// The compile's diagnostics and the printer's, in output order.
fn merged(compiled: &[Diagnostic], printed: &Printed) -> Vec<Diagnostic> {
    let all = compiled
        .iter()
        .chain(printed.diagnostics())
        .cloned()
        .collect();
    sort_diagnostics(all, &ContainerOrder::of_resolved(&printed.presentation))
}

/// A session's media, as a store.
#[derive(Clone, Copy)]
pub struct SessionMedia<'s, 'a>(pub &'s Session<'a>);

impl MediaStore for SessionMedia<'_, '_> {
    fn open(&self, sha256: &Sha256) -> Option<MediaHandle> {
        self.0.media(sha256)
    }

    fn decoded(&self, sha256: &Sha256) -> Option<MediaHandle> {
        self.0.decoded_file(sha256)
    }
}

/// What printing a finished session gives.
#[derive(Clone, Debug, PartialEq)]
pub struct PrintedSession {
    pub printed: Printed,
    /// Every diagnostic, the session's included, in output order.
    pub diagnostics: Vec<Diagnostic>,
}

/// Prints what a session finished, as [`Compiler::compile`] prints a compile; `None` when nothing resolved.
pub fn print_finished(
    session: &Session<'_>,
    finished: &Finished,
    generator: Generator,
) -> Result<Option<PrintedSession>, DriverError> {
    let (Some(presentation), Some(source)) = (&finished.presentation, &finished.source) else {
        return Ok(None);
    };
    let emit = EmitOptions {
        provenance: html_provenance(session.provenance()),
        generator,
    };
    // The text of the families the deck ships faces of prints their families.
    let stacks = shipping::font_stacks(&finished.fonts);
    let motion = if session.native_motion() {
        Motion::On
    } else {
        Motion::Off
    };
    let printed = print_presentation_with(
        &PrintInput {
            presentation,
            source,
            emit: &emit,
            features: session.features(),
            diagnostics: &finished.diagnostics,
            media: &SessionMedia(session),
        },
        &mut |presentation, options| {
            slidock_html::emit_documents_shipping(presentation, options, &stacks, motion)
        },
    )?;
    let mut diagnostics = merged(&finished.diagnostics, &printed);
    if session.ships_embedded_fonts() {
        // The deck ships the embedded fonts the backend's census calls unused.
        diagnostics.retain(|diagnostic| {
            diagnostic.code != codes::HTML6004
                || diagnostic.feature != Some(FeatureId::FontEmbedded)
        });
    }
    Ok(Some(PrintedSession {
        printed,
        diagnostics,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offers_every_frontend_in_sniffing_order_and_every_pass() {
        let registry = Frontends::default().registry().unwrap();
        let formats: Vec<&str> = registry.frontends().map(|f| f.format().as_str()).collect();
        let expected: &[&str] = if cfg!(feature = "ir-input") {
            &["pptx", "odp", "keynote", "ir"]
        } else {
            &["pptx", "odp", "keynote"]
        };
        assert_eq!(formats, expected);
        assert_eq!(pass_manager().unwrap().passes().len(), 12);
        let compiler = Compiler::new(Frontends::default()).unwrap();
        assert_eq!(compiler.passes().passes().len(), 12);
    }

    /// A zip's first local header holding a stored entry `name` with `data`.
    fn zip_head(name: &str, data: &[u8]) -> Vec<u8> {
        let mut head = b"PK\x03\x04".to_vec();
        head.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        let size = u32::try_from(data.len()).unwrap().to_le_bytes();
        head.extend_from_slice(&size);
        head.extend_from_slice(&size);
        head.extend_from_slice(&u16::try_from(name.len()).unwrap().to_le_bytes());
        head.extend_from_slice(&[0, 0]);
        head.extend_from_slice(name.as_bytes());
        head.extend_from_slice(data);
        head
    }

    #[test]
    fn sniffs_each_format_and_lets_the_earlier_frontend_claim_an_ambiguous_zip() {
        use slidock_core_api::SniffHints;
        let registry = Frontends::default().registry().unwrap();
        let sniff = |head: &[u8], name: &str, entries: Option<&[&str]>| {
            let names: Option<Vec<String>> =
                entries.map(|entries| entries.iter().map(|e| (*e).to_owned()).collect());
            let hints = SniffHints {
                name,
                zip_names: names.as_deref(),
            };
            registry
                .sniff(head, &hints)
                .map(|frontend| frontend.format().as_str())
        };
        let zip = zip_head("a.xml", b"<a/>");
        let pptx = ["[Content_Types].xml", "ppt/presentation.xml"];
        let keynote = ["Index/Document.iwa", "Data/a.png"];
        let odp = zip_head(
            "mimetype",
            b"application/vnd.oasis.opendocument.presentation",
        );
        assert_eq!(sniff(&zip, "deck.bin", Some(&pptx)), Some("pptx"));
        assert_eq!(sniff(&odp, "deck.bin", Some(&["mimetype"])), Some("odp"));
        assert_eq!(sniff(&zip, "deck.bin", Some(&keynote)), Some("keynote"));
        assert_eq!(
            sniff(&zip, "Deck.key", Some(&["Deck.key/Index/Document.iwa"])),
            Some("keynote")
        );
        let ir = cfg!(feature = "ir-input").then_some("ir");
        assert_eq!(sniff(b"{}", "deck.presentation-ir.json", None), ir);
        // The extension decides nothing: what the bytes hold does.
        assert_eq!(sniff(&zip, "deck.pptx", Some(&["a.xml"])), None);
        assert_eq!(sniff(&zip, "deck.key", Some(&pptx)), Some("pptx"));
        let both = ["ppt/presentation.xml", "Index/Document.iwa"];
        assert_eq!(sniff(&odp, "deck.bin", Some(&both)), Some("pptx"));
        assert_eq!(
            sniff(&odp, "deck.presentation-ir.json", Some(&keynote)),
            Some("odp")
        );
    }

    #[test]
    fn options_default_to_the_whole_presentation_verified_at_the_boundaries_by_slidock() {
        let options = Options::default();
        assert_eq!(options.frontend, FrontendChoice::Auto);
        assert_eq!(options.only, None);
        assert_eq!(options.verify, VerifyMode::Boundaries);
        assert!(options.clock.is_none() && !options.trace);
        assert_eq!(options.generator, slidock_generator());
    }

    /// Without the `ir-input` feature (the wasm build), an IR document is refused as an input no frontend reads; with
    /// it, the IR frontend reads the document. The frontends every build has refuse it whatever the features, so this
    /// runs under a workspace build too, where the CLI turns the feature on.
    #[test]
    fn refuses_an_ir_document_unless_the_ir_input_is_built_in() {
        let document = format!("{{\"$schema\":\"{SCHEMA_URL}\",\"slides\":[]}}");
        let reader = BytesReader::new(document.into_bytes());
        let input = Input::new("deck.presentation-ir.json", &reader);
        let binary = FrontendRegistry::new(binary_frontends()).unwrap();
        let without_ir =
            slidock_core::compile(&input, &CompileOptions::new(&binary, Phase::Source));
        assert_eq!(
            without_ir.refusal.as_ref().map(|d| d.code.code()),
            Some("IR0902")
        );
        assert!(without_ir.source.is_none());
        assert_eq!(IR_INPUT, cfg!(feature = "ir-input"));
        let compiler = Compiler::new(Frontends::default()).unwrap();
        let compiled = compiler.compile(&input, &Options::default()).unwrap();
        let refusal = compiled.result.refusal.as_ref().map(|d| d.code.code());
        if cfg!(feature = "ir-input") {
            // The IR frontend reads it, and refuses the document for what it lacks (a size).
            assert_eq!(refusal, Some("IR0001"));
        } else {
            assert_eq!(refusal, Some("IR0902"));
            assert!(compiled.printed.is_none());
        }
    }
}
