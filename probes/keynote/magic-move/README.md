# Magic Move probe (Keynote, attended)

Four 1024 × 768 slides that hold what Keynote's Magic Move does with matched objects at the slide's edges,
built in Keynote 14.5 by `build-minimal.applescript` from `pic-a.png` and `pic-b.png`:

1. Two text lines, a picture, and a panel off the slide to the right (x ≥ 1080): a 30 % backdrop, three labelled boxes
   ("Code generation" at 50 %) and two lines.
2. The panel 560 pt left, on the slide; the text 30 pt up; the picture swapped for another (unmatched both ways).
3. "Code generation" at 100 %.
4. The panel back off the slide.

Every transition is a Magic Move of 2 s, in-out, fading unmatched objects.

```sh
osascript probes/keynote/magic-move/build-minimal.applescript /tmp/magic-move-probe.key \
  "$PWD/probes/keynote/magic-move/pic-a.png" "$PWD/probes/keynote/magic-move/pic-b.png"
osascript bench/scripts/export-keynote-movie.applescript /tmp/magic-move-probe.key /tmp/magic-move-probe.pdf \
  /tmp/magic-move-probe.m4v
```

The deck and its exports are Keynote's files and stay private: the corpus's `magic-move-probe` (sha256 `161f3a4a…`)
was built by this script with two other short text lines. Its movie is the motion lane's oracle of the deck's
transition rows; `slidock-bench oracle --movie --record` pins it.
