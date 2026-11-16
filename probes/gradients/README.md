# Gradient probe decks

Spec-authored gradient decks: 960 × 540 pt slides, one case per slide, each named
by a 12 pt Liberation Sans caption in its top-left corner that gives the case's parameters. Unless a case says
otherwise the object is a 600 × 200 pt box at (180, 170), filled black → white so that the drawn luminance is the ramp
parameter t, on a #BFC9D4 slide. `xtask/src/probes_gradients.rs` lists every slide.

| File (`probes/out/`)        | Slides | What                                                                       |
| --------------------------- | ------ | -------------------------------------------------------------------------- |
| `gradient-probe.pptx`       | 43     | G01–G43: `lin`, stops, alpha, turns, flips, paths, tiles, cells, themes    |
| `gradient-probe.odp`        | 41     | O01–O41: every ODF style, borders, steps, stops, transparency, outlines    |
| `gradient-probe-odp12.odp`  | 2      | N01–N02: ODF 1.2 by LibreOfficeDev 6.0.5.2 (unitless angles are tenths)    |
| `gradient-probe.cases.json` |        | per slide: parameters, LibreOffice validity, diagnostics, PowerPoint maths |

The cases file gives every slide's id, caption, row id and parameters, whether LibreOffice is a valid oracle for it,
the diagnostics the compiler should report on it, and for the PPTX slides PowerPoint's analytic geometry in box
fractions (linear ends, a circle's centre, radii and focus, a rect path's frame and inner rectangle).

## Regenerate

```sh
cargo xtask probes-gradients            # writes the three decks and the cases file
cargo xtask probes-gradients --check    # fails when a committed file differs (the xtask tests run the same check)
```

## LibreOffice oracles

```sh
slidock-bench fonts link <links> --releases && mkdir <fonts> && cp -L <links>/LiberationSans-*.ttf <fonts>/
probes/gradients/oracle.sh <fonts> <out>   # LibreOffice 25.2.3.2 (slidock-probes-lo:trixie-20261030), no network
```

The script also writes `pdf/pptx/gradient-probe.lo-valid.pdf`: the pages of the slides the cases file marks
LibreOffice-valid (G01, G02, G04–G11, G13–G15, G17–G19, G33, G36–G38, G40, G42, G43), cut with poppler's `pdfseparate`
and `pdfunite`. It is the oracle of the `gradient-probe-pptx` bench entry, whose rows are those slides only: on the
other slides LibreOffice is not PowerPoint (it ignores `scaled`, spans a turned shape's page box, turns a corner focus
into a 45° linear gradient, draws rect and shape paths as equal-inset contours, and ignores tiles), and a row there
would reward its errors. PowerPoint's own geometry for them is the `powerpoint` object of the cases file.

Each PDF must name only Liberation Sans (`pdffonts`). Its bytes change with every export; its pages rasterise the same.

What LibreOffice 25.2.3.2's PDF shows, measured on these decks (72 and 144 dpi rasters of `pdftoppm`):

- G38: the circle cell is PowerPoint's centred circle, t = d / (half the cell diagonal), RGB rms 0.0034 (max 0.0079)
  over the cell, as on G18's shape; a table cell that names no borders gets thin black ones, so the cells name none.
- G16: a horizontal flip with `rotWithShape="0"` mirrors the gradient as with `rotWithShape="1"` (G14).
- G25, G26: the corner presets are drawn as 45° linear gradients; G35's `tileRect` and `flip` are ignored; G41's line
  and text are drawn in their first colour.
- O07, O08: a mirrored shape's gradient is not mirrored (O07 draws as O02, O08 as O01).
- O40: a `draw:path` keeps its view box scale, but one whose bounds do not start at the view box's origin is moved to
  its frame's corner, so the probe's path starts there.

## Keynote transform gradients (K2)

Keynote's import of `gradient-probe.pptx` (K1, `gradient-probe-keynote.key`, saved by Keynote 14.5) writes angle
gradients only. `keynote_transform.py` patches a copy of it: on 16 of K1's styles it writes a transform gradient
(start, end and base size) in place of the angle, inflections, an opacity, a P3 stop, alpha stops, the advanced flag
with an angle, and a gradient with neither (cases T01–T17 of `keynote_transform.py`, by the style each patches; a style shared
by several slides moves all of them, and `--cases` lists which). It reads the protobuf wire format without a schema,
so every field it does not edit keeps its bytes; it re-frames the IWA files it changed with literal-only Snappy and
fixes their MessageInfo lengths. Python 3 standard library only.

```sh
python3 -I probes/gradients/keynote_transform.py --round-trip K1.key   # every IWA file rebuilt unchanged: identical
python3 -I probes/gradients/keynote_transform.py K1.key K2.key --cases K2.cases.json
python3 -I probes/gradients/keynote_transform_test.py                  # the same on a synthetic package
```

Only a person opens K2 in Keynote 14.5 (to see that Keynote accepts it), saves it as `gradient-probe-transform.key`
and exports its PDF oracle.
