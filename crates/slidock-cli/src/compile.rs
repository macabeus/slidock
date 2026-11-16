//! `slidock compile`: one input through the frontend, the passes and the bundler into a deck zip or folder (`--out`,
//! by default `<input stem>.zip` in the working folder); with `--emit`, through the frontend, the passes and, for
//! `documents`, the HTML backend, writing what `--emit` asks for. Flags that shape a deck conflict with `--emit`.
use std::path::{Path, PathBuf};

use clap::{Args, ValueEnum};
use slidock::{
    BundleOptions, BundleSink, CompileError, CompileResult, Compiled, Compiler, Described,
    Diagnostic, FrontendChoice, Frontends, Input, MediaFiles, NotesMode, Options, Packaging,
    Severity, SlideId, SourceFormat, Stop, VerifyMode, compile_report, dump_presentation,
    has_errors, size_notice,
};

use crate::Exit;
use crate::bundle::{Folder, Target, ZipFile};
use crate::files::{MediaSources, media_dir, open_input, write_documents, write_text};
use crate::fonts::{font_catalog, font_dirs};
use crate::say::{self, Verbosity};
use crate::sources::{FontSources, cache_dir, compile_deck};

/// What `--emit` writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Emit {
    /// The presentation in slidock's intermediate representation, as JSON (`presentation-ir.json`).
    Ir,
    /// A standalone HTML file per slide (`slides/`) and per layout (`layouts/`), with `presentation.json` listing
    /// them and the media they show (`media/`).
    Documents,
    /// The compile report, as JSON (`report.json`): the input's details, every diagnostic, the features the
    /// presentation uses and what each slide shows.
    Report,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Verify {
    /// After every pass.
    Each,
    /// Before and after the passes.
    Boundaries,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum PhaseArg {
    /// Stop after lowering and verify-source.
    Source,
    /// Run every pass.
    Resolved,
}

/// `--frontend`: the input's format, or `auto` to read it from the input's bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum FrontendArg {
    Auto,
    Pptx,
    Odp,
    Keynote,
    /// slidock's intermediate representation (JSON), for development.
    #[value(hide = true)]
    Ir,
}

/// The IR dump's name in the output folder.
pub const IR_FILE: &str = "presentation-ir.json";
/// The compile report's name in the output folder.
pub const REPORT_FILE: &str = "report.json";

const FONTS: &str = "Fonts";
const PAGE: &str = "Page";
const ADVANCED: &str = "Advanced";

#[derive(Debug, Args)]
pub struct CompileArgs {
    /// The presentation: a .pptx, .key or .odp file
    ///
    /// A PowerPoint (.pptx), Keynote (.key) or OpenDocument (.odp) file. Its format is read from its contents, not
    /// from its name.
    input: PathBuf,
    /// Where to write the deck: a .zip file or a folder [default: the input's name with .zip, in the working folder]
    ///
    /// A path ending in `.zip` writes a zip; any other path, a folder holding `index.html`, `media/` and
    /// `LICENSES.txt` (only those are replaced, once the deck is complete). With --emit, the folder the outputs go
    /// to; without --out, `ir` or `report` goes to stdout.
    ///
    /// [default: the input's name with .zip, in the working folder]
    #[arg(long, short, value_name = "PATH")]
    out: Option<PathBuf>,
    /// Puts the media inside index.html, so the deck is one file
    ///
    /// The media are inlined when they add up to at most 50 MiB; a deck with more keeps them in `media/`.
    #[arg(long, conflicts_with = "emit")]
    single_file: bool,
    /// Compiles only these slides: numbers from 1, or ids such as pp-256 (comma-separated)
    ///
    /// Slide numbers count from 1 in show order, hidden slides included. Slide ids are the ones diagnostics and
    /// `--emit report` print: `pp-256` (PowerPoint), `od-p1` (OpenDocument), `kn-5213335` (Keynote).
    #[arg(long, value_delimiter = ',', value_name = "SLIDE")]
    only: Vec<String>,
    /// Stops the compile after this many seconds (by default there is no time limit)
    ///
    /// The time counts from opening the input to the last file written; past it, the compile fails with IR0905 and
    /// writes no deck. A service should also cap the process's memory and wall-clock time from outside, as
    /// SECURITY.md describes.
    #[arg(long, value_name = "SECS")]
    timeout: Option<u64>,
    /// Prints errors only
    #[arg(long, short, conflicts_with = "verbose")]
    quiet: bool,
    /// Prints every diagnostic, info included, and the fonts each source added
    #[arg(long, short)]
    verbose: bool,

    /// Makes no network request (by default, missing fonts are downloaded from Google Fonts)
    ///
    /// By default, the fonts a deck uses but neither embeds nor finds elsewhere are downloaded from Google Fonts
    /// (open-licence families only) and cached in the user's cache folder (`$SLIDOCK_CACHE_DIR` chooses another).
    /// With --offline, slidock makes no network request at all.
    #[arg(long, conflicts_with = "emit", help_heading = FONTS)]
    offline: bool,
    /// Takes no font from Google Fonts, not even one already cached
    #[arg(long, conflicts_with = "emit", help_heading = FONTS)]
    no_google: bool,
    /// Also takes missing fonts from the fonts installed on this computer
    ///
    /// They are tried after the embedded fonts and before Google Fonts. Off by default, so a deck does not depend on
    /// the computer that compiled it.
    #[arg(long, conflicts_with = "emit", help_heading = FONTS)]
    system_fonts: bool,
    /// Ignores the fonts embedded in the presentation (by default they come first)
    #[arg(long, conflicts_with = "emit", help_heading = FONTS)]
    no_embedded: bool,
    /// A folder of font files to take the fonts still missing from (repeatable)
    ///
    /// TrueType, OpenType and collection files (.ttf, .otf, .ttc), searched recursively. They are tried after every
    /// other source; a later folder's font wins over an earlier one's. With --emit, they lay out the text.
    #[arg(long, value_name = "DIR", help_heading = FONTS)]
    fonts: Vec<PathBuf>,
    /// Ships no substitute font file
    ///
    /// By default, a font found nowhere is replaced by a metric-compatible one built into slidock (Carlito for
    /// Calibri, for instance), which the deck ships. With --no-substitutes, the deck only names the substitute, and
    /// the viewer's own fonts draw it.
    #[arg(long, conflicts_with = "emit", help_heading = FONTS)]
    no_substitutes: bool,
    /// Compiles a deck as --emit does: no font source, no font shipped and no native motion data. For comparing
    /// decks with --emit output.
    #[arg(
        long,
        hide = true,
        conflicts_with_all = ["emit", "no_embedded", "system_fonts", "offline", "no_google", "no_substitutes"]
    )]
    fonts_off: bool,

    /// The page's title [default: the first slide's title, else the input's file name]
    #[arg(long, conflicts_with = "emit", help_heading = PAGE)]
    title: Option<String>,
    /// The page's description for search results and link previews; "" for none
    ///
    /// [default: the other slides' titles, up to 160 characters]
    #[arg(long, conflicts_with = "emit", help_heading = PAGE)]
    description: Option<String>,
    /// The deck's public address (an absolute http or https URL), named as the page's canonical URL
    #[arg(long, value_name = "URL", conflicts_with = "emit", help_heading = PAGE)]
    site_url: Option<String>,
    /// Leaves out the presenter view
    ///
    /// By default, P opens the presenter view in a second window: the current and next slide, the speaker notes and
    /// timers.
    #[arg(long, conflicts_with = "emit", help_heading = PAGE)]
    no_presenter: bool,
    /// Leaves out the speaker notes
    #[arg(long, conflicts_with = "emit", help_heading = PAGE)]
    no_notes: bool,
    /// Also shows the speaker notes on the page, in the slides' text below the deck
    #[arg(long, conflicts_with_all = ["emit", "no_notes", "no_text_view"], help_heading = PAGE)]
    notes_on_page: bool,
    /// Leaves out the slides' text below the deck
    ///
    /// By default the page carries the slides' text below the deck, which search engines index and readers can
    /// scroll to.
    #[arg(long, conflicts_with = "emit", help_heading = PAGE)]
    no_text_view: bool,

    /// Writes these instead of a deck (comma-separated)
    ///
    /// They go into the --out folder; without --out, `ir` or `report` goes to stdout.
    #[arg(long, value_enum, value_delimiter = ',', value_name = "WHAT", help_heading = ADVANCED)]
    emit: Vec<Emit>,
    /// With --emit documents: writes no media files (the documents still link to them)
    #[arg(long, requires = "emit", help_heading = ADVANCED)]
    no_media: bool,
    /// The input's format, when its contents should not decide
    #[arg(long, value_enum, value_name = "FORMAT", default_value_t = FrontendArg::Auto, help_heading = ADVANCED)]
    frontend: FrontendArg,

    /// The media folder of an IR input [default: `media/` beside it].
    #[arg(long, value_name = "DIR", hide = true)]
    media: Option<PathBuf>,
    /// When the IR is verified.
    #[arg(long, value_enum, default_value_t = Verify::Boundaries, hide = true)]
    verify: Verify,
    /// How far to compile: `source` stops after the frontend.
    #[arg(long, value_enum, default_value_t = PhaseArg::Resolved, requires = "emit", hide = true)]
    phase: PhaseArg,
    /// Writes each failed pass's input (view, facts, invariants) into this folder as `<pass>-<container>.json`.
    #[arg(long, value_name = "DIR", requires = "emit", hide = true)]
    dump_failures: Option<PathBuf>,
    /// Times the passes with the host's clock, printed and in the report.
    #[arg(long, requires = "emit", hide = true)]
    timings: bool,
    /// With --emit ir and no documents: the IR keeps the motion data a deck plays (effects in the common vocabulary,
    /// timing, match keys, and each format's own placement and delivery), which --emit leaves out.
    #[arg(long, requires = "emit", hide = true)]
    native_motion: bool,
}

fn usage(message: impl std::fmt::Display) -> Exit {
    say::line(format!("error: {message}"));
    Exit::Usage
}

fn failed(message: impl std::fmt::Display) -> Exit {
    say::line(format!("error: {message}"));
    Exit::Failed
}

fn frontend_choice(value: FrontendArg) -> Result<FrontendChoice, Exit> {
    let name = match value {
        FrontendArg::Auto => return Ok(FrontendChoice::Auto),
        FrontendArg::Pptx => "pptx",
        FrontendArg::Odp => "odp",
        FrontendArg::Keynote => "keynote",
        FrontendArg::Ir => "ir",
    };
    SourceFormat::parse(name)
        .filter(|format| slidock::input_formats().contains(format))
        .map(FrontendChoice::Format)
        .ok_or_else(|| usage(format!("--frontend {name} is not in this build")))
}

/// One line per diagnostic: severity, code, name, where, message.
fn format_diagnostic(d: &Diagnostic) -> String {
    let place: Vec<&str> = [d.container.as_ref(), d.node.as_ref()]
        .into_iter()
        .flatten()
        .map(|id| id.as_str())
        .collect();
    let place = if place.is_empty() {
        String::new()
    } else {
        format!(" {}", place.join(" "))
    };
    let count = d.count.map_or(String::new(), |n| format!(" (×{n})"));
    format!(
        "{:<7} {} {}{place}: {}{count}",
        d.severity.as_str(),
        d.code.code(),
        d.code.name(),
        d.message
    )
}

/// The least severe diagnostic a verbosity lists.
fn listed_from(verbosity: Verbosity) -> Severity {
    match verbosity {
        Verbosity::Quiet => Severity::Error,
        Verbosity::Normal => Severity::Warning,
        Verbosity::Verbose => Severity::Info,
    }
}

/// The count of each severity, most severe first (`2 errors, 12 lossy, 88 info`), and how many of them are not
/// listed.
fn summary(diagnostics: &[Diagnostic], listed: Severity) -> String {
    let counts: Vec<String> = Severity::ALL
        .iter()
        .filter_map(|&severity| {
            let n = diagnostics
                .iter()
                .filter(|d| d.severity == severity)
                .count();
            let name = match (severity, n) {
                (Severity::Lossy | Severity::Info, _) | (_, 1) => severity.as_str().to_owned(),
                _ => format!("{}s", severity.as_str()),
            };
            (n > 0).then(|| format!("{n} {name}"))
        })
        .collect();
    let hidden = diagnostics
        .iter()
        .filter(|d| !d.severity.at_least(listed))
        .count();
    let hidden = if hidden == 0 {
        String::new()
    } else {
        format!(" ({hidden} not listed; -v lists them)")
    };
    format!("diagnostics: {}{hidden}", counts.join(", "))
}

/// Prints the diagnostics the verbosity lists, then the summary unless `-q`.
fn print_diagnostics(diagnostics: &[Diagnostic]) {
    let verbosity = say::verbosity();
    let listed = listed_from(verbosity);
    for diagnostic in diagnostics {
        if diagnostic.severity.at_least(listed) {
            say::line(format_diagnostic(diagnostic));
        }
    }
    if !diagnostics.is_empty() {
        say::normal(summary(diagnostics, listed));
    }
}

/// Where a text output goes.
fn write_output(out: Option<&Path>, file: &str, text: &str) -> Result<(), Exit> {
    match out {
        Some(dir) => write_text(&dir.join(file), text).map_err(failed),
        None => say::stdout(text.as_bytes()).map_err(failed),
    }
}

/// The deck's path without `--out`: the input's name with `.zip` for its extension, in the working folder.
fn default_out(input: &Path) -> Result<PathBuf, Exit> {
    let stem = input
        .file_stem()
        .filter(|stem| !stem.is_empty())
        .ok_or_else(|| usage(format!("{}: not a file name", input.display())))?;
    let mut name = stem.to_owned();
    name.push(".zip");
    Ok(PathBuf::from(name))
}

/// Whether `a` and `b` are the same existing file.
fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// The slides `--only` names: ids as they are, numbers (from 1, in show order with hidden slides) as the ids of
/// those slides, read from the input's metadata.
fn only_slides(
    only: &[String],
    compiler: &Compiler,
    source: &Input<'_>,
    options: &Options<'_>,
) -> Result<Vec<SlideId>, Exit> {
    let mut slides: Option<Vec<SlideId>> = None;
    let mut ids = Vec::new();
    for entry in only {
        let entry = entry.trim();
        let Ok(number) = entry.parse::<usize>() else {
            ids.push(SlideId::new(entry).map_err(|_| {
                usage(format!(
                    "--only: {entry} is not a slide number or a slide id (such as pp-256)"
                ))
            })?);
            continue;
        };
        if slides.is_none() {
            let analyzed = compiler.analyze(source, options);
            let Some(analysis) = analyzed.analysis else {
                print_diagnostics(&analyzed.diagnostics);
                return Err(if analyzed.refusal.is_some() {
                    Exit::Usage
                } else {
                    Exit::Failed
                });
            };
            slides = Some(analysis.slides.into_iter().map(|slide| slide.id).collect());
        }
        let all = slides.as_deref().unwrap_or_default();
        let id = number
            .checked_sub(1)
            .and_then(|index| all.get(index))
            .ok_or_else(|| {
                usage(format!(
                    "--only: there is no slide {number}; the presentation has {}",
                    all.len()
                ))
            })?;
        ids.push(id.clone());
    }
    Ok(ids)
}

/// What one run produced: a compile printed to documents, or one through the frontend and the passes only.
enum Outcome {
    Printed(Box<Compiled>),
    Ran(Box<CompileResult>),
}

impl Outcome {
    fn diagnostics(&self) -> &[Diagnostic] {
        match self {
            Outcome::Printed(compiled) => &compiled.diagnostics,
            Outcome::Ran(result) => &result.diagnostics,
        }
    }

    fn result(&self) -> &CompileResult {
        match self {
            Outcome::Printed(compiled) => &compiled.result,
            Outcome::Ran(result) => result,
        }
    }

    /// The presentation the IR and the report describe: the printed one with documents, the compile's otherwise.
    fn described(&self, phase: PhaseArg) -> Option<Described<'_>> {
        match self {
            Outcome::Printed(compiled) => compiled.presentation().map(Described::Resolved),
            Outcome::Ran(result) => match phase {
                PhaseArg::Source => result.source.as_ref().map(Described::Source),
                PhaseArg::Resolved => result.presentation.as_ref().map(Described::Resolved),
            },
        }
    }
}

pub fn run(args: CompileArgs) -> Exit {
    say::set_verbosity(if args.quiet {
        Verbosity::Quiet
    } else if args.verbose {
        Verbosity::Verbose
    } else {
        Verbosity::Normal
    });
    match compile(args) {
        Ok(exit) | Err(exit) => exit,
    }
}

fn compile(args: CompileArgs) -> Result<Exit, Exit> {
    let mut emits: Vec<Emit> = Vec::new();
    for emit in &args.emit {
        if !emits.contains(emit) {
            emits.push(*emit);
        }
    }
    let bundling = emits.is_empty();
    let documents = emits.contains(&Emit::Documents);
    if args.native_motion && (documents || !emits.contains(&Emit::Ir)) {
        return Err(usage("--native-motion needs --emit ir and no documents"));
    }
    if documents && args.out.is_none() {
        return Err(usage(
            "--emit documents needs --out, the folder to write them to",
        ));
    }
    if documents && args.phase == PhaseArg::Source {
        return Err(usage(
            "--emit documents writes HTML, which needs the resolved phase",
        ));
    }
    if args.no_media && !documents {
        return Err(usage("--no-media needs --emit documents"));
    }
    if args.out.is_none() && emits.contains(&Emit::Ir) && emits.contains(&Emit::Report) {
        return Err(usage(
            "only one of ir and report can go to stdout; give --out, the folder to write both to",
        ));
    }
    if let Some(url) = &args.site_url {
        slidock::site_url(url).map_err(usage)?;
    }
    let out = match (&args.out, bundling) {
        (Some(out), _) => Some(out.clone()),
        (None, true) => Some(default_out(&args.input)?),
        (None, false) => None,
    };
    if let Some(out) = &out
        && same_file(out, &args.input)
    {
        return Err(usage(format!(
            "{} is the input; give another --out",
            out.display()
        )));
    }
    let frontend = frontend_choice(args.frontend)?;

    let cwd = std::env::current_dir().map_err(usage)?;
    let fonts_on = bundling && !args.fonts_off;
    let catalog_dirs: &[PathBuf] = if fonts_on { &[] } else { &args.fonts };
    let (fonts, problems) = font_catalog(catalog_dirs, &cwd).map_err(usage)?;
    for problem in problems {
        say::normal(format!(
            "warning font {}: {}",
            problem.file.display(),
            problem.message
        ));
    }
    let sources = if fonts_on {
        FontSources {
            embedded: !args.no_embedded,
            system: args.system_fonts,
            google: !(args.no_google || args.offline),
            ship: true,
            bundled: !args.no_substitutes,
            dropped: font_dirs(&args.fonts, &cwd).map_err(usage)?,
            cache: cache_dir(),
        }
    } else {
        FontSources::default()
    };
    let input = open_input(&args.input)
        .map_err(|error| usage(format!("{}: {error}", args.input.display())))?;
    let name = args
        .input
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| usage(format!("{}: not a file name", args.input.display())))?
        .to_owned();
    let ir_media = match &args.media {
        Some(dir) => {
            media_dir(dir).map_err(|error| usage(format!("{}: {error}", dir.display())))?
        }
        None => {
            let beside = args
                .input
                .parent()
                .map_or_else(|| PathBuf::from("media"), |dir| dir.join("media"));
            media_dir(&beside).unwrap_or_else(|_| MediaFiles::new())
        }
    };
    let ir_frontends = Frontends::default().with_ir_media(ir_media);
    let compiler = Compiler::new(ir_frontends.clone())
        .map_err(|error| usage(format!("The resolved phase is not available: {error}")))?;

    let clock = crate::clock::Wall::start();
    let mut limits = slidock::Limits::default();
    if let Some(secs) = args.timeout {
        let ms = secs.saturating_mul(1000);
        (limits.analyze_ms, limits.lower_ms, limits.compile_ms) = (ms, ms, ms);
    }
    let mut options = Options {
        frontend,
        verify: match args.verify {
            Verify::Each => VerifyMode::Each,
            Verify::Boundaries => VerifyMode::Boundaries,
        },
        clock: (args.timings || args.timeout.is_some()).then_some(&clock as &dyn slidock::Clock),
        limits,
        fonts: &fonts,
        // A deck keeps the native motion data; --emit and --fonts-off leave it out.
        native_motion: fonts_on || args.native_motion,
        ..Options::default()
    };
    let source = Input::new(&name, &input);
    let only = if args.only.is_empty() {
        None
    } else {
        Some(only_slides(&args.only, &compiler, &source, &options)?)
    };
    options.only = only.as_deref();
    if let (true, Some(out)) = (bundling, &out) {
        let bundle_options = BundleOptions {
            single_file: args.single_file,
            text_view: !args.no_text_view,
            presenter: !args.no_presenter,
            notes: if args.no_notes {
                NotesMode::None
            } else if args.notes_on_page {
                NotesMode::Page
            } else {
                NotesMode::Presenter
            },
            title: args.title.clone(),
            description: args.description.clone(),
            site_url: args.site_url.clone(),
            ..BundleOptions::default()
        };
        return bundle(
            &source,
            ir_frontends,
            &options,
            &sources,
            bundle_options,
            out,
        );
    }
    let outcome = if documents {
        Outcome::Printed(Box::new(
            compiler.compile(&source, &options).map_err(failed)?,
        ))
    } else {
        let stop = match args.phase {
            PhaseArg::Source => Stop::Source,
            PhaseArg::Resolved => Stop::Resolved,
        };
        Outcome::Ran(Box::new(compiler.run(&source, &options, stop)))
    };

    let mut outcome = outcome;
    if !args.timings {
        // A clock given only for --timeout times nothing that is written.
        match &mut outcome {
            Outcome::Printed(compiled) => compiled.result.stats = None,
            Outcome::Ran(result) => result.stats = None,
        }
    }
    let diagnostics = outcome.diagnostics();
    print_diagnostics(diagnostics);
    let result = outcome.result();
    if let Some(dir) = &args.dump_failures {
        for failure in &result.failures {
            let container = failure
                .container
                .as_ref()
                .map_or("presentation", |id| id.as_str());
            let text = slidock::to_canonical_json(failure, slidock::JsonLayout::Indented)
                .map_err(failed)?;
            write_text(
                &dir.join(format!("{}-{container}.json", failure.pass)),
                &text,
            )
            .map_err(failed)?;
        }
    }
    let out = out.as_deref();
    let described = outcome.described(args.phase);
    if emits.contains(&Emit::Ir) {
        let dump = match described {
            Some(Described::Source(source)) => Some(dump_presentation(source)),
            Some(Described::Resolved(resolved)) => Some(dump_presentation(resolved)),
            None => None,
        };
        if let Some(dump) = dump {
            write_output(out, IR_FILE, &dump.map_err(failed)?)?;
        }
    }
    if emits.contains(&Emit::Report) {
        let report = compile_report(result, described, diagnostics).map_err(failed)?;
        write_output(out, REPORT_FILE, &report)?;
    }
    if let Some(stats) = &result.stats {
        for timing in &stats.passes {
            say::line(format!("timing {}: {} ms", timing.pass, timing.ms));
        }
    }
    if let (Some(dir), Outcome::Printed(compiled)) = (out, &outcome)
        && let (Some(documents), Some(store)) = (compiled.documents(), compiled.media())
    {
        let media = MediaSources {
            store,
            input: &input,
            max_pixels: options.policy.transcode_pixels,
        };
        write_documents(dir, documents, (!args.no_media).then_some(&media)).map_err(failed)?;
    }

    if result.refusal.is_some() {
        return Ok(Exit::Usage);
    }
    Ok(if has_errors(diagnostics) {
        Exit::Failed
    } else {
        Exit::Ok
    })
}

/// Bundles `input` into the deck zip or folder `out`. A refused input exits 2; a presentation that fails as a whole,
/// or a bundle that cannot be written, exits 1; neither leaves a zip or changes the folder.
fn bundle(
    input: &Input<'_>,
    frontends: Frontends,
    options: &Options<'_>,
    sources: &FontSources,
    mut bundle_options: BundleOptions,
    out: &Path,
) -> Result<Exit, Exit> {
    let target = Target::of(out);
    bundle_options.packaging = match target {
        Target::Zip(_) => Packaging::Zip,
        Target::Folder(_) => Packaging::Files,
    };
    let written = |sink: &mut dyn BundleSink| {
        compile_deck(
            input,
            frontends.clone(),
            options,
            sources,
            &bundle_options,
            sink,
        )
    };
    let compiled = match &target {
        Target::Zip(path) => {
            let mut zip = ZipFile::create(path).map_err(failed)?;
            let compiled = written(&mut zip);
            if compiled.is_ok() {
                zip.commit().map_err(failed)?;
            }
            compiled
        }
        Target::Folder(path) => {
            let mut folder = Folder::create(path).map_err(failed)?;
            let compiled = written(&mut folder);
            if compiled.is_ok() {
                folder.commit().map_err(failed)?;
            }
            compiled
        }
    };
    match compiled {
        Ok(bundled) => {
            print_diagnostics(&bundled.diagnostics);
            if let Some(notice) = size_notice(&bundled.report, bundle_options.text_view) {
                say::normal(format!("note: {notice}"));
            }
            say::normal(format!("wrote {}", out.display()));
            Ok(if has_errors(&bundled.diagnostics) {
                Exit::Failed
            } else {
                Exit::Ok
            })
        }
        Err(CompileError::Open(error)) => {
            print_diagnostics(&error.diagnostics);
            Ok(if error.refusal.is_some() {
                Exit::Usage
            } else {
                Exit::Failed
            })
        }
        Err(CompileError::Failed { diagnostics }) => {
            print_diagnostics(&diagnostics);
            Ok(Exit::Failed)
        }
        Err(error) => Err(failed(error)),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn diagnostic(extra: serde_json::Value) -> Diagnostic {
        let mut value = json!({ "code": "IR0011", "severity": "error", "message": "The message",
                                "stage": "frontend", "emitter": "ir" });
        for (key, field) in extra.as_object().unwrap() {
            value[key] = field.clone();
        }
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn formats_a_diagnostic_as_severity_code_name_place_message_and_count() {
        let full = diagnostic(json!({ "container": "ir-s1", "node": "ir-s1.a", "count": 3 }));
        assert_eq!(
            format_diagnostic(&full),
            "error   IR0011 origin-invalid ir-s1 ir-s1.a: The message (×3)"
        );
        let bare = diagnostic(json!({}));
        assert_eq!(
            format_diagnostic(&bare),
            "error   IR0011 origin-invalid: The message"
        );
    }

    fn of(code: &str, severity: &str) -> Diagnostic {
        diagnostic(json!({ "code": code, "severity": severity }))
    }

    #[test]
    fn sums_up_each_severity_most_severe_first_and_what_is_not_listed() {
        let diagnostics = [
            of("MID5801", "info"),
            of("MID5401", "lossy"),
            of("MID5401", "lossy"),
            of("IR0011", "error"),
            of("MID5801", "info"),
            of("KN1005", "warning"),
        ];
        assert_eq!(
            summary(&diagnostics, listed_from(Verbosity::Normal)),
            "diagnostics: 1 error, 2 lossy, 1 warning, 2 info (2 not listed; -v lists them)"
        );
        assert_eq!(
            summary(&diagnostics, listed_from(Verbosity::Verbose)),
            "diagnostics: 1 error, 2 lossy, 1 warning, 2 info"
        );
        let errors = [of("IR0011", "error"), of("IR0011", "error")];
        assert_eq!(
            summary(&errors, listed_from(Verbosity::Normal)),
            "diagnostics: 2 errors"
        );
    }

    #[test]
    fn the_default_deck_is_the_inputs_name_with_zip_in_the_working_folder() {
        assert_eq!(
            default_out(Path::new("/decks/talk.pptx")).unwrap(),
            PathBuf::from("talk.zip")
        );
        assert_eq!(
            default_out(Path::new("talk.v2.key")).unwrap(),
            PathBuf::from("talk.v2.zip")
        );
        assert_eq!(
            default_out(Path::new("slides")).unwrap(),
            PathBuf::from("slides.zip")
        );
    }
}
