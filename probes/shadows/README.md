# Shadow probe decks

Spec-authored shadow decks: 960 × 540 pt slides, one case per slide, each named by a
12 pt Liberation Sans label in its top-left corner. All drawings are our own. `xtask/src/probes_shadows.rs` lists
every slide.

| File (`probes/out/`)      | Slides | What                                                                                  |
| ------------------------- | ------ | ------------------------------------------------------------------------------------- |
| `shadow-probe.pptx`       | 46     | A01–A46: direction, distance, blur, colour, carriers, rotation, flips, groups, cost   |
| `shadow-probe.odp`        | 46     | its twin: the same numbers as ODF shadow attributes (`loext:shadow-blur` for blur)    |
| `shadow-probe-odp.odp`    | 16     | B01–B16: LibreOffice-only semantics (style chains, placeholders, `fo:text-shadow`, …) |
| `shadow-probe-ooxml.pptx` | 25     | C01–C25: OOXML-only cases (`effectRef`, inner and preset shadows, scale, skew, …)     |

## Regenerate

```sh
cargo xtask probes-shadows            # writes the four decks
cargo xtask probes-shadows --check    # fails when a committed deck differs (the xtask tests run the same check)
probes/shadows/make-media.sh          # only to change media/: photo.jpg, clip.mp4, poster.png (then regenerate)
```

## LibreOffice oracles

```sh
slidock-bench fonts link <links> --releases && mkdir <fonts> && cp -L <links>/LiberationSans-*.ttf <fonts>/
probes/shadows/oracle.sh <fonts> <out>   # LibreOffice 25.2.3.2 (slidock-probes-lo:trixie-20261030), no network
```

Each PDF must name only Liberation Sans (`pdffonts`). Its bytes change with every export; its pages rasterise the same.

What LibreOffice 25.2.3.2's PDF shows, measured on these decks (1920 × 1080 rasters of the bench's renderer):

- LibreOffice's PPTX import makes every box 1/100 mm smaller than the same box read from ODF (`8.466cm` against
  `8.467cm` for 240 pt), so the twins' PDFs differ by a fraction of a pixel at every edge: per-page SSIM ≥ 0.999 on 43
  pages, 0.9978–0.9988 on A32, A34 and A46 (no page is byte-identical). Each twin is scored against its own PDF.
- A blurred shadow of an opaque bitmap (the JPEG of A21, an opaque PNG) is not drawn; at blur 0 it is. A PNG with
  alpha keeps its blurred shadow.
- A video (A36) casts no shadow.
- A filled text box's text casts its own shadow, hidden under the box except where the text overflows (A28).
- B13: a `draw:g` with a shadow style draws none. C02: the theme's shadow is drawn despite an empty own `a:effectLst`;
  C06–C09, C11, C18, C20 and C21 draw none; only `sx` scales (C12 wider, C13 unscaled); C14's skew is ignored; C22
  (`a:effectDag`) is drawn.
