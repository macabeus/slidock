# Motion probe decks

Spec-authored PPTX and ODP decks that carry every transition, build preset and timing feature the frontends map to
the effect vocabulary (`docs/format.md` §5.2), with the expectation each frontend must meet.
Every shape names the effect it carries; all text, shapes and media are our own.

| File (`probes/out/`)                    | What                                                                                                                                                                                                                          |
| --------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `motion-probe.pptx`                     | PresentationML: ECMA-376 transitions, `p14`/`p15`/`p159` inside `mc:AlternateContent` with a `p:fade` fallback, `p:timing` trees                                                                                              |
| `motion-probe.odp`                      | ODF 1.3: drawing-page transitions (`smil:*`, `presentation:transition-*`, the timing root's exact `transitionFilter` duration), `anim:*` trees                                                                                |
| `motion-probe.expect.json`              | per deck, per slide: transition and steps with native id, vocabulary effect and params, trigger, delivery, paragraphs, offset/delay/duration, repeat, after effect, or the fallback and diagnostic; plus the catalog coverage |
| `motion-probe.{pptx,odp}.manifest.json` | generator, inputs and sha256s                                                                                                                                                                                                 |
| `motion-probe.validation.txt`           | the xmllint result per XML part                                                                                                                                                                                               |

## Regenerate

```sh
cargo xtask probes-motion            # writes the five files above
cargo xtask probes-motion --check    # fails when a committed file differs (the xtask tests run the same check)
probes/motion/validate.sh            # xmllint every part; rewrites motion-probe.validation.txt
```

The generator (`xtask/src/probes_motion.rs` and `probes_{catalog,pptx,odp,zip,json}.rs`) is deterministic: zip
entries in a fixed order with a fixed 1980-01-01 timestamp, our own deflate, no clock. Regenerating is a no-op.

The media (`media/clip.mp4`, `media/poster.png`) come from `make-media.sh` (ffmpeg); they are committed and embedded,
so ffmpeg is needed only to change them.

## Layout

One ordered list of slides is shared by both decks; a slide, shape or step one format can't express is left out of
that deck only, so shared slides keep the same order, keys and labels (`key` in the expectations):

1. `title`
2. `tr-*`: one slide per transition × direction/option. PPTX: every catalog row `12.1/…` (ECMA, p14, p15, p159).
   ODP: every entry of LibreOffice's `transitions.xml` and `transitions-ogl.xml`, plus a custom fade colour,
   `zigZagWipe`, `bowTieWipe`, `veeWipe`, `starWipe` and three ODF 1.0 `transition-style` names. Shared slides pair
   each OOXML transition with the SMIL transition that moves the same way.
3. `tm-*`: speeds, exact durations (`p14:dur`, `transitionFilter smil:dur`) and advance (on click, after a time,
   ODF semi-automatic and automatic with no duration).
4. `pr-*`: every preset of LibreOffice's `effects.xml` (entrance, exit, emphasis, motion path), six to a slide, each
   on its own click; the PPTX deck carries the ones with a PowerPoint `presetID`/`presetSubtype` (LibreOffice's
   `preset_mapping`, sub-types inverted from `getConvertedSubType`).
5. `ft-*`: triggers with delays, an automatic group 0, by paragraph (an empty paragraph, a level-2 child), iterate by
   word/letter/paragraph (`tmAbs`, `tmPct`, backwards), background then text (`animBg`, `anim:sub-item`), repeat
   (count, until end of slide, until next click), auto-reverse, accelerate/decelerate, after effects (hide, dim),
   tier 2 (an unknown preset with set/anim+`fmla`/animEffect/animMotion/animRot/animScale/animClr; custom effects for
   every `animEffect@filter` row), an unknown preset that must fall back, a custom motion path, media
   play/pause/stop, an interactive trigger (dropped), a line with an arrowhead fading in (`ft-line-fade`: a box of zero
   height its stroke and arrowhead overflow, PPTX `p:cxnSp` with a `tailEnd`, ODP `draw:line` with a `draw:marker-end`),
   and hostile timing a frontend must survive with a diagnostic:
   a self-referencing condition and a two-node condition cycle (dropped), a repeat count, a duration and a delay past
   their limits (clamped), a formula nested 400 deep (can't compile: the fallback plays), an effect nested in 64
   container levels (dropped), and malformed values (NaN and negative durations, an overflowing delay, a negative
   advance time: the declared default, or clamped).

## Conventions

`motion-probe.expect.json` → `conventions` states them; the main ones:

- `dir` is the direction the content moves, in OOXML tokens (`docs/format.md` §5.2). Fly in from the left is `dir=r`; an exit
  names where the content goes; a wipe names the way its edge travels.
- PPTX `push`/`cover`/`pull` follow ECMA-376 (the slide moves in `dir`), as Keynote's own PPTX export confirms (its
  default Push, Move In and Reveal move the content left in its movie export, written `dir="l"`). LibreOffice's PPTX
  import maps `l`/`r` to SMIL `fromLeft`/`fromRight` (the opposite horizontal sense); the ODP deck uses SMIL's
  meaning, as LibreOffice's own `transitions.xml` names do (`push-left` = `pushWipe fromRight`).
- An exit's params name the entrance it reverses (exit zoom `out` is `zoom{inOut: in}`; exit box `in`, the filter
  `irisWipe rectangle reverse` / `box(in)` played out, is `iris{inOut: in}`, a hole opening from the centre as
  LibreOffice's engine plays it and ECMA-376 reads `transition="out"`), and a named param is what LibreOffice's
  `effects.xml` animates, never a sub-type's word (swivel `vertical` animates the width: `axis: horz`). The
  expectations spell an orientation or an axis in the vocabulary's OOXML tokens (`horz`, `vert`).
- A preset the vocabulary only approximates is played from its own behaviours (tier 2) in both decks; the
  expectation names the approximation (with effect-approximated) only where the behaviours compile to nothing.
- The colour and font emphases animate what `effects.xml` animates: the fill or line they colour is switched on beside
  the colour (`set fill=solid`, `set stroke=solid`; PPTX `fill.type`, `stroke.on`), a shape whose line a step
  colours draws one, Color Over by Letter's sets reverse on their own over half the step (played: a switch keeps its
  value over twice its span), and Change Font Size animates the font size (a declared fallback: the layout would
  move).
- Flash Once's entrance is the visibility switch `effects.xml` writes: it lasts the step and does not hold (no
  `fill`), so the element shows while it lasts and hides after (`appear`, after `hide`). The flash bulb pulses (a scale
  that reverses on its own over half the step) while it fades out to half and back under `effects.xml`'s time filter
  (`smil:keySplines`, PPTX `tmFilter`); the flicker flashes its colours to white and back. Both end at rest. The
  other emphases with no motion of their own (blast among them) are our own held scale to 110 %: a tier-2 transform
  emphasis that keeps its end, as `effects.xml`'s blast keeps its scale and colour (`fill="hold"`).
- A play command lasts the clip; stop and pause are instants (PowerPoint's 1 ms, `effects.xml`'s 0.001 s).
- One rule maps a clip to the vocabulary for presets, tier 2 and transitions: the SMIL filter (type, subtype,
  direction, mode), never a preset sub-type label. LibreOffice's labels for venetian blinds, random bars and diagonal
  squares name the opposite orientation or corner to the filter they play (sub-type `vertical` plays
  `blindsWipe horizontal`, so PPTX `entr.3/10` is `blinds orient=horizontal`). PPTX filter names read as
  LibreOffice reads them (`wipe(down)` starts at the bottom, the edge moves up). Each preset in the ODP deck carries
  LibreOffice's own clip; its other behaviours are our own.
- Effect names and params are the vocabulary's (`docs/format.md` §5.2); `params` lists what the spec fixes and a frontend may add defaults.
- Shared slide keys agree across the decks except where `conventions.crossFormat` says (PowerPoint's soft wipe,
  speed scales, diagnostic code prefixes); a test checks it.

## Catalog sources (pinned, not committed)

LibreOffice's tables (the ODF presets and transitions, the clip each preset plays, PowerPoint's filter names, the
preset id and sub-type mapping) are kept in `xtask/src/probes_libreoffice.rs` under the MPL-2.0 (see `NOTICE`),
taken from LibreOffice core at tag `libreoffice-25.2.3.2` (`bbb074479178df812d175f709636b368952c2ce3`).
`cargo xtask probes-motion --check-catalog [<dir>]` (and `cargo test -p xtask -- --ignored catalog`) re-checks them
against the files (sha256 in the manifests), fetched with:

```sh
d=~/.slidock/tools/libreoffice-bbb07447; mkdir -p $d
for f in sd/xml/effects.xml sd/xml/transitions.xml sd/xml/transitions-ogl.xml oox/source/ppt/pptfilterhelpers.cxx; do
  curl -sSfL -o "$d/$(echo $f | tr / _)" \
    "https://raw.githubusercontent.com/LibreOffice/core/bbb074479178df812d175f709636b368952c2ce3/$f"
done
```

## Validation schemas (pinned, not committed)

`validate.sh` reads `~/.slidock/tools/specs/` (override with `SLIDOCK_SPECS`):

| Schemas                                       | Source                                                                                                                                                                                  | sha256                                                             |
| --------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------ |
| `ecma376-t/*.xsd`                             | ECMA-376 Part 4, 5th edition (Dec 2016), `OfficeOpenXML-XMLSchema-Transitional.zip` inside `https://ecma-international.org/wp-content/uploads/ECMA-376-4_5th_edition_december_2016.zip` | `bd25da1109f73762356596918bf5ff8b74a1331642dba5f1c1d1dfc6bed34ecd` |
| `opc/*.xsd`                                   | ECMA-376 Part 2, 5th edition (Dec 2021), `OpenPackagingConventions-XMLSchema.zip` inside `https://ecma-international.org/wp-content/uploads/ECMA-376-2_5th_edition_december_2021.zip`   | `1d489dc491168ea1f9e9a59063acc8dd5f02b4ad1d21aa7ec19ba9a58d020c70` |
| `odf13/OpenDocument-v1.3-schema.rng`          | `https://docs.oasis-open.org/office/OpenDocument/v1.3/os/schemas/OpenDocument-v1.3-schema.rng`                                                                                          | `40bad03efdbb02825230d357da0aa6ac679934c5bf56c6281752c0c24d58e4e6` |
| `odf13/OpenDocument-v1.3-manifest-schema.rng` | `https://docs.oasis-open.org/office/OpenDocument/v1.3/os/schemas/OpenDocument-v1.3-manifest-schema.rng`                                                                                 | `8aee71f03484be112af972d622cc9031280c007b0a454ae8815e8e333c9bdd17` |

PPTX parts are validated after Markup Compatibility processing (`validation/mce-fallback.xsl` picks every
`mc:Fallback`), because no public XSD covers `p14`, `p15` or `p159`; the `mc:Choice` branches are checked for
well-formedness only (`validation/mce-choice.xsl`). The malformed-timing slide breaks the PPTX schema on purpose
(`ST_TLTime` is an unsigned int): the manifest names it in `schemaInvalidByDesign` and `validate.sh` requires it to
fail. The ODP deck's malformed values stay schema-valid (`smil:dur` and `smil:begin` are strings).

## Not covered here

- PowerPoint's own output would be the reference for PPTX presets; without PowerPoint, these decks are
  spec-authored. LibreOffice round trips (`soffice --convert-to`) need the Docker image of
  `probes/fonts/` and are pending.
- Keynote and the Google Slides probe are attended steps: `probes/keynote/RUNBOOK.md`,
  `probes/google/RUNBOOK.md`.
