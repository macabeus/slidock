# Decisions and limitations

Standing decisions about slidock's output, and the limits of what it reads, beyond what `docs/format.md` and the
code's own documentation state. The goldens (`tests/golden/`) hold every one of them: changing one is a decision,
recorded with a new golden recording and its reason. Corpus rows that differ from the parity reference on purpose go
in `bench/parity/allow.json`.

## Documents

- The parity path's slide documents (`--emit documents`) print no timeline template: a slide's build steps are in the
  IR, and its transition in the IR and `presentation.json`. A deck's slide documents print the steps in a
  `<template data-sd-timeline>` in their head, but a deck page's slide templates hold the composed
  `<main>` alone: the player reads the steps and the transition from the deck JSON (`docs/format.md` §5).

## Strings

- A Rust `String` cannot hold a lone surrogate. Where a cut at a UTF-16 length would split a surrogate pair, the half
  character is dropped: format references (120 units), effect ids (120), the a11y title (200), and Keynote's link,
  alt, layout-name and URL cuts. At a Keynote character-style or link boundary inside a pair, both halves are dropped.
- A lone surrogate escape in IR JSON (`\ud800`) reads as U+FFFD, so `--emit ir` and `presentation.json` hold U+FFFD,
  as the HTML does.

## Diagnostic reason text

Codes, containers, severities and the other data fields of a diagnostic are fixed by its registry entry. A reason
that quotes a parser or decoder quotes the Rust library slidock uses:

- a schema violation (IR0001) quotes serde (`invalid type: null, expected a number`), once per value: no follow-on
  IR0008 repeats it;
- a part that cannot be read (ODP's OD9001 and OD9003, PPTX's PX2002, after the same class prefix) quotes roxmltree
  or miniz_oxide: `unknown token at 1:39`, `invalid deflate data`, `Inflated more than N bytes`.

## Values that fail the schema

slidock-ir stops at the first schema error in a value, so a container with two or more errors gets one IR0001 and is
failed keeping its layout, origin and background. This reaches:

- the IR frontend, on agent- or tool-written IR;
- PPTX custom geometry with two or more NaN coordinates in one container (each NaN goes to verify-source as `null`).

On malformed IR JSON the IR frontend also:

- refuses a document that names no `$schema`, or another one than `urn:slidock:presentation-ir:1` (IR0001 at
  `$schema`);
- reports a container key that is not a node id as a presentation-level IR0001;
- refuses a number literal that overflows f64 (`1e400`) as "Not JSON" (exit 2).

## Hostile or out-of-grid values

- ODP values computed off the IR grid (svg:viewBox sizes with more than 3 decimals, path points shifted by a non-zero
  viewBox origin after rounding, percentages such as `fo:line-height: 66.67%` lowered as `Number(x)/100`) are rounded
  to the grid, and the content is lowered.
- A NaN that Keynote lowering computes (a NaN stroke width or path point) is stored as 0. PPTX custom-geometry NaN
  fails its container (above).
- slidock-io reads XML declarations a strict parser refuses (an uppercase `<?XML`, a `standalone` other than yes/no,
  an unusual version or encoding name) and reads the part.

## Containers

- Keynote merged-cell edges: KN1112 tells two strokes apart by their fields in field-number order. slidock decodes
  into prost messages, which keep no wire order; Keynote writes fields in field-number order, so only a hand-made
  file that writes another order could get another verdict.
- TIFF and BMP media transcode to PNG with the same IHDR and the same inflated IDAT as any conforming encoder; the
  deflate stream is miniz_oxide's. Media are referenced by their source's hash, so the encoder moves no document.

## CLI

- A folder as input, or a missing or unreadable `--fonts` folder, exits 2 with `error: …`.
- On Linux, a font file whose name is not UTF-8 is skipped.

## Lint

slidock-format's lint refuses a stray `)`, a non-identifier property (`*zoom`) and brackets nested deeper than 75.
slidock's own printer never writes these.

## Numbers

- `slidock_canon::js_number` (`Number(text)`: arabic list labels, and the PPTX and ODP attributes read as numbers)
  rounds as JavaScriptCore does: a `0x`, `0o` or `0b` integer past 2^53 is summed from its last digit.
