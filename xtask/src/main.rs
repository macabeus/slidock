//! Repository tasks: code generation (ts-rs exports, preset tables, catalogs), probe-deck generation and release
//! checks. Run with `cargo xtask <task>`.

mod diagnostics;
mod fuzz_smoke;
mod golden;
mod google_catalog;
mod libreoffice_presets;
mod lo_engine;
mod notices;
mod probes_callouts;
mod probes_gradients;
mod probes_grown_text;
mod probes_lines;
mod probes_motion;
mod probes_photos;
mod probes_pictures;
mod probes_shadows;
mod probes_shrink;
mod probes_videos;

use std::process::ExitCode;

const USAGE: &str = "usage: cargo xtask <task>

tasks:
  diagnostics [--check]
      Writes docs/diagnostics.md, the reference of every registered diagnostic code (code, name, severity, message,
      feature), from crates/slidock-ir/src/diagnostics/registry.rs. --check compares instead of writing.
  fuzz-smoke [--seconds <n>] [--only <target,...>] [--jobs <n>] [--extra <dir>]
      Builds every cargo-fuzz target in fuzz/ (nightly, cargo-fuzz) and runs each for --seconds (default 60) from
      its seeds, with -rss_limit_mb=2048 -timeout=10. Fails on any crash, timeout or out-of-memory; prints each
      target's executions, coverage and corpus size.
  golden (--record --reason <text> | --check | --pin --reason <text>)
      slidock's golden files: its own outputs under $SLIDOCK_PRIVATE/golden/<commit>/ (~/.slidock), pinned by SHA-256
      in tests/golden/manifest.json with every input, the bench metrics' scoring goldens ($SLIDOCK_PRIVATE/golden/
      scoring/) among them. --record builds the release CLI at a clean HEAD, compiles the manifest's corpus pairs (the
      bench's static-lane arguments) and the IR fixtures of $SLIDOCK_PRIVATE/golden/fixtures/ir/, runs every crate's
      `golden` suites so they record their own outputs, scores the scoring goldens again, and rewrites the manifest
      with the commit, the reason and every input and output. --check verifies every pinned file, compiles again and
      compares byte for byte, then runs every crate's `golden` suites (in release where the crate's Cargo.toml asks,
      `release = true` under [package.metadata.golden]). Both skip, saying so, when the recording is absent. --pin
      pins the files the pinned folders gained, keeping the recording; a changed pinned input refuses it.
  google-catalog [--checkout <dir>] [--out <file>] [--check]
      Writes crates/slidock-fonts/data/google-fonts.txt from the METADATA.pb files of google/fonts at the pinned
      commit (open licences only). The checkout defaults to $SLIDOCK_PRIVATE/tools/google-fonts (~/.slidock), and
      is fetched there (sparse, METADATA.pb only) when missing. --check compares instead of writing.
  libreoffice-presets [--dir <dir>] [--out <dir>] [--check]
      Writes crates/slidock-presets/src/libreoffice/{effects,transitions,pptx,migration}.rs (MPL-2.0) from LibreOffice
      core's sd/xml/effects.xml, transitions.xml, oox/source/ppt/commontimenodecontext.cxx and
      sd/source/core/EffectMigration.cxx at the pinned tag (libreoffice-25.2.3.2),
      checked by SHA-256. The files default to
      $SLIDOCK_PRIVATE/tools/libreoffice-sd-xml/<commit> (~/.slidock), and are fetched there with curl when missing.
      --check compares instead of writing.
  lo-engine [--slidock <binary>] [--out <dir>]
      Checks every iris and split clip of the motion probes against LibreOffice's own presentation engine: each deck
      exported to SVG by the pinned LibreOffice (Docker, no network), its engine's frames sampled in Chromium against
      the player's clips for slidock's reading. Fails on any row that plays otherwise.
  notices [--check]
      Writes THIRD-PARTY-NOTICES.txt (and crates/slidock-cli/THIRD-PARTY-NOTICES.txt, which `slidock licenses`
      prints): LICENSE, NOTICE, the crates the slidock command and slidock-wasm link (cargo tree), the web app's
      JavaScript packages (node_modules: run pnpm install first) and the bundled fonts, with every distinct licence
      text once. --check compares instead of writing.
  probes-callouts [--check]
      Writes the callout probe decks (probes/out/callout-probe.pptx, ECMA's callout presets with their tails on every
      side, inside, far outside, on the diagonals, turned and mirrored; callout-probe.odp, LibreOffice's own callout
      types drawn from enhanced paths and equations of our own). --check compares instead of writing.
  probes-gradients [--check]
      Writes the gradient probe decks (probes/out/gradient-probe.pptx, G01-G43; gradient-probe.odp, O01-O41;
      gradient-probe-odp12.odp, ODF 1.2 angles) and gradient-probe.cases.json (each slide's parameters, whether
      LibreOffice is a valid oracle, its expected diagnostics and PowerPoint's analytic geometry). --check compares
      instead of writing.
  probes-grown-text [--check]
      Writes the grown-text probe deck (probes/out/grown-text-probe.pptx), made to be imported into Keynote: text
      boxes sized to their text inside groups, whose anchors sit at other distances from the slide's edges than from
      their group's. --check compares instead of writing.
  probes-lines [--check]
      Writes the line-ends probe decks (probes/out/line-ends-probe.pptx, .odp): arrows whose ends sit on curves or
      past a short last segment. --check compares instead of writing.
  probes-motion [--check] [--check-catalog [<dir>]]
      Writes the motion probe decks and their expectations (probes/out/motion-probe.*). --check compares instead of
      writing; --check-catalog compares the generator's LibreOffice tables with the pinned files.
  probes-photos [--check]
      Writes the photo probe deck (probes/out/photo-probe.pptx): raster pictures larger than it draws them (cropped,
      used twice, an image fill, a background, EXIF-turned, with alpha), an animated GIF and an SVG, for the
      download's picture compression. --check compares instead of writing.
  probes-pictures [--check]
      Writes the picture probe decks (probes/out/picture-probe.pptx, .odp): vector pictures, each an SVG of our own
      beside its PNG preview; the picture-types probe decks (picture-types-probe.pptx, .odp): ISO base media
      pictures typed by their ftyp brands; and the HEIC probe decks (heic-probe.pptx, .odp): Apple-encoded photos.
      --check compares instead of writing.
  probes-shadows [--check]
      Writes the shadow probe decks (probes/out/shadow-probe.pptx and .odp, twins of 46 slides; shadow-probe-odp.odp,
      LibreOffice-only semantics; shadow-probe-ooxml.pptx, OOXML-only), one case per slide. --check compares instead
      of writing.
  probes-shrink [--check]
      Writes the shrink probe decks (probes/out/shrink-probe.pptx, .odp): text boxes that shrink on overflow and
      store no scale, fitted by PowerPoint's steps. --check compares instead of writing.
  probes-videos [--check]
      Writes the video probe deck (probes/out/video-probe.pptx): movies drawn smaller than they are (with AAC or PCM
      sound, turned by their matrix, used twice and trimmed), one drawn larger than it is, one already small and one
      with alpha, for the download's video compression. --check compares instead of writing.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("diagnostics") => diagnostics::run(args.get(1..).unwrap_or_default()),
        Some("fuzz-smoke") => fuzz_smoke::run(args.get(1..).unwrap_or_default()),
        Some("golden") => golden::run(args.get(1..).unwrap_or_default()),
        Some("google-catalog") => google_catalog::run(args.get(1..).unwrap_or_default()),
        Some("libreoffice-presets") => libreoffice_presets::run(args.get(1..).unwrap_or_default()),
        Some("lo-engine") => lo_engine::run(args.get(1..).unwrap_or_default()),
        Some("notices") => notices::run(args.get(1..).unwrap_or_default()),
        Some("probes-callouts") => probes_callouts::run(args.get(1..).unwrap_or_default()),
        Some("probes-gradients") => probes_gradients::run(args.get(1..).unwrap_or_default()),
        Some("probes-grown-text") => probes_grown_text::run(args.get(1..).unwrap_or_default()),
        Some("probes-lines") => probes_lines::run(args.get(1..).unwrap_or_default()),
        Some("probes-motion") => probes_motion::run(args.get(1..).unwrap_or_default()),
        Some("probes-photos") => probes_photos::run(args.get(1..).unwrap_or_default()),
        Some("probes-pictures") => probes_pictures::run(args.get(1..).unwrap_or_default()),
        Some("probes-shadows") => probes_shadows::run(args.get(1..).unwrap_or_default()),
        Some("probes-shrink") => probes_shrink::run(args.get(1..).unwrap_or_default()),
        Some("probes-videos") => probes_videos::run(args.get(1..).unwrap_or_default()),
        _ => Err(USAGE.to_owned()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn crate_name() {
        assert_eq!(env!("CARGO_PKG_NAME"), "xtask");
    }
}
