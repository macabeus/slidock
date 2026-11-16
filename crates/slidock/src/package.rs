//! A finished session as a deck (its deck JSON and document set) and the deck bundled: the product path,
//! `open` → `finish` → [`finish_deck`] → [`bundle`](crate::bundle), which [`compile`] runs in one call.

use std::collections::BTreeMap;

use slidock_bundle::{
    BundleError, BundleOptions, BundleReport, BundleSink, CompiledDeck, DeckMedia,
};
use slidock_core::{FontSource as AddedSource, Session, SessionError, SessionOptions};
use slidock_core_api::{Deadline, Input, MediaStore};
use slidock_format::manifest::Generator;
use slidock_html::deck::{Deck, DeckInput, FontFileInfo, FontSource, build_deck};
use slidock_html::{Documents, HtmlError, MediaPlanRef};
use slidock_io::RangeReader;
use slidock_ir::diagnostics::Diagnostic;
use thiserror::Error;

use crate::driver::html_provenance;
use crate::{
    DriverError, Frontends, Options, SessionMedia, SetupError, pipeline, print_finished,
    write_media_file,
};

/// A finished session, printed: the deck JSON, its document set and every diagnostic.
#[derive(Clone, Debug, PartialEq)]
pub struct FinishedDeck {
    pub deck: Deck,
    pub documents: Documents,
    /// Every diagnostic, the session's included, in output order.
    pub diagnostics: Vec<Diagnostic>,
}

/// Why a deck could not be made.
#[derive(Debug, Error)]
pub enum CompileError {
    #[error(transparent)]
    Setup(#[from] SetupError),
    /// The input was refused, or a frontend stage failed; the diagnostics say why.
    #[error("the input cannot be opened")]
    Open(Box<slidock_core::OpenError>),
    #[error(transparent)]
    Session(#[from] SessionError),
    #[error(transparent)]
    Driver(#[from] DriverError),
    #[error(transparent)]
    Deck(#[from] HtmlError),
    /// The presentation failed as a whole: nothing prints.
    #[error("the presentation did not compile")]
    Failed { diagnostics: Vec<Diagnostic> },
    #[error(transparent)]
    Bundle(#[from] BundleError),
}

/// The deck's font source of a font file the host added.
fn deck_source(source: AddedSource) -> FontSource {
    match source {
        AddedSource::User => FontSource::User,
        AddedSource::Google => FontSource::Google,
        AddedSource::Local => FontSource::Local,
        AddedSource::Embedded => FontSource::Embedded,
        AddedSource::Bundled => FontSource::Bundled,
    }
}

/// The font files added to a session, for the deck JSON.
pub fn added_font_files(session: &Session<'_>) -> Vec<FontFileInfo> {
    session
        .added_fonts()
        .iter()
        .map(|font| FontFileInfo {
            sha256: font.sha256,
            source: deck_source(font.source),
            licence: None,
        })
        .collect()
}

/// Prints what a session finished and builds its deck JSON (with no player stamp: a bundle records the player it
/// embeds). Fails with [`CompileError::Failed`] when the presentation failed as a whole.
pub fn finish_deck(
    session: &Session<'_>,
    finished: &slidock_core::Finished,
    generator: Generator,
) -> Result<FinishedDeck, CompileError> {
    let Some(printed) = print_finished(session, finished, generator)? else {
        return Err(CompileError::Failed {
            diagnostics: finished.diagnostics.clone(),
        });
    };
    let deadline = session.deadline();
    if deadline.expired() {
        let mut diagnostics = printed.diagnostics;
        diagnostics.push(slidock_core::stage_stopped("print", &deadline, false));
        return Err(CompileError::Failed { diagnostics });
    }
    let Some(documents) = printed.printed.documents else {
        return Err(CompileError::Failed {
            diagnostics: printed.diagnostics,
        });
    };
    let deck = build_deck(&DeckInput {
        presentation: &printed.printed.presentation,
        documents: &documents,
        diagnostics: &printed.diagnostics,
        source: &html_provenance(session.provenance()),
        fonts: &added_font_files(session),
    })?;
    Ok(FinishedDeck {
        deck,
        documents,
        diagnostics: printed.diagnostics,
    })
}

/// The deck's media files, read from a media store and the input its ranges point into.
pub struct StoreMedia<'a> {
    store: &'a dyn MediaStore,
    input: &'a dyn RangeReader,
    plans: BTreeMap<String, MediaPlanRef>,
    max_pixels: u64,
}

impl std::fmt::Debug for StoreMedia<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoreMedia")
            .field("files", &self.plans.len())
            .field("max_pixels", &self.max_pixels)
            .finish_non_exhaustive()
    }
}

impl<'a> StoreMedia<'a> {
    /// The media files `documents` reference; transcoded rasters over `max_pixels` are refused.
    pub fn new(
        store: &'a dyn MediaStore,
        input: &'a dyn RangeReader,
        documents: &Documents,
        max_pixels: u64,
    ) -> StoreMedia<'a> {
        StoreMedia {
            store,
            input,
            plans: documents.media.clone(),
            max_pixels,
        }
    }
}

impl DeckMedia for StoreMedia<'_> {
    fn write_media(&self, path: &str, out: &mut dyn std::io::Write) -> Result<(), BundleError> {
        let media = |message: String| BundleError::Media {
            path: path.to_owned(),
            message,
        };
        let plan = self
            .plans
            .get(path)
            .ok_or_else(|| media("not a media file of the deck".to_owned()))?;
        write_media_file(plan, self.store, self.input, self.max_pixels, out)
            .map_err(|error| media(error.to_string()))
    }
}

/// A sink that refuses the bytes once `deadline` passes, so a bundle stops between its writes.
struct Within<'s, 'd> {
    sink: &'s mut dyn BundleSink,
    deadline: Deadline<'d>,
}

impl BundleSink for Within<'_, '_> {
    fn write(&mut self, path: &str, bytes: &[u8]) -> Result<(), BundleError> {
        if self.deadline.expired() {
            return Err(BundleError::Sink(format!(
                "bundle stopped: over the compile's {} ms limit",
                self.deadline.limit_ms()
            )));
        }
        self.sink.write(path, bytes)
    }
}

/// [`slidock_bundle::bundle`] within the compile's deadline ([`Session::deadline`]): checked before every write, so
/// reading and transcoding media count too.
pub fn bundle_within(
    deck: &CompiledDeck<'_>,
    options: &BundleOptions,
    sink: &mut dyn BundleSink,
    deadline: Deadline<'_>,
) -> Result<BundleReport, BundleError> {
    slidock_bundle::bundle(deck, options, &mut Within { sink, deadline })
}

/// What [`compile`] made.
#[derive(Clone, Debug, PartialEq)]
pub struct Bundled {
    /// The deck JSON as `index.html` holds it.
    pub deck: Deck,
    pub report: BundleReport,
    /// Every diagnostic, in output order.
    pub diagnostics: Vec<Diagnostic>,
}

/// One input through its frontend, every pass, the backend and the bundler, into `sink`: `open`, `finish` with the
/// catalog's fonts (none of the input's embedded fonts is added), [`finish_deck`], then [`bundle`](crate::bundle).
pub fn compile(
    input: &Input<'_>,
    frontends: Frontends,
    options: &Options<'_>,
    bundle_options: &BundleOptions,
    sink: &mut dyn BundleSink,
) -> Result<Bundled, CompileError> {
    let registry = frontends.registry().map_err(SetupError::from)?;
    let pipeline = pipeline().map_err(SetupError::from)?;
    let session_options = SessionOptions {
        frontend: options.frontend,
        only: options.only,
        verify: options.verify,
        limits: options.limits,
        fonts: options.fonts,
        policy: options.policy,
        clock: options.clock,
        trace: options.trace,
        native_motion: options.native_motion,
        ..SessionOptions::new(&registry, &pipeline)
    };
    let mut session = Session::open(input, session_options).map_err(CompileError::Open)?;
    let finished = session.finish()?;
    let FinishedDeck {
        deck,
        documents,
        diagnostics,
    } = finish_deck(&session, &finished, options.generator.clone())?;
    let store = SessionMedia(&session);
    let media = StoreMedia::new(
        &store,
        input.reader,
        &documents,
        options.policy.transcode_pixels,
    );
    let compiled = CompiledDeck {
        deck,
        documents,
        media: &media,
        fonts: Vec::new(),
    };
    let report = bundle_within(&compiled, bundle_options, sink, session.deadline())?;
    Ok(Bundled {
        deck: slidock_bundle::bundled_deck(&compiled, bundle_options)?,
        report,
        diagnostics,
    })
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    struct Kept(Vec<String>);

    impl BundleSink for Kept {
        fn write(&mut self, path: &str, _bytes: &[u8]) -> Result<(), BundleError> {
            self.0.push(path.to_owned());
            Ok(())
        }
    }

    #[test]
    fn a_bundle_writes_nothing_once_the_compile_deadline_passes() {
        let now = Cell::new(0.0);
        let clock = || now.get();
        let mut kept = Kept(Vec::new());
        let mut within = Within {
            sink: &mut kept,
            deadline: Deadline::start(Some(&clock), 10),
        };
        assert!(within.write("index.html", b"a").is_ok());
        now.set(10.0);
        let refused = within.write("media/x.png", b"b").unwrap_err();
        assert_eq!(
            refused.to_string(),
            "bundle stopped: over the compile's 10 ms limit"
        );
        assert_eq!(kept.0, ["index.html"]);
    }
}
