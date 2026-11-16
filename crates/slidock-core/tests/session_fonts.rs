//! The Session's font loop: `finish` re-runs from bind-fonts with the fonts added since and gives what one finish
//! with every font gives, parsing once; the source's embedded fonts are read lazily, within limits, and must prove
//! their identity; faces resolve in the user's order.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use std::borrow::Cow;
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::json;
use slidock_core::{
    CollectGlyphs, FontIssue, FontSource, FontUnwrap, FrontendRegistry, GlyphUse, Pipeline,
    Session, SessionOptions, SubstituteFace, UnwrappedFont,
};
use slidock_core_api::frontend::FontContainer;
use slidock_core_api::{EmbeddedFont, FrontendError, Input, MediaSource, Pass};
use slidock_fonts::resolve::{Fill, Mismatch, SourceKind, SourceOrder};
use slidock_fonts::{FamilyKey, FontCatalog};
use slidock_io::{BytesReader, RangeError, RangeReader};
use slidock_ir::fonts::{FaceRequest, FaceStyle, FontFile};
use support::*;

const FAKE: &[u8] = b"FAKE presentation";

fn reader() -> BytesReader<Vec<u8>> {
    BytesReader::new(FAKE.to_vec())
}

/// After fonts: every recorded family's binding takes the metrics and file of the catalog's regular face, so the
/// output shows which face resolution chose.
fn binder() -> Pass {
    presentation_pass("binder", |presentation, ctx| {
        let mut out = presentation.clone();
        for entry in out.fonts.values_mut() {
            let Some(found) = ctx.fonts.find_family(&entry.request.family) else {
                continue;
            };
            let Some(face) = found.family.css_match(400, FaceStyle::Normal) else {
                continue;
            };
            if let Some(binding) = entry.binding.as_mut() {
                binding.metrics = face.metrics.clone();
                binding.file = face.sha256.map(|sha256| FontFile {
                    sha256,
                    postscript_name: face.postscript_name.clone(),
                });
            }
        }
        Ok(out)
    })
}

fn pipeline() -> Pipeline {
    Pipeline::new(vec![claim_all(&[])], vec![binder()]).unwrap()
}

fn canonical(finished: &slidock_core::Finished) -> String {
    slidock_ir::canonical::to_json(
        &json!({ "presentation": finished.presentation, "diagnostics": finished.diagnostics }),
    )
    .unwrap()
}

/// A frontend that hands on the fixture and these embedded fonts, counting its lowerings.
struct Embedding {
    inner: FakeFrontend,
    fonts: Vec<EmbeddedFont>,
    lowered: Rc<Cell<usize>>,
}

impl slidock_core_api::Frontend for Embedding {
    fn format(&self) -> slidock_ir::formats::SourceFormat {
        self.inner.format()
    }

    fn version(&self) -> &str {
        self.inner.version()
    }

    fn sniff(&self, head: &[u8], hints: &slidock_core_api::SniffHints<'_>) -> bool {
        self.inner.sniff(head, hints)
    }

    fn analyze(
        &self,
        input: &Input<'_>,
        ctx: &mut slidock_core_api::FrontendCtx<'_>,
    ) -> Result<slidock_core_api::Analysis, FrontendError> {
        self.inner.analyze(input, ctx)
    }

    fn lower(
        &self,
        input: &Input<'_>,
        ctx: &mut slidock_core_api::FrontendCtx<'_>,
        options: &slidock_core_api::LowerOptions<'_>,
    ) -> Result<slidock_core_api::LowerResult, FrontendError> {
        self.lowered.set(self.lowered.get() + 1);
        let mut lowered = self.inner.lower(input, ctx, options)?;
        lowered.fonts = self.fonts.clone();
        Ok(lowered)
    }
}

fn registry_with(
    presentation: slidock_ir::source::Presentation,
    fonts: Vec<EmbeddedFont>,
) -> (FrontendRegistry, Rc<Cell<usize>>) {
    let lowered = Rc::new(Cell::new(0));
    let frontend = Embedding {
        inner: FakeFrontend::new(presentation),
        fonts,
        lowered: Rc::clone(&lowered),
    };
    (
        FrontendRegistry::new(vec![Box::new(frontend)]).unwrap(),
        lowered,
    )
}

fn embedded(
    family: &str,
    weight: u32,
    style: FaceStyle,
    container: FontContainer,
    source: MediaSource,
) -> EmbeddedFont {
    EmbeddedFont {
        family: family.to_owned(),
        faces: vec![FaceRequest { weight, style }],
        container,
        source,
    }
}

fn bytes(data: Vec<u8>) -> MediaSource {
    MediaSource::Bytes(Arc::from(data))
}

/// The ascender of the face the binder bound Lato to.
fn lato_ascender(finished: &slidock_core::Finished) -> i32 {
    let presentation = finished.presentation.as_ref().unwrap();
    presentation.fonts["lato"].binding.metrics.hhea.ascender
}

#[test]
fn finishing_again_after_more_fonts_equals_one_finish_with_every_font_and_parses_once() {
    let pipeline = pipeline();
    let reader = reader();
    let (registry, lowered) = registry_with(full(), Vec::new());
    let mut rounds = Session::open(
        &Input::new("presentation.fake", &reader),
        SessionOptions::new(&registry, &pipeline),
    )
    .unwrap();
    let first = rounds.finish().unwrap();
    rounds
        .add_font(font("Lato", 400, false, 700), FontSource::User)
        .unwrap();
    let second = rounds.finish().unwrap();
    rounds
        .add_font(font("Lato", 400, false, 800), FontSource::Google)
        .unwrap();
    let third = rounds.finish().unwrap();
    assert_eq!(lowered.get(), 1, "parsing ran once");

    let (registry, lowered) = registry_with(full(), Vec::new());
    let mut once = Session::open(
        &Input::new("presentation.fake", &reader),
        SessionOptions::new(&registry, &pipeline),
    )
    .unwrap();
    once.add_font(font("Lato", 400, false, 700), FontSource::User)
        .unwrap();
    once.add_font(font("Lato", 400, false, 800), FontSource::Google)
        .unwrap();
    let single = once.finish().unwrap();
    assert_eq!(lowered.get(), 1);

    assert_eq!(canonical(&third), canonical(&single));
    // The rounds did bind differently: the built-in metrics, the dropped file, then Google's (before dropped files).
    assert_ne!(lato_ascender(&first), 700);
    assert_eq!(lato_ascender(&second), 700);
    assert_eq!(lato_ascender(&third), 800);
}

#[test]
fn embedded_fonts_are_taken_only_when_on() {
    let pipeline = pipeline();
    let reader = reader();
    let fonts = vec![embedded(
        "Lato",
        400,
        FaceStyle::Normal,
        FontContainer::Sfnt,
        bytes(font("Lato", 400, false, 900)),
    )];
    let (registry, _) = registry_with(full(), fonts);
    let off = Session::open(
        &Input::new("presentation.fake", &reader),
        SessionOptions::new(&registry, &pipeline),
    )
    .unwrap();
    assert_eq!(off.embedded_fonts().len(), 1);
    assert!(off.added_fonts().is_empty());
    assert!(off.font_needs().iter().any(|need| need.family == "Lato"));

    let mut on = Session::open(
        &Input::new("presentation.fake", &reader),
        SessionOptions {
            embedded_fonts: true,
            ..SessionOptions::new(&registry, &pipeline)
        },
    )
    .unwrap();
    assert_eq!(on.added_fonts().len(), 1);
    assert_eq!(on.added_fonts()[0].source, FontSource::Embedded);
    assert!(on.font_needs().iter().all(|need| need.family != "Lato"));
    assert_eq!(lato_ascender(&on.finish().unwrap()), 900);
    // The embedded face comes first in the default order, a host's later Google face does not replace it.
    on.add_font(font("Lato", 400, false, 600), FontSource::Google)
        .unwrap();
    assert_eq!(lato_ascender(&on.finish().unwrap()), 900);
}

/// A reader that counts its reads.
struct Counting {
    bytes: Vec<u8>,
    reads: Arc<AtomicUsize>,
}

impl RangeReader for Counting {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn read_at(&self, offset: u64, len: usize) -> Result<Cow<'_, [u8]>, RangeError> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        let start = offset as usize;
        self.bytes
            .get(start..start + len)
            .map(Cow::Borrowed)
            .ok_or(RangeError::Invalid {
                offset,
                len: len as u64,
            })
    }
}

#[test]
fn an_embedded_font_is_read_on_first_use_once_and_within_limits() {
    let pipeline = pipeline();
    let reader = reader();
    let reads = Arc::new(AtomicUsize::new(0));
    let deferred = MediaSource::Reader(Arc::new(Counting {
        bytes: font("Lato", 400, false, 900),
        reads: Arc::clone(&reads),
    }));
    let large = vec![0u8; 4096];
    let fonts = vec![
        embedded(
            "Lato",
            400,
            FaceStyle::Normal,
            FontContainer::Sfnt,
            deferred,
        ),
        embedded(
            "Big",
            400,
            FaceStyle::Normal,
            FontContainer::Sfnt,
            bytes(large),
        ),
        embedded(
            "Woff",
            400,
            FaceStyle::Normal,
            FontContainer::Woff2,
            bytes(b"wOF2 that is no font".to_vec()),
        ),
        // The container the source declares is a hint: the bytes decide.
        embedded(
            "Mislabeled",
            400,
            FaceStyle::Normal,
            FontContainer::Eot,
            bytes(font("Mislabeled", 400, false, 900)),
        ),
    ];
    let (registry, _) = registry_with(full(), fonts);
    let mut limits = slidock_core_api::Limits::default();
    limits.zip.entry_bytes = 2048;
    let mut session = Session::open(
        &Input::new("presentation.fake", &reader),
        SessionOptions {
            embedded_fonts: true,
            limits,
            ..SessionOptions::new(&registry, &pipeline)
        },
    )
    .unwrap();
    assert_eq!(reads.load(Ordering::Relaxed), 0, "nothing is read at open");
    let _ = session.font_needs();
    assert_eq!(reads.load(Ordering::Relaxed), 1);
    session.finish().unwrap();
    let finished = session.finish().unwrap();
    assert_eq!(reads.load(Ordering::Relaxed), 1, "read once");
    assert_eq!(
        finished.font_issues,
        vec![
            FontIssue::EmbeddedTooLarge {
                family: "Big".into(),
                bytes: 4096,
                limit: 2048
            },
            FontIssue::EmbeddedUnreadable {
                family: "Woff".into(),
                reason: "The header is truncated".into()
            },
        ]
    );
    let read: Vec<&str> = session
        .added_fonts()
        .iter()
        .map(|font| font.faces[0].family.as_str())
        .collect();
    assert_eq!(read, ["Lato", "Mislabeled"]);
}

/// The fixture with a recorded "Roboto Medium" (embedded, regular) and no binding for it yet.
fn with_roboto_medium() -> slidock_ir::source::Presentation {
    let mut presentation = full_json();
    presentation["fonts"]["robotomedium"] = json!({ "request": { "family": "Roboto Medium",
        "faces": [{ "weight": 400, "style": "normal" }], "postscriptNames": [], "embedded": true } });
    typed(presentation)
}

#[test]
fn a_regular_file_embedded_as_medium_is_demoted_and_google_medium_is_planned() {
    let pipeline = pipeline();
    let reader = reader();
    let fonts = vec![embedded(
        "Roboto Medium",
        400,
        FaceStyle::Normal,
        FontContainer::Sfnt,
        bytes(font("Roboto", 400, false, 900)),
    )];
    let (registry, _) = registry_with(with_roboto_medium(), fonts);
    let mut session = Session::open(
        &Input::new("presentation.fake", &reader),
        SessionOptions {
            embedded_fonts: true,
            ..SessionOptions::new(&registry, &pipeline)
        },
    )
    .unwrap();
    assert_eq!(
        session.font_issues(),
        vec![FontIssue::EmbeddedMislabeled {
            family: "Roboto Medium".into(),
            weight: 400,
            style: FaceStyle::Normal,
            file_family: "Roboto".into(),
            file_weight: 400,
            file_italic: false,
            mismatch: Mismatch::Weight { file_weight: 400 },
        }]
    );
    assert_eq!(
        session.font_issues()[0].to_string(),
        "Roboto Medium 400: the file is Roboto 400 (another weight); used only if no source has the face"
    );
    let need = session
        .font_needs()
        .into_iter()
        .find(|need| need.key == FamilyKey::of("Roboto Medium"))
        .unwrap();
    assert!(matches!(need.slots[0].fill, Fill::Demoted { .. }));
    assert_eq!(need.slots[0].weight, 500);
    let plan = need.google.unwrap();
    assert_eq!(
        plan.url,
        "https://fonts.googleapis.com/css2?family=Roboto:wght@500&display=block"
    );
    assert_eq!(
        need.local.unwrap().families,
        [FamilyKey::of("Roboto Medium"), FamilyKey::of("Roboto")]
    );
    // The fetched Medium fills the slot.
    session
        .add_font(font("Roboto", 500, false, 950), FontSource::Google)
        .unwrap();
    assert!(
        session
            .font_needs()
            .iter()
            .all(|need| need.key != FamilyKey::of("Roboto Medium"))
    );
}

#[test]
fn the_user_s_order_decides_and_a_source_left_out_is_ignored() {
    let pipeline = pipeline();
    let reader = reader();
    let (registry, _) = registry_with(full(), Vec::new());
    let bound = |order: SourceOrder| {
        let mut session = Session::open(
            &Input::new("presentation.fake", &reader),
            SessionOptions {
                font_order: order,
                ..SessionOptions::new(&registry, &pipeline)
            },
        )
        .unwrap();
        session
            .add_font(font("Lato", 400, false, 700), FontSource::User)
            .unwrap();
        session
            .add_font(font("Lato", 400, false, 800), FontSource::Local)
            .unwrap();
        lato_ascender(&session.finish().unwrap())
    };
    assert_eq!(bound(SourceOrder::default()), 800);
    assert_eq!(
        bound(SourceOrder::new([
            SourceKind::UserDropped,
            SourceKind::ViewerInstalled
        ])),
        700
    );
    assert_eq!(bound(SourceOrder::new([SourceKind::UserDropped])), 700);
    let off = bound(SourceOrder::off());
    assert!(off != 700 && off != 800, "{off}");
}

/// Takes every container as a plain sfnt; an EOT is a subset file.
struct SubsetEot;

impl FontUnwrap for SubsetEot {
    fn unwrap_font(&self, bytes: &[u8], container: FontContainer) -> Result<UnwrappedFont, String> {
        Ok(UnwrappedFont {
            sfnt: bytes.to_vec(),
            subset: container == FontContainer::Eot,
        })
    }
}

/// Printable ASCII but `left_out`.
fn ascii_but(left_out: &str) -> Vec<u32> {
    (0x21..0x7f)
        .filter(|&point| !left_out.contains(char::from_u32(point).unwrap()))
        .collect()
}

#[test]
fn a_subset_embedded_file_must_hold_every_code_point_drawn_with_its_face() {
    let pipeline = pipeline();
    let reader = reader();
    let bound = |points: Vec<u32>, container: FontContainer| {
        let file = font_with("Lato", 400, false, 900, vec![(*b"cmap", cmap(&points))]);
        let fonts = vec![embedded(
            "Lato",
            400,
            FaceStyle::Normal,
            container,
            bytes(file),
        )];
        let (registry, _) = registry_with(full(), fonts);
        let mut session = Session::open(
            &Input::new("presentation.fake", &reader),
            SessionOptions {
                embedded_fonts: true,
                unwrap: &SubsetEot,
                ..SessionOptions::new(&registry, &pipeline)
            },
        )
        .unwrap();
        let issues = session.font_issues();
        let alone = lato_ascender(&session.finish().unwrap());
        session
            .add_font(font("Lato", 400, false, 600), FontSource::Google)
            .unwrap();
        (issues, alone, lato_ascender(&session.finish().unwrap()))
    };
    // The text draws "Master title", "Presentation IR", …: a subset without `M` and `P` lacks two of them.
    let (issues, alone, with_google) = bound(ascii_but("MP"), FontContainer::Eot);
    assert_eq!(
        issues,
        vec![FontIssue::EmbeddedSubset {
            family: "Lato".into(),
            weight: 400,
            style: FaceStyle::Normal,
            missing: 2,
        }]
    );
    assert_eq!(
        alone, 900,
        "with no other source, the subset file stands in"
    );
    assert_eq!(with_google, 600, "a source that has the face comes first");
    // Negative controls: a subset that holds every drawn code point, and a whole file missing the same two, are
    // accepted and come before Google's face.
    for (points, container) in [
        (ascii_but(""), FontContainer::Eot),
        (ascii_but("MP"), FontContainer::Sfnt),
    ] {
        let (issues, alone, with_google) = bound(points, container);
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!((alone, with_google), (900, 900));
    }
}

#[test]
fn installed_files_count_against_the_budget_of_a_session_that_ships_fonts() {
    let pipeline = pipeline();
    let reader = reader();
    let (registry, _) = registry_with(full(), Vec::new());
    let glyphs = Uses(Vec::new());
    let open = |ships: bool| {
        Session::open(
            &Input::new("presentation.fake", &reader),
            SessionOptions {
                glyphs: ships.then_some(&glyphs as &dyn CollectGlyphs),
                ..SessionOptions::new(&registry, &pipeline)
            },
        )
        .unwrap()
    };
    let mut session = open(true);
    for _ in 0..slidock_fonts::resolve::MAX_LOCAL_PICKS {
        session
            .add_font(font("Lato", 400, false, 800), FontSource::Local)
            .unwrap();
    }
    assert!(matches!(
        session.add_font(font("Lato", 400, false, 800), FontSource::Local),
        Err(slidock_core::SessionError::LocalRefused(
            slidock_fonts::resolve::LocalRefusal::TooManyFiles
        ))
    ));
    // Negative control: other sources are not counted.
    session
        .add_font(font("Lato", 400, false, 800), FontSource::User)
        .unwrap();

    // A session that ships nothing (the parity path, the determinism gate's wasm leg) takes installed files as
    // catalog fonts, as the CLI's --fonts folders: no budget.
    let mut parity = open(false);
    for _ in 0..=2 * slidock_fonts::resolve::MAX_LOCAL_PICKS {
        parity
            .add_font(font("Lato", 400, false, 800), FontSource::Local)
            .unwrap();
    }
    let finished = parity.finish().unwrap();
    assert_eq!(lato_ascender(&finished), 800);
}

#[test]
fn an_embedded_font_s_bytes_are_dropped_once_read_and_count_against_the_budget() {
    let pipeline = pipeline();
    let reader = reader();
    let file = font("Lato", 400, false, 900);
    let size = file.len() as u64;
    let source: Arc<dyn RangeReader> = Arc::new(Counting {
        bytes: file.clone(),
        reads: Arc::new(AtomicUsize::new(0)),
    });
    let weak = Arc::downgrade(&source);
    let fonts = vec![embedded(
        "Lato",
        400,
        FaceStyle::Normal,
        FontContainer::Sfnt,
        MediaSource::Reader(source),
    )];
    // The frontend keeps its own copy of what it hands on.
    let (registry, _) = registry_with(full(), fonts);
    let open = |embedded_fonts: bool, memory_bytes: u64| {
        let mut limits = slidock_core_api::Limits::default();
        limits.zip.memory_bytes = memory_bytes;
        Session::open(
            &Input::new("presentation.fake", &reader),
            SessionOptions {
                embedded_fonts,
                limits,
                ..SessionOptions::new(&registry, &pipeline)
            },
        )
        .unwrap()
    };
    let off = open(false, u64::MAX);
    assert_eq!(
        weak.strong_count(),
        1,
        "a session that takes no embedded font keeps none"
    );
    drop(off);
    let on = open(true, u64::MAX);
    assert_eq!(weak.strong_count(), 2);
    assert_eq!(on.added_fonts().len(), 1);
    assert_eq!(weak.strong_count(), 1, "the bytes are dropped once read");
    drop(on);
    // A reader's bytes count twice while read, then the file kept: three times the file fits, less does not.
    assert_eq!(open(true, 3 * size).added_fonts().len(), 1);
    let short = open(true, 3 * size - 1);
    assert!(short.added_fonts().is_empty());
    assert!(matches!(
        short.font_issues()[..],
        [FontIssue::EmbeddedTooLarge { .. }]
    ));
}

/// A browser's `css2` response for Lato 400: one rule per script chunk.
const LATO_CHUNKS: &str = "/* cyrillic */
@font-face { font-family: 'Lato'; font-style: normal; font-weight: 400; font-display: block;
  src: url(https://fonts.gstatic.com/s/lato/v24/cyrillic.woff2) format('woff2'); unicode-range: U+0400-045F; }
/* latin-ext */
@font-face { font-family: 'Lato'; font-style: normal; font-weight: 400; font-display: block;
  src: url(https://fonts.gstatic.com/s/lato/v24/latin-ext.woff2) format('woff2'); unicode-range: U+0100-02BA; }
/* latin */
@font-face { font-family: 'Lato'; font-style: normal; font-weight: 400; font-display: block;
  src: url(https://fonts.gstatic.com/s/lato/v24/latin.woff2) format('woff2'); unicode-range: U+0000-00FF; }
";

#[test]
fn of_a_browser_s_google_response_only_the_chunks_the_text_draws_from_are_fetched() {
    let pipeline = pipeline();
    let reader = reader();
    let (registry, _) = registry_with(full(), Vec::new());
    let session = Session::open(
        &Input::new("presentation.fake", &reader),
        SessionOptions::new(&registry, &pipeline),
    )
    .unwrap();
    let need = session
        .font_needs()
        .into_iter()
        .find(|need| need.key == FamilyKey::of("Lato"))
        .unwrap();
    assert!(need.google.is_some());
    let urls = |css: &str| -> Vec<String> {
        session
            .google_faces(css)
            .unwrap()
            .into_iter()
            .map(|face| face.url)
            .collect()
    };
    assert_eq!(
        urls(LATO_CHUNKS),
        ["https://fonts.gstatic.com/s/lato/v24/latin.woff2"]
    );
    // A response without ranges (static files) is kept whole; a family the deck never asked Google for is not.
    let whole = "@font-face { font-family: 'Lato'; font-style: normal; font-weight: 400;
      src: url(https://fonts.gstatic.com/s/lato/v24/Lato-Regular.ttf) format('truetype'); }";
    assert_eq!(
        urls(whole),
        ["https://fonts.gstatic.com/s/lato/v24/Lato-Regular.ttf"]
    );
    assert_eq!(
        urls(&LATO_CHUNKS.replace("'Lato'", "'Oswald'")),
        Vec::<String>::new()
    );
}

/// A glyph collector that reports these uses, whatever the deck draws.
struct Uses(Vec<GlyphUse>);

impl CollectGlyphs for Uses {
    fn collect(
        &self,
        _presentation: &slidock_ir::resolved::Presentation,
        _catalog: &FontCatalog,
    ) -> Result<Vec<GlyphUse>, String> {
        Ok(self.0.clone())
    }
}

/// The face the catalog matches for `family` at `weight`: its PostScript name and whether a file provides it.
fn matched(catalog: &FontCatalog, family: &str, weight: u32) -> (String, bool) {
    let found = catalog.find_family(family).unwrap();
    let face = found.family.css_match(weight, FaceStyle::Normal).unwrap();
    (face.postscript_name.clone(), face.sha256.is_some())
}

#[test]
fn a_session_that_ships_fonts_measures_with_files_over_the_bundled_floor() {
    let pipeline = pipeline();
    let reader = reader();
    let fonts = vec![embedded(
        "Roboto Medium",
        400,
        FaceStyle::Normal,
        FontContainer::Sfnt,
        bytes(font("Roboto", 400, false, 900)),
    )];
    let (registry, _) = registry_with(with_roboto_medium(), fonts);
    let glyphs = Uses(Vec::new());
    let open = |ships: bool| {
        Session::open(
            &Input::new("presentation.fake", &reader),
            SessionOptions {
                embedded_fonts: true,
                glyphs: ships.then_some(&glyphs as &dyn CollectGlyphs),
                ..SessionOptions::new(&registry, &pipeline)
            },
        )
        .unwrap()
    };
    let mut shipping = open(true);
    let catalog = shipping.catalog();
    // The demoted Regular draws Roboto Medium: no source has the Medium, and the built-in metrics-only Medium is no
    // file.
    assert_eq!(
        matched(&catalog, "Roboto Medium", 500),
        ("Roboto-Regular".to_owned(), true)
    );
    // Of the base fonts only the bundled families stay, metrics only until their files arrive.
    assert!(catalog.get(&FamilyKey::of("Oswald")).is_none());
    assert_eq!(
        matched(&catalog, "Liberation Sans", 400),
        ("LiberationSans".to_owned(), false)
    );
    // Caladea, Cambria's substitute, is in the floor too.
    assert_eq!(
        matched(&catalog, "Caladea", 700),
        ("Caladea-Bold".to_owned(), false)
    );
    // A bundled file replaces its family's metrics-only faces, and fills no slot of a family the deck asks for.
    shipping
        .add_font(
            font("Liberation Sans", 400, false, 905),
            FontSource::Bundled,
        )
        .unwrap();
    shipping
        .add_font(font("Lato", 400, false, 905), FontSource::Bundled)
        .unwrap();
    let catalog = shipping.catalog();
    let liberation = catalog.get(&FamilyKey::of("Liberation Sans")).unwrap();
    assert_eq!(liberation.faces.len(), 1);
    assert_eq!(
        matched(&catalog, "Liberation Sans", 700),
        ("LiberationSans-Regular".to_owned(), true)
    );
    assert!(
        shipping
            .font_needs()
            .iter()
            .any(|need| need.family == "Lato")
    );

    // Without shipping (the parity path) the base fonts all stay, the built-in Medium included.
    let parity = open(false);
    let catalog = parity.catalog();
    assert_eq!(
        matched(&catalog, "Roboto Medium", 500),
        ("Roboto-Medium".to_owned(), false)
    );
    assert!(catalog.get(&FamilyKey::of("Oswald")).is_some());
    // The parity path binds as the built-in table does: Caladea is not in it.
    assert!(catalog.get(&FamilyKey::of("Caladea")).is_none());
}

#[test]
fn finish_names_the_bundled_faces_drawn_with_no_file() {
    let pipeline = pipeline();
    let reader = reader();
    let (registry, _) = registry_with(full(), Vec::new());
    let face = |family: &str, postscript_name: &str, italic: bool| GlyphUse {
        family: family.to_owned(),
        postscript_name: postscript_name.to_owned(),
        weight: 400,
        italic,
        ..GlyphUse::default()
    };
    let glyphs = Uses(vec![
        face("Liberation Sans", "LiberationSans-Italic", true),
        face("Oswald", "Oswald-Regular", false),
        face("Liberation Sans", "LiberationSans-Italic", true),
        GlyphUse {
            sha256: Some(slidock_ir::ids::Sha256::of(b"a file")),
            ..face("Carlito", "Carlito-Regular", false)
        },
        face("DejaVu Sans", "DejaVuSans", false),
    ]);
    let mut session = Session::open(
        &Input::new("presentation.fake", &reader),
        SessionOptions {
            glyphs: Some(&glyphs),
            ..SessionOptions::new(&registry, &pipeline)
        },
    )
    .unwrap();
    let finished = session.finish().unwrap();
    assert_eq!(
        finished.substitutes,
        [
            SubstituteFace {
                family: "Liberation Sans".into(),
                postscript_name: "LiberationSans-Italic".into(),
                weight: 400,
                style: FaceStyle::Italic,
            },
            SubstituteFace {
                family: "DejaVu Sans".into(),
                postscript_name: "DejaVuSans".into(),
                weight: 400,
                style: FaceStyle::Normal,
            },
        ]
    );
}

#[test]
fn finish_names_the_last_resort_face_when_a_face_waiting_for_its_file_draws_past_the_bundled_coverage()
 {
    let pipeline = pipeline();
    let reader = reader();
    let (registry, _) = registry_with(full(), Vec::new());
    let last_resort = SubstituteFace {
        family: "DejaVu Sans".into(),
        postscript_name: "DejaVuSans".into(),
        weight: 400,
        style: FaceStyle::Normal,
    };
    let substitutes = |text: &str, forced: &[u32]| {
        let mut codepoints = slidock_fonts::CodepointSet::new();
        codepoints.insert_text(text);
        let glyphs = Uses(vec![GlyphUse {
            family: "Liberation Sans".to_owned(),
            postscript_name: "LiberationSans".to_owned(),
            weight: 400,
            forced: forced.iter().copied().collect(),
            codepoints,
            ..GlyphUse::default()
        }]);
        let mut session = Session::open(
            &Input::new("presentation.fake", &reader),
            SessionOptions {
                glyphs: Some(&glyphs),
                ..SessionOptions::new(&registry, &pipeline)
            },
        )
        .unwrap();
        let first = session.finish().unwrap().substitutes;
        // Once the session holds the file, it is not asked for again.
        session
            .add_font(
                slidock_substitutes::FACES
                    .iter()
                    .find(|face| face.file == "DejaVuSans.woff2")
                    .unwrap()
                    .bytes
                    .to_vec(),
                FontSource::Bundled,
            )
            .unwrap();
        let again = session.finish().unwrap().substitutes;
        (first, again)
    };
    let liberation = SubstituteFace {
        family: "Liberation Sans".into(),
        postscript_name: "LiberationSans".into(),
        weight: 400,
        style: FaceStyle::Normal,
    };
    // ❖ is outside what every bundled face maps.
    let (first, again) = substitutes("Agenda \u{2756}", &[]);
    assert_eq!(first, [liberation.clone(), last_resort.clone()]);
    assert_eq!(again, std::slice::from_ref(&liberation));
    // A U+2010 hyphen the text draws is outside it too (Caladea lacks it).
    let (first, _) = substitutes("Café\u{2010}", &[]);
    assert_eq!(first, [liberation.clone(), last_resort]);
    // Negative control: Latin-1 text, a hyphen the face keeps that the text does not draw and a zero-width space
    // need no last resort.
    let (first, _) = substitutes("Café\u{2010}\u{200b}", &[0x2010]);
    assert_eq!(first, [liberation]);
}

/// Barlow Condensed Regular (OFL-1.1, slidock-fonts' fixtures).
const BARLOW: &[u8] = include_bytes!("../../slidock-fonts/tests/data/container/barlow.ttf");

/// `font` without its `OS/2` table, as old Apple TrueType fonts are (checksums left stale: nothing here checks them).
fn without_os2(font: &[u8]) -> Vec<u8> {
    let be16 = |at: usize| u16::from_be_bytes([font[at], font[at + 1]]) as usize;
    let be32 = |at: usize| u32::from_be_bytes(font[at..at + 4].try_into().unwrap()) as usize;
    let records: Vec<([u8; 4], usize, usize, usize)> = (0..be16(4))
        .map(|i| 12 + i * 16)
        .map(|r| {
            (
                font[r..r + 4].try_into().unwrap(),
                be32(r + 4),
                be32(r + 8),
                be32(r + 12),
            )
        })
        .filter(|(tag, ..)| tag != b"OS/2")
        .collect();
    let mut out = font[..12].to_vec();
    out[4..6].copy_from_slice(&(records.len() as u16).to_be_bytes());
    let mut data = Vec::new();
    let start = 12 + 16 * records.len();
    for (tag, checksum, offset, length) in &records {
        out.extend(tag);
        out.extend((*checksum as u32).to_be_bytes());
        out.extend(((start + data.len()) as u32).to_be_bytes());
        out.extend((*length as u32).to_be_bytes());
        data.extend(&font[*offset..offset + length]);
        data.resize(data.len().next_multiple_of(4), 0);
    }
    out.extend(data);
    out
}

/// The fixture asking for Barlow Condensed (by its PostScript name too) and Apple Color Emoji.
fn with_barlow_and_emoji() -> slidock_ir::source::Presentation {
    let mut presentation = full_json();
    presentation["fonts"]["barlowcondensed"] = json!({ "request": { "family": "Barlow Condensed",
        "faces": [{ "weight": 400, "style": "normal" }], "postscriptNames": ["BarlowCondensed-Regular"],
        "embedded": false } });
    presentation["fonts"]["applecoloremoji"] = json!({ "request": { "family": "Apple Color Emoji",
        "faces": [{ "weight": 400, "style": "normal" }], "postscriptNames": ["AppleColorEmoji"],
        "embedded": false } });
    typed(presentation)
}

#[test]
fn a_face_without_os2_is_repaired_and_binds_as_restricted() {
    let pipeline = pipeline();
    let reader = reader();
    let (registry, _) = registry_with(with_barlow_and_emoji(), Vec::new());
    let glyphs = Uses(Vec::new());
    let open = |target: slidock_fonts::Target| {
        Session::open(
            &Input::new("presentation.fake", &reader),
            SessionOptions {
                glyphs: Some(&glyphs as &dyn CollectGlyphs),
                target,
                ..SessionOptions::new(&registry, &pipeline)
            },
        )
        .unwrap()
    };
    let old = without_os2(BARLOW);
    assert!(slidock_fonts::sfnt::sanitizer_refusal(&old, 0).is_some());

    // A preview draws with the repaired face, and says the download leaves it out.
    let mut preview = open(slidock_fonts::Target::Preview);
    preview.add_font(old.clone(), FontSource::Local).unwrap();
    let added = preview.added_fonts();
    assert_eq!(added[0].repairs, [Some(slidock_fonts::FaceRepair::Os2)]);
    assert!(slidock_fonts::sfnt::sanitizer_refusal(&added[0].bytes, 0).is_none());
    assert_eq!(preview.refusal(added[0], 0), None);
    assert_eq!(
        matched(&preview.catalog(), "Barlow Condensed", 400),
        ("BarlowCondensed-Regular".to_owned(), true)
    );
    let issues = preview.font_issues();
    assert_eq!(issues.len(), 1, "{issues:?}");
    assert_eq!(issues[0].code(), "MID5420");
    assert_eq!(
        issues[0].to_string(),
        "Barlow Condensed (BarlowCondensed-Regular, local): it has no OS/2 table, which browsers require; slidock \
         wrote one from its other tables so they load it"
    );

    // A download leaves it out: its licence is unknown, so restricted.
    let mut download = open(slidock_fonts::Target::Download);
    download.add_font(old, FontSource::Local).unwrap();
    let issues = download.font_issues();
    assert!(matches!(
        &issues[1],
        FontIssue::LicenceExcluded {
            unstated: true,
            in_preview: false,
            ..
        }
    ));
    assert_eq!(
        issues[1].to_string(),
        "Barlow Condensed (BarlowCondensed-Regular, local): the face states no embedding permissions (it has no OS/2 \
         table), so it is treated as restricted; its substitute draws instead"
    );
    let catalog = download.catalog();
    assert!(catalog.get(&FamilyKey::of("Barlow Condensed")).is_none());
    let reason = catalog.refusal("Barlow Condensed", &[] as &[&str]).unwrap();
    assert_eq!(
        reason,
        "it states no embedding permissions (it has no OS/2 table), so it is treated as restricted and a download \
         leaves it out"
    );

    // Negative control: the face with its OS/2 table is neither repaired nor restricted.
    let mut sound = open(slidock_fonts::Target::Download);
    sound.add_font(BARLOW.to_vec(), FontSource::Local).unwrap();
    assert!(sound.added_fonts()[0].repairs.is_empty());
    assert!(sound.font_issues().is_empty());
    assert!(
        sound
            .catalog()
            .refusal("Barlow Condensed", &[] as &[&str])
            .is_none()
    );
}

#[test]
fn an_emoji_family_is_never_a_need() {
    let pipeline = pipeline();
    let reader = reader();
    let (registry, _) = registry_with(with_barlow_and_emoji(), Vec::new());
    let glyphs = Uses(Vec::new());
    let session = Session::open(
        &Input::new("presentation.fake", &reader),
        SessionOptions {
            glyphs: Some(&glyphs as &dyn CollectGlyphs),
            ..SessionOptions::new(&registry, &pipeline)
        },
    )
    .unwrap();
    let needs: Vec<String> = session
        .font_needs()
        .into_iter()
        .map(|need| need.family)
        .collect();
    assert!(needs.contains(&"Barlow Condensed".to_owned()), "{needs:?}");
    assert!(
        !needs.contains(&"Apple Color Emoji".to_owned()),
        "{needs:?}"
    );
    // No installed emoji file is picked, even one the presentation names by PostScript name.
    let meta = |postscript_name: &str, family: &str| slidock_fonts::resolve::LocalFontMeta {
        postscript_name: postscript_name.to_owned(),
        full_name: family.to_owned(),
        family: family.to_owned(),
        style: "Regular".to_owned(),
    };
    let picks = session.select_local(&[
        meta("AppleColorEmoji", "Apple Color Emoji"),
        meta("BarlowCondensed-Regular", "Barlow Condensed"),
    ]);
    let picked: Vec<&str> = picks
        .iter()
        .map(|pick| pick.postscript_name.as_str())
        .collect();
    assert_eq!(picked, ["BarlowCondensed-Regular"]);
}
