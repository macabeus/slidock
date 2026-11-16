# Callout probe decks

Spec-authored decks for callout tails: 960 × 540 pt slides, four callouts per slide, each slide named by a 12 pt
Liberation Sans label in its top-left corner. All drawings are our own. `xtask/src/probes_callouts.rs` lists every
slide.

| File (`probes/out/`) | Slides | What                                                                                       |
| -------------------- | ------ | ------------------------------------------------------------------------------------------ |
| `callout-probe.pptx` | 14     | P01–P14: ECMA's callout presets, tails on every side, inside, far out, on the diagonals, … |
| `callout-probe.odp`  | 10     | O01–O10: LibreOffice's own callout types, drawn from enhanced paths and equations          |

The ODP shapes carry LibreOffice's type names (`rectangular-callout`, `round-rectangular-callout`, `round-callout`,
`cloud-callout`, `line-callout-1`–`3`) with paths and equations of our own, as LibreOffice writes every such shape
with its path: LibreOffice draws them from the file, and so does slidock.

## Regenerate

```sh
cargo xtask probes-callouts            # writes the two decks
cargo xtask probes-callouts --check    # fails when a committed deck differs (the xtask tests run the same check)
```

## LibreOffice oracles

```sh
slidock-bench fonts link <links> --releases && mkdir <fonts> && cp -L <links>/LiberationSans-*.ttf <fonts>/
probes/callouts/oracle.sh <fonts> <out>   # LibreOffice 25.2.3.2 (slidock-probes-lo:trixie-20261030), no network
```

Each PDF must name only Liberation Sans (`pdffonts`). Its bytes change with every export; its pages rasterise the same.

What LibreOffice 25.2.3.2's PDF shows, its vector outlines read with `pdftocairo -svg` (LibreOffice draws the page
through one `matrix(0.998969, 0, 0, -0.998969, 0, 539.72)`, undone before comparing):

- PPTX: every callout preset's outline (wedge, round-rect, ellipse and cloud bodies with their tails and bubbles, the
  line callouts' leaders and accent bars, turned and mirrored; 97 outlines) lies within 0.13 pt of slidock's, which
  draws ECMA's formulas. The PPTX frontend has no callout gap.
- ODP: every outline lies within 0.11 pt of slidock's evaluation of the file's own path. `T`/`U` take their angles
  clockwise from 3 o'clock on the page and run clockwise (O08: `0 180` is the lower half); `W`/`V` run clockwise and
  `A`/`B` counter-clockwise between the ellipse's points on the two rays; `X`/`Y` alternate with each point.
- A filled path that is open (a line callout's leader without `F`) is filled as if closed and stroked open, as SVG
  does.
