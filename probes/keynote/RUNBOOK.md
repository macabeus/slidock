# Keynote motion catalog (attended)

Keynote's builds and transitions can only be authored in Keynote itself, so this step runs by hand on the Mac with
Keynote 14.5 (`/Applications/Keynote.app`, bundle `com.apple.iWork.Keynote`). It produces the Keynote oracles the frontend and
the bench read: Apple's own mapping of each `apple:*` effect and direction number onto OOXML (the PPTX
export), the build-stage oracle (the "all stages" PDF), and the timing oracle (the QuickTime movie).

Everything goes to `~/.slidock/probes-incoming/keynote/` first. The decks hold only our own text, so once reviewed they
may be committed under `probes/out/keynote-*`; the movies stay in `$SLIDOCK_PRIVATE`.

## 0. Before you start

- Close every Keynote document you care about; the scripts open their own and close them unsaved.
- System Settings → Privacy & Security → Automation: allow your terminal to control Keynote (the first run asks).
- `mkdir -p ~/.slidock/probes-incoming/keynote`

## 1. Transition catalog (scripted)

```sh
osascript probes/keynote/make-transition-catalog.applescript ~/.slidock/probes-incoming/keynote
```

It makes one slide per `transition effect` term (1.5 s, on click), saves `transition-catalog.key` and exports
`.pptx`, `.pdf` (all stages) and `.m4v`. `transition-catalog.tsv` lists, per slide, the term requested, the term Keynote
reports back and the status. A term the running Keynote does not know is logged as `unknown term` and skipped: open
Script Editor → File → Open Dictionary → Keynote → "transition effect" and add any missing names to `effectNames`.

Directions are not scriptable. Duplicate the catalog deck as `transition-directions.key` and, in the Animate inspector,
give each directional effect (push, wipe, move in, reveal, iris in/out, cube, flip, page flip, pivot, swing, switch,
doorway, revolving door, grid, swap, mosaic, object push/cube/flip) one slide per direction the inspector offers
(left to right, right to left, top to bottom, bottom to top, the diagonals where offered, clockwise/counter-clockwise,
in/out). Title each slide `Transition: <effect> <direction as the UI names it>`. Also add:

- `fade through color` with a non-black colour (e.g. orange #FF8000);
- `magic move` between two slides sharing a picture, a shape and a text box that move and resize, once with
  "Fade unmatched objects" on and once off, and once with Text: "By Word" and "By Character" (Anagram-like);
- one automatic transition ("Start Transition: After previous transition", delay 2 s).

Export it with `osascript probes/keynote/export-catalog.applescript ~/.slidock/probes-incoming/keynote/transition-directions.key ~/.slidock/probes-incoming/keynote`.

## 2. Build catalog (by hand)

Builds (direction, chunking, delivery, triggers) are not scriptable. Make `build-catalog.key` (1920×1080, Basic White)
with these slides, each titled with what it holds and each shape labelled with its build (our convention:
`<effect> <direction> <delivery>`, e.g. "move in from left by paragraph"):

1. **Every Build In effect × every direction it offers**, one shape per effect/direction, on click, 1.0 s. Group four
   to six shapes per slide by family (Appear/Dissolve/Fade and Move…, Move In, Wipe, Iris, Scale/Zoom/Pop, Spin/Twirl,
   Drop/Bounce, Typewriter/Keyboard, Compress/Expand, Pivot, Flip/Superflip, Drift and Scale, Blur, Skid, Squish,
   Swap, and the particle effects: Sparkle, Shimmer, Confetti, Lens Flare, Anvil, Flame, Fireworks, Comet …).
2. **Every Build Out effect × direction**, the same way.
3. **Action builds**, one of each: Move (motion path, with "align to path" on one), Rotate (90° clockwise and 180°
   counter-clockwise), Scale (150%), Opacity (30%), Blink, Bounce, Flip, Jiggle (high intensity), Pop, Pulse.
4. **Chunking**: a text box with four paragraphs: "first", an empty line, "second", and "child of second" indented
   one level. One slide per Delivery option the Build inspector offers: All at Once, By Paragraph, By Paragraph Group,
   By Bullet, By Bullet Group (and By Highlighted … if present). Build In: Dissolve.
5. **Granularity and order**: the same text with Dissolve and Move In, "Delivery: By Word" and "By Character", and
   each "Order" (Forward, Backward, From Center, From Edges, Random).
6. **Triggers**: three shapes: build 1 On Click; build 2 "With Build 1", delay 0.5 s; build 3 "After Build 2", delay
   1.0 s; a fourth on click. One more slide whose first build starts "After Transition".
7. **Media**: a movie (any clip of your own, e.g. `probes/motion/media/clip.mp4`) with Start Movie and Stop Movie builds.
8. **Acceleration and timing**: Move In with each Acceleration (None, Ease In, Ease Out, Ease In and Out, Custom).

Save it to `~/.slidock/probes-incoming/keynote/build-catalog.key`, then export:

```sh
osascript probes/keynote/export-catalog.applescript ~/.slidock/probes-incoming/keynote/build-catalog.key ~/.slidock/probes-incoming/keynote
```

## 3. After the run

- `probes/keynote/movie-transitions.sh <movie> <out dir>` measures a catalog movie: where each transition plays, how
  long it lasts, and a contact sheet of each for reading its direction.
- Pin each new file's sha256 in `crates/slidock-keynote/tests/catalog.rs` and hold what it settles there, beside the
  readings it confirms already.

- `slidock compile <deck>.key --emit ir` and read each slide's `transition.effect.id` / timeline `effect.id`: that pins
  each UI name to its `apple:*` id. The PPTX export pins each id and `database_direction` number to OOXML
  (`p:transition` children, `p:timing` presets): compare slide by slide with `motion-probe.expect.json`'s PPTX rows.
- Record the Keynote version (`defaults read /Applications/Keynote.app/Contents/Info CFBundleShortVersionString`), the
  date and the sha256 of every file in `~/.slidock/probes-incoming/keynote/manifest.json`.
- The "all stages" PDF page count per slide must equal the IR's click-group count + 1, plus a
  lead page when the slide's first build starts by itself.
