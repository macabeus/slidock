# Google Slides motion probe (attended)

One Google Slides deck feeds the PPTX and ODP frontends from the same source, which gives cross-format motion checks,
and its PDF export is the static oracle. Google Slides has no automation, so this is done by hand in the maintainer's
Google Slides probe deck.

Use only our own text. Label every shape with what it carries, in the convention of the generated decks
(`probes/out/motion-probe.*`): `<effect> <direction> <delivery> <trigger>`, e.g. "fly in from left, by paragraph,
after previous". Title each slide with its section.

## Slides to add

1. **Title** — "slidock Google Slides motion probe", no motion.
2. **Transitions** — one slide per transition Google Slides offers (Slide → Transition), each with a big labelled box
   and the default duration, then one more with the slowest and one with the fastest duration:
   None, Dissolve, Fade, Slide from right, Slide from left, Flip, Cube, Gallery (and any newer entry the menu shows).
   Leave "Apply to all slides" off.
3. **Entrance animations** — one shape per animation, On click, default duration: Appear, Fade in, Fly in from left,
   Fly in from right, Fly in from bottom, Fly in from top, Zoom in, Spin.
4. **Exit animations** — one shape per animation: Disappear, Fade out, Fly out to left, Fly out to right, Fly out to
   bottom, Fly out to top, Zoom out, Spin.
5. **Emphasis** — Spin (on a shape that stays).
6. **Triggers** — four shapes: Fade in On click; Fly in from left "With previous"; Zoom in "After previous"; Fade in
   On click. Google Slides has no delay field; note that on the slide.
7. **By paragraph** — a text box with four paragraphs ("first", an empty paragraph, "second", "child of second"
   indented one level), Fly in from bottom, "By paragraph" checked, On click. A second box with the same text and
   "By paragraph" + "After previous".
8. **Durations** — Fade in at the slowest (5 s) and fastest (0.1 s) the slider allows.
9. **Fonts** — six text boxes, each naming its face: Roboto 400 and 700 italic, Lato 300 and 900, Merriweather 400,
   Oswald 500, Caveat 400, Source Code Pro 400 (the fonts-probe families plus the corpus families). Give two of them
   an entrance animation so a build and a webfont meet on one slide.

## Exports

File → Download, three times, saved as:

```
~/.slidock/probes-incoming/google/google-motion-probe.pptx   (Microsoft PowerPoint)
~/.slidock/probes-incoming/google/google-motion-probe.odp    (ODP Document)
~/.slidock/probes-incoming/google/google-motion-probe.pdf    (PDF Document)
```

Then record the export date and each file's sha256 in `~/.slidock/probes-incoming/google/manifest.json`
(`shasum -a 256 ~/.slidock/probes-incoming/google/*`).

## What the frontends check with it

- The PPTX and ODP exports of one deck must lower to the same vocabulary effect, direction (the way the content
  moves), trigger, delivery and paragraph ranges per shape: the same cross-format agreement `motion-probe.expect.json`
  asserts for the generated decks.
- Google's PPTX export writes `p14`/`p15` transitions for Flip, Cube and Gallery inside `mc:AlternateContent`; record
  which choice and fallback it writes. Its ODP export goes through Google's own writer, not LibreOffice: note any
  preset id or sub-type it writes that LibreOffice's `effects.xml` does not list.
- The PDF is the static oracle (every slide fully built).

The expectations are `probes/attended/google-motion-probe.expect.json` (the exports pinned by SHA-256, what both
formats read per shape and transition, and why Google's two writers differ where they do), checked by
`crates/slidock-pptx/tests/google_probe.rs` and `crates/slidock-odp/tests/google_probe.rs`, which skip when the
private files are absent.
