# Changelog

All notable changes to slidock are documented in this file. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and slidock adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - unreleased

The first public release.

### Added

- A compiler from PowerPoint (`.pptx`), Keynote (`.key`, file or package) and OpenDocument (`.odp`) presentations to
  a standalone HTML deck: one `index.html` with its media and the licences of the fonts it ships, written as a zip
  or a folder. It is written in Rust and builds natively and to WebAssembly with the same output bytes.
- The `slidock` CLI: `slidock compile <deck> -o deck.zip`, font source switches (`--offline`, `--no-google`,
  `--system-fonts`, `--fonts <dir>`, …), page metadata (`--title`, `--description`, `--site-url`), presenter and notes
  options, and `--emit ir,documents,report` to inspect what the compiler sees.
- The player every deck embeds: build animations and transitions in a common effect vocabulary (Keynote's Magic Move
  included), PowerPoint's presentation keys, a presenter view in a second window with notes and timers, and a text
  view of the slides for readers and search engines.
- Fonts: embedded faces (EOT, ODF), Google Fonts for open-licence families, installed fonts on request, dropped
  files, and bundled metric-compatible substitutes; every shipped face subset to WOFF2, with its licence listed.
- The web app (<https://macabeus.github.io/slidock/>): drop a deck, view, present and download it, compiled in the
  browser with nothing uploaded.
- Diagnostics with stable codes for everything the compiler cannot represent exactly.
- The deck format and the player API, documented in `docs/format.md` and `docs/player-api.md`.
