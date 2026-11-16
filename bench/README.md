# bench

Public configuration of `slidock-bench` (`crates/slidock-bench`), slidock's fidelity bench: it compiles a corpus of
decks, renders every slide in a pinned headless Chromium, and scores each render against an oracle (the authoring
app's own PDF export). Nothing here is corpus-derived: the corpus, its manifest, the oracles, the cache, the runs and
the baseline are private and live under `$SLIDOCK_PRIVATE` (default `~/.slidock`). The bench is a maintainer tool;
contributors do not need it (see `CONTRIBUTING.md`).

## Files

- `docker/`: the renderer recipe: the pinned Playwright 1.60 base image (linux/arm64), poppler 24.02, the headless
  shell started with Playwright's `headless: true` switches, a fontconfig that sees only `/fonts`. Its sha256 is the
  stamp's `renderer.recipe`.
- `docker/no-fonts/fonts.conf`: the fonts lane's renderer variant: the same image with a fontconfig that sees only the
  Last Resort face at `/fonts`.
- `fonts/fonts.lock.json`: the pinned font set the static lane renders with: open-licensed release files, and fonts
  of the reference Mac (`host` entries, copied from their paths by `fonts sync`, never committed, and no path of
  theirs reaches a report). A family a corpus deck names and the set lacks shows in the compile reports as MID5401 or
  MID5402; the Mac's file of that family goes in as a `host` entry. The goldens compile with the set, so a change to
  it takes a new recording (`cargo xtask golden --record`) as well as a new baseline.
- `fonts/fonts-lane.lock.json`: the fonts lane's pinned files: Google Fonts as the CLI fetched it (each `css2`
  response and every `fonts.gstatic.com` file it names, with sha256), the bundled substitutes, and Unicode's Last
  Resort face. Open licences only, no host font.
- `fonts/probes.json`: the probe decks of `probes/out/` the fonts lane renders beside the corpus, by sha256.
- `fonts/fallback-allow.json`: reviewed fonts-lane rows whose glyphs the renderer's fallback draws: the glyph count
  reviewed, the reason, and where the fix is tracked.
- `motion/probes.json`: the motion probe decks of `probes/out/` the motion lane plays beside the corpus, by sha256,
  with their expectations file and deck key.
- `motion/fallback-allow.json`: reviewed motion items: `entries`, an item that expects an effect and plays another
  (`played`: `fallback:<name>` or an effect), and `checks`, a failed check a review accepts; each with the reason and
  where the fix is tracked.
- `motion/lo-roundtrip.json`: what LibreOffice 25.2.3.2 keeps of the motion probes saved as ODP and PPTX
  (`slidock-bench lo-roundtrip`): per round trip, fates by effect and every item not kept.
- `scripts/export-keynote.applescript`: Keynote's PDF export of one deck (`all stages:false`, or `all stages:true`
  with `all-stages`), run by the attended `oracle` command.
- `budgets.json`: the size and compile budgets `budgets --check` holds (`slidock-budgets@1`), from `budgets --record`
  (see Budgets below); the player's gzip budgets are the `BUDGET_BYTES` of the player's build test
  (`packages/player/test/build.test.ts`), which `--record` reads and a unit test holds the committed entry to.
- `parity/allow.json`: reviewed divergences from the goldens (`parity/README.md`).
- `public.json`: the presentations the public report may publish (`slidock-bench-public@1`), by id; see Publishing
  the public report below.
- `wasm-cli.mjs`: the determinism gate's wasm leg: the compiler built to WebAssembly (`pnpm wasm:build`) under Node,
  writing the document set and the deck zip.

## Private layout

Each path is under `$SLIDOCK_PRIVATE`; the variable in brackets overrides it.

- `corpus/` (`SLIDOCK_CORPUS_DIR`): the corpus files. A manifest `file` is a name hint: files resolve by sha256.
- `bench/corpus/manifest.json` (`SLIDOCK_CORPUS_MANIFEST`): the corpus manifest (`slidock-corpus@1`).
- `bench/oracles/keynote/<sourceSha>/<exporterId>.pdf` (`SLIDOCK_KEYNOTE_ORACLES`): Keynote's exports.
- `oracles/keynote-stages/<sourceSha>/<exporterId>-all-stages.pdf` (`SLIDOCK_STAGES_ORACLES`): Keynote's all-stages
  exports, the motion lane's stage oracles.
- `bench/cache/` (`SLIDOCK_BENCH_CACHE`): fonts, oracle copies, rasters and words (mounted into the renderer at
  `/work`), GIF first frames, runs.
- `bench/baseline/results.json` (`SLIDOCK_BENCH_BASELINE`): the static baseline.
- `golden/<commit>/`: slidock's goldens, recorded by `cargo xtask golden --record` and pinned by
  `tests/golden/manifest.json`; parity reads each corpus pair's static-lane output from
  `corpus/<presentation>.<frontend>/`.
- `golden/scoring/`: the metrics' corpus goldens: each row's scoring inputs and scores, pinned by
  `crates/slidock-bench-metrics/tests/golden/scoring.json` and by `tests/golden/manifest.json`.

The repository names a corpus deck only by its presentation id: the manifest maps each id to its source files, and
the goldens refer to a source as `corpus:<presentation>.<frontend>`.

## Bring your own corpus

The bench runs on any set of decks you have the right to use. Nothing of the corpus enters the repository.

1. Put the decks (`.pptx`, `.odp`, `.key`) and, for each, the oracle PDF its authoring app exports, in
   `$SLIDOCK_PRIVATE/corpus/`.
2. Write `$SLIDOCK_PRIVATE/bench/corpus/manifest.json` (`slidock-corpus@1`): per presentation a neutral `id`, its
   `tier` (`smoke`, `real`, `probe` or `holdout`), its `features`, its `sources` (frontend, file, sha256, bytes, slide
   ids as `rows`) and its `oracle` (`pdf-file` with the exporter, or `keynote-export`). The schema is
   `crates/slidock-bench-schema/src/corpus.rs`, and `crates/slidock-bench-schema/tests/data/manifest.json` is a
   complete example. `slidock-bench corpus locate` prints where each file resolves.
3. `slidock-bench fonts sync`, `slidock-bench renderer start`, then `slidock-bench oracle --record` rasterises every
   oracle and records its page hashes in the manifest; `slidock-bench run` scores the corpus.

Without oracles, `run` classes every row `oracle-missing`, but the determinism gate, `budgets`, and the fonts, motion
and shell lanes still run (the motion lane leaves Keynote's stage checks pending). Parity needs a golden recording of
the same corpus.

Platform limits:

- The renderer container is built for linux/arm64 (Apple silicon, or an arm64 Linux host with Docker); elsewhere,
  use `--renderer native` where a lane offers it.
- Scores are bit-identical only on macOS arm64: the metrics' raw scores go through the host's libm. The quantised
  records and classes match on every platform.
- Keynote oracles need Keynote 14.5 on macOS, at an interactive session.

## Commands

```sh
slidock-bench fonts sync                                         # materialise the pinned set, every hash checked
slidock-bench renderer start                                     # build (first use) and start the renderer; prints its identity
slidock-bench oracle --record                                    # rasterise and record every oracle
slidock-bench oracle --stages [--record]                         # Keynote's all-stages exports (attended), pinned by page rasters and pagesPerRow
slidock-bench run [--only <row>,…] [--presentation …] [--frontend …] [--tier …] [--class …]
slidock-bench baseline                                           # a clean full run, written as the baseline
slidock-bench status
slidock-bench parity reference                                   # render and score the goldens' static-lane output
slidock-bench parity report | check [--strict]                   # B + C: a run against the golden reference
slidock-bench regression [--base baseline] [--results latest]    # acceptance against the baseline (exit 0/1/2/3)
slidock-bench diff [--base baseline] [--results latest]          # every field that moved
slidock-bench show <row> | list [--class …] | histogram [--all]
slidock-bench explain <row> [--from latest]                      # per-element findings and pictures
slidock-bench determinism [--score]                              # native CLI vs wasm under Node: the same bytes
slidock-bench run --lane host [--from latest] [--end-state]       # each row mounted by the player vs standalone
slidock-bench run --lane fonts [--renderer container|native]    # each deck with its fonts on; every glyph attributed
slidock-bench fonts lane-lock                                    # record the fonts lane's Google stand-in, for review
slidock-bench run --lane motion [--renderer container|native]   # each deck played by its own player; every transition and build checked
slidock-bench run --lane shell [--from <motion run>]             # each deck's zip opened from file://: the motion run's frames, no request
slidock-bench ui [--port 5290] [--dist apps/bench-web/dist]      # the bench UI over the private data, on 127.0.0.1
slidock-bench report --single-file [--images mismatch]           # the UI as one HTML file with the data inlined
slidock-bench report --public [--out apps/web/public/bench]      # the public report: bench/public.json's decks only
slidock-bench lo-roundtrip [--check]                             # the motion probes through LibreOffice (Docker): bench/motion/lo-roundtrip.json
slidock-bench gc [--keep 5]                                      # old runs out of the cache (every run collects past 20 GiB)
slidock-bench budgets [--check | --record] [--runs 5]           # wasm and player sizes, compile time and peak memory (exit 3: loaded)
slidock-bench renderer stop
```

`--base` and `--results` take `baseline`, `latest`, a run key, or a path: the baseline is private, not in git.

`run` builds the working copy's compiler (`cargo build --release --locked -p slidock-cli`) unless `SLIDOCK_COMPILER`
names a binary, and compiles each (presentation, frontend) with `slidock compile --emit documents,report,ir`.

### Re-recording the static baseline

A baseline names a commit and a harness: `regression` refuses to compare a run with a baseline whose harness hash
differs (`incomparable: harness differ`). The harness hash covers `crates/slidock-bench*/src` and their
`Cargo.toml`, `bench/docker` and `bench/fonts/fonts.lock.json`, so a merge that changes any of them is followed by a
new baseline, on main, at the merge commit:

1. Check out the merge commit with a clean tree: `baseline` refuses a compiler or bench that differs from HEAD.
2. Keep the current baseline beside the new one: copy `$SLIDOCK_PRIVATE/bench/baseline/results.json`
   (`SLIDOCK_BENCH_BASELINE`) to `results.<its compiler commit>.json`.
3. On a fresh cache, `slidock-bench fonts sync` (and `renderer start`); the static lane needs the pinned set.
4. `slidock-bench baseline`: the determinism gate with `--score` first (a failed gate refuses the baseline), then a
   clean full run of the default tiers (no `--only`, `--presentation` or `--frontend`), written to the run and to
   `results.json` with the gate's report hash stamped.
5. Review what moved against the kept file: `slidock-bench diff --base <kept file> --results baseline`. It lists every
   field that moved and exits 2 when the stamps differ, as they do across harnesses. A row whose class moved is a
   finding to explain, not a baseline to keep.
6. `slidock-bench regression --base baseline --results latest` now compares (exit 0).

## Keynote oracles (attended)

A Keynote deck's oracle is Keynote's own PDF export, made only by `slidock-bench oracle`, at a logged-in, interactive
session: Keynote opens on screen, and a large deck takes minutes. `run` never exports; it reads the export on disk
and classes a missing one `oracle-missing`.

1. The first export asks for the Automation permission (System Settings → Privacy & Security → Automation: allow
   your terminal to control Keynote). Without it the probe fails and the command says so.
2. The command copies the deck into the export folder and exports the copy with Keynote 14.5
   (`com.apple.iWork.Keynote`; Keynote Creator Studio only for a file 14.5 refuses), `all stages:false`, best image
   quality, skipped slides out, then closes it unsaved. The corpus file is never opened.
3. It quits Keynote afterwards if it launched it, with SIGTERM if Keynote ignores `quit` for 30 s, and removes the
   renderer container it started for itself (passed, failed or erred). It reuses the cache's shared renderer when
   that one runs, and leaves it running: another command may be using it. Whether an app
   runs is read from the process list, since LaunchServices answers `false` for every app inside a sandboxed
   session, and the app's version is read from its `Info.plist` without launching it (AppleScript's
   `path to application id` would).
4. `pdffonts` must find every PostScript name the manifest lists for the deck; a missing one means Keynote
   substituted a font (`oracle-font-missing`).
5. `--record` writes the export's sha256 and the page rasters' sha256s into the manifest. A PDF export carries its
   creation date, so a fresh export of an unchanged deck has a new sha256; `--record` accepts it when every page
   raster equals the rasters the manifest records under the same raster key, and `--accept-oracle` otherwise.

## Lanes and gates

- **Renders** (the static lane, `explain`, `parity reference`, and the host lane's mounts): each screenshot waits
  for two of the page's own animation frames, so the page has drawn its last change (fonts and images loaded, the
  glyphs painted out for the textless twin) before the capture. A capture that forces the frame drawing a change
  makes Chromium's software raster of that change vary between renders: a shape edge near the change, such as a
  stroke's outermost anti-aliased column, comes out a few levels off, and stays so for the page's later screenshots.
  Rastered on a frame of the page's own, the change draws as a fresh load of the changed page draws it.
- **Determinism** compiles every pair natively twice (manifest order, `--verify boundaries`, `TZ=UTC LANG=C`; then
  reverse order, `--verify each`, `TZ=Pacific/Chatham LANG=tr_TR.UTF-8`) and with the wasm build under Node; the
  document sets, reports, IR and zips must be the same bytes (a deck over 512 MiB is `native-only`). Every leg binds
  against the bench's font folder, as `run` does: `--fonts` natively, and `wasm-cli.mjs` adds the folder's files to
  the wasm compiler one by one in the order the CLI reads them. The gate builds the working copy's wasm
  (`wasm-pack build crates/slidock-wasm`, as `pnpm wasm:build` does) unless `SLIDOCK_WASM_PKG` names a package, and
  the report records the sha256 of the native binary and of the wasm module. Three `--only` rows per pair must equal
  the whole compile's slide and layout files; `--score` scores every row twice in opposite orders, the second pass on
  the first's compile, which must equal leg A's; no JSON or HTML output, in a folder or inside a deck zip, may name a
  private or temporary path. The work folder is removed when the gate ends. `baseline` runs it with `--score` first
  and stamps the report's hash. Reports: `<cache>/determinism/<sha256>.json`.
- **Budgets** measures what the compiler costs to ship and to run: the wasm module `pnpm wasm:build` writes (raw, and
  brotli quality 11 as a server sends it), the player's IIFE and ESM builds (raw, and gzip level 9 with Node's zlib, as
  the player's build test measures it), and per pair the compile to a zip, `--runs` times on each leg: the native CLI
  (`slidock compile <deck> -o deck.zip --offline`, in a helper process whose `getrusage` children usage is the
  compile's: its CPU time, user and system, and its peak resident set) and the wasm module in the host's headless shell
  through the app's own compile worker (the production build in `apps/web/dist`, served on a loopback port; the deck
  reaches the page as a `File` and the worker runs `open`, `finish`, adds the bundled substitutes the deck draws,
  `finish` again and `bundle`, timed as the CPU time of the shell's renderer processes from the worker's `ready` to the
  zip, then answers `memory` with its linear memory, which only grows). Each time is the fastest run, each peak the
  largest. The app's build must hold the measured module. A measurement taken on a quiet machine (the load bound
  below) goes to `<cache>/budgets/<content id>.json` and into the stamp (`perf`) of the run `--run` names (`latest` by
  default) when that run's compiler is the working copy's; it is stamped after the run, so the run key stays. One taken
  above the bound is only printed, so no file holds CPU times that were not judged. `--check` holds every value to `budgets.json` and prints
  the machine's 1-minute load average (`sysctl vm.loadavg`, `/proc/loadavg`; one that does not parse is refused), the
  highest of a reading before the compiles and one after each pair. It exits 1 when a value passes its budget, or a
  selected pair has no budget or was not measured, and the last line names each value over. A budget its own measurement
  noise exceeds is a wrong rule, so the CPU times are judged only on a quiet machine: with the load above the core
  count, `--check` marks each time `----`, still judges every size and peak, and exits 3 (1 when a size or peak is
  over), its last line saying how many times went unjudged and at which load. `--record` writes the file: each compile's
  CPU time plus 20 % and 30 ms, its peak plus 5 % and 8 MiB, each size plus 5 %. It measures with every core but two
  kept busy (a quiet machine runs its cores at a higher clock, which would make the budgets too tight), and refuses to
  record (exit 3) when the load is above the core count before it starts them, or above the cores and those threads
  while it measures. The 20 % holds what other processes add below that bound (two daemons holding a core add up to
  about 17 %). The 30 ms covers the spread of a small deck's fastest runs, which is more than its 20 % (two 10 ms
  steps); the 8 MiB the spread of a small deck's native peaks. Both times are CPU time because a loaded machine
  stretches the wall clock; macOS `ps` reads the renderers' CPU time in 10 ms steps, and the wasm leg refuses a `ps` that prints whole seconds
  (Linux procps).
- **Host** bundles each pair (`slidock compile -o <folder>`), loads its `index.html` in the renderer with JavaScript
  on, and mounts each row with the deck's own player (`Slidock.renderStatic`, a shadow root, fitted to the width) at
  the standalone render's size; the PNG must equal the static run's render. The standalone document shows every
  element, the player's end state hides what an exit build removed, so the mount clears the build state;
  `--end-state` keeps it (the motion lane's concern), which differs exactly on the slides with exit builds.
- **Parity B and C** compare a run with slidock's goldens. `parity reference` reads the set the committed manifest
  pins (every file checked against its sha256 first; a missing or changed file refuses the set), renders and scores
  each corpus pair's golden static-lane output with this bench into `<cache>/parity/golden-<commit12>/`, and writes
  its `reference.json` there, keyed under the bench's renderer recipe, font set, metric versions and classifier.
  `parity report | check` compare a run with it: per row the composed document's sha256, the render's sha256, the
  class and erosion guards, render-vs-render scores where renders drift, and the first differing IR path against
  the golden IR; per IR container, equal or the first differing path. A reference rendered from another golden
  manifest is refused. `runs/<key>/parity.json`, hashed into the run's stamp with the golden's commit and manifest.
- **Fonts** (`run --lane fonts`) compiles each corpus pair and each probe deck of `fonts/probes.json` with its fonts on
  and checks that the deck draws every glyph with a face it ships. The CLI's deck path runs with its default sources:
  the fonts embedded in the source, then Google Fonts, then the bundled substitutes; installed fonts stay off, so no
  host font ships. Google is a pinned stand-in: the CLI looks every URL up in its cache (`SLIDOCK_CACHE_DIR`,
  `google/<sha256 of the URL>`) before fetching it, and the lane fills that cache with the `css2` responses and
  `fonts.gstatic.com` files of `fonts/fonts-lane.lock.json`, so the compiler's own `css2` plan gets the static faces
  live Google serves, as `google`; the substitutes sit in the same cache. Every proxy variable points at a closed
  port, so a request the lock does not answer fails instead of reaching the network, and the lane fails on it
  (`unpinned`). `fonts lane-lock` records the lock: each lane deck compiled with no network lists its `css2`
  requests, compiled with the network lets the CLI fetch them into a fresh cache, and compiled with no network again
  must make no request; the lock then pins every response and file in that cache, with their sha256. Each row is
  mounted from the bundled `index.html` as the host lane mounts it, in the renderer image with the `no-fonts`
  fontconfig (only the Last Resort face), or with `--renderer native` in the same Chromium build on the host
  (`SLIDOCK_BENCH_CHROMIUM`, else Playwright's install), where the host's own fonts are the fallback. Every family the
  slide names and the deck ships no face of gets a face that maps no character, so an installed font of that name
  never draws: text falls through its stack as in a renderer without it. `CSS.getPlatformFontsForNode` on every text
  node of the deck's text (not slidock's own labels in their generic font: a foreign object's marker such as "Image
  not included", a failed container's code) then gives, per glyph, the face that drew it: `exact` (a shipped face of
  the requested family, any source), `substitute` (a bundled face standing in for another family), or the renderer's
  `fallback`; a glyph not drawn by the first shipped family of its stack is a coverage gap. The colour-emoji exception: an emoji sequence (a base with
  its VS16, skin-tone modifier, tags or keycap, and pictographs joined by ZWJ) is one glyph from a face that shapes it
  but one box per code point from Unicode's Last Resort (the container's only face) or the shadow face. A sequence
  was boxed when every other face that drew its text node is a shipped face whose file lacks the sequence's base, and
  the boxing face drew at least its code points; it then counts once, so both renderers count `🙋‍♀️` as one. Any
  other face drawing the node (a colour emoji font, any renderer font) may have shaped it, and the node's counts stay
  as drawn, so Last Resort boxes beside a shaped emoji still count one by one. A row is `fonts-complete` with no fallback glyph and
  `fonts-fallback` otherwise; the lane fails on a `fonts-fallback` row unless `fonts/fallback-allow.json` lists it
  with exactly its fallback glyph count, and on an entry whose row no longer falls back. Per pair the results also
  list the shipped faces (source, PostScript name, WOFF2 size and sha256), the faces drawn with no file, the glyphs
  per family and source, and the compile's MID54xx notices. `regression` between two fonts runs moves rows by fonts
  class and guards a row's exact share (−0.001), its fallback glyphs and coverage gaps, and a pair's WOFF2 bytes
  (+10 %) and files. The bench UI's Fonts tab reads the selected fonts run, else the newest, and names the renderer
  the run used.

- **Motion** (`run --lane motion`) compiles each corpus pair and each probe deck of `motion/probes.json` on the deck
  path (fonts pinned as the fonts lane pins them, no network) and its IR (`--emit ir --native-motion`, no network, with
  the CLI's built-in fonts only: the lane reads its timelines, which no face changes, so a fresh cache needs no
  `fonts sync` for it; a pair whose IR does not compile or read fails the lane), and plays each deck with its own player in the renderer with JavaScript on
  and `prefers-reduced-motion: no-preference`. A script evaluated before the deck's own takes the page over: the deck's
  player is mounted presenting at the deck's pixel size with keys, pointer, chrome and auto-advance timers off; media
  never play and a video shows its poster as an image; every Web Animation the player creates is recorded and paused as
  it is created. The lane cuts to the slide before a run of 24 shown slides and moves on through them, so each slide is
  entered by its transition, and clicks through every step. No wall clock decides a frame: each batch of animations the
  player makes at once is seeked (a transition to 0, 250, 500, 750 and 1000 ‰, a stage's group to its start, middle and
  end, and each step of it to its own start, quarters and end), screenshotted once the frame holds still, with what
  every animated node shows then (its box with transforms applied, the computed value of each property animated on it,
  whether it shows), then finished, and the player goes on as it does when they end; each settled stage is
  screenshotted with the same nodes probed again, then the player's still render of the same position (`renderStatic`).
  An endless repeat counts one round toward its batch's length and loops at the pauses inside it; the batch's end is
  the click that cuts it, so it holds a still frame where its round ends, as the still render does. Beside
  `motion/probes.json`, the probe tier plays `build-subset.key` (a Keynote probe of Keynote's own motion paths) from
  `$SLIDOCK_PRIVATE/probes-incoming/keynote/` when it is on disk, pinned by sha256; without it the lane says it left
  the deck out.
  Checks:
  - **end state**: a stage's end frame and its settled frame equal the still render. The settled one byte for byte, or
    within the player's own antialias bound. The end one is held to its nodes first: each animated property of each
    node at its end must equal its value once settled (identities such as an identity matrix or a perspective alone
    read as `none`; a split word or character must end at rest), and a node hidden once settled must be hidden at its
    end. Only when every node compared equal may the frame differ by more than the antialias bound, classed `match` by
    the static lane's metrics (a glyph re-rasterized in a composited layer);
  - **start state**: where an entrance of the stage ends, its first frame must not already show it built (it may show
    its own first keyframe, a motion that starts in view), and the frame as a whole must equal the state before the
    stage within the antialias bound or the static lane's `match`;
  - **moves**: every step and transition that animates, paused at a quarter, the middle and three quarters of its own
    span, has some animated property at neither its start nor its end value, and, where its frames change at all in its
    nodes' boxes, a middle frame that differs from both its first and its last (a jump cut, or keyframes that change
    nothing, fails; a cut through black is a switch). Two frames differ by more than a twentieth of the span's largest
    change, or by more pixels than the antialias bound beyond its channel bound: the letters of a by-character exit
    count next to the jump of its shape hiding at its end. A set (a tier-2 `{"property": "set"}` track: PPTX `p:set`,
    ODF `anim:set` of bold, italic, underline, a colour or the opacity) is a switch at its begin
    instead, as an appear is. Its property's pose key is the one the player animates (`opacity`, `--sd-anim-color`,
    `--sd-anim-fill`, `--sd-anim-line`, `--sd-anim-bold`, `--sd-anim-italic`, `--sd-anim-underline`); each unit starts
    its switches where its first animation of that key begins. The walk also pauses 1 ms before and 1 ms after each
    switch (at most 12 switch times per step, spread from the first to the last), and at every pause, from a switch's
    begin to the next switch of its key, the key shows the switch's `to` (colours by value), the last one held to the
    end of the batch or until another animation of that key begins; before its first switch the key shows what it
    showed before the stage, unless another step animates it first. A set the lane cannot read fails. A step whose
    tracks are all sets is not held to `moves`; in a step that also moves, its sets' keys do not count as moving, and
    a set of a property another track of the step animates moves with that track. A track of equal frames is no set,
    and fails `moves`. A tier-2 step is held to its tracks' own begin and length (`beginMs`, `durationMs`; a set's
    animation may also last 0 ms), as the player times them;
  - **direction**: an effect whose `dir` is the direction its content moves (builds `flyIn`, `floatIn`, `peek`,
    `wipe`, `fadeAndMove`, `driftAndScale`; transitions `push`, `cover`, `uncover`, `wipe`) shows at its middle on the
    side it comes from (an entrance, by its boxes against its end, or for a clip by where its frame has changed) or the
    side it goes to (an exit);
  - **reveal**: what shows (each step's units' computed visibility and opacity) is what the timeline gives, and nothing
    changes but by a step of the stage;
  - **timeline**: the animations attributed to each step play in its click group, in order, at the start and for the
    length the deck data gives, on allowed properties only, natively unless the player reports a fallback (RT0006); a
    transition plays for its length, starts on the outgoing slide and ends on the incoming one as it opens, before the
    builds that start by themselves (the incoming layer's nodes end as they show then: Keynote's lead page); the deck's
    timeline matches the IR's in structure where the IR carries one;
  - **placement**: a Magic Move or Morph shares match keys with the slide it leaves; one keyed only with the next slide
    is the move out of its slide, played into it.

  Rows: per slide with a timeline or a transition, `#s<k>` per position and `#t@250|500|750`; each row records the
  checks it failed with their details (`failures`). Each transition and step gets a verdict: native, a declared
  fallback, an accepted fallback, an accepted defect (it played as expected but failed a check a review tracks),
  dropped, or mismatched. A probe deck is held to `probes/out/motion-probe.expect.json`: the effect, and each param the
  expectation names (in one spelling: `horizontal` is `horz`, hex colours in lower case, `[x, y]` is `{x, y}`, a path
  `d` as its commands and numbers). `motion/fallback-allow.json` accepts a fallback by deck, slide, item, native id and
  what played, and a failed check by deck, slide (or `*` for every slide of the deck), item, native id, check and the
  failure's exact detail, as the lane prints it (`problem` lines). The lane fails on a mismatched item, an unaccepted
  `motion-mismatch` row, a slide it could not play or a pair it could not compile, or a stale entry of
  `motion/fallback-allow.json`. Keynote's `all stages` exports are read from
  `$SLIDOCK_PRIVATE/oracles/keynote-stages/<sourceSha>/` (`SLIDOCK_STAGES_ORACLES`), each pinned by its page rasters
  in the manifest's `stages.pageHashes` under the raster key (`oracle --stages --record`, as the static oracles are:
  Keynote writes its creation date into the PDF, so a re-export never has the same bytes), or only by `stages.sha256`
  where no rasters are pinned; a Keynote deck with no export is reported pending. The lane prints how each export is
  pinned (`pinned by its <n> page rasters`) and says what `oracle --stages --record` would still pin. **Stage
  oracles:**
  Keynote prints, per shown slide (a skipped slide prints nothing), a page before the first click and one per click,
  plus a lead page first when the slide's timeline opens with builds that start by themselves (the slide as it
  appears, before them); a build with or after another prints on that build's page. So position `#s<k>` of a slide
  is page `first + lead + k`, and the pages must add up to the export's page count (`oracle-page-count` otherwise).
  `oracle --stages --record` records the pages of each shown slide (`stages.pagesPerRow`, from the deck's IR with
  `--native-motion`, never when they do not add up to the export; rows that differ from a record take
  `--accept-oracle`), and the lane holds the deck's rows to the record: a slide that prints another count is
  `stage-count`, and the deck is not scored (a click moved between slides keeps the total, not the rows). The lane's
  `stages.json` lists the rows it used (`pagesPerRow`).
  The pages are rasterised and read by poppler in the renderer container (started for the purpose and stopped after
  with `--renderer native`). Each position's settled frame is scored against its page with the static lane's
  metrics (SSIM, ΔE00, the text recall of the page's words against the words the settled stage draws, measured by
  the walk, a word split across text nodes joined as `measure.js` joins it) and classed by `classifier@2`. A deck
  with an export compiles with the static lane's pinned font set (`<cache>/fonts`, when it is the one
  `fonts.lock.json` pins; otherwise its stages are `pending` until `fonts sync`) as its last font source, so its frames
  draw the faces the static lane renders it in, not the deck path's substitutes for Keynote's Apple and corpus faces;
  the shell lane bundles it the same way. Such a deck's `corpus` digest in the stamp covers that set, so a run on a
  synced cache and one on an unsynced cache have different run keys, and the shell lane refuses to check a motion run
  whose decks drew other fonts than its own. The lane prints the class of every position and, apart, of each slide's
  built position (its last, the state the static lane renders); classes are reported, not checked. The motion is held
  by two checks on a slide with builds, each comparing where Keynote's pages changed with where the frames changed on a 48-cell grid (most changed cells of each side
  within two cells of the other's): **reveal** (against the position before; the lead page and the opening frame for
  position 0) and **remaining** (against the slide's last position, for every position before it, so position 0
  holds the start state even without a lead page). Every position of a slide with builds has at least one; a
  position that fails one is a stage `mismatch`, which fails the lane, as do an export that is not the pinned one,
  pages that do not add up, and rows that are not the recorded ones. A position where neither check could run (a neighbour's frame or page missing) is counted
  as `unchecked`, apart from the matches. `runs/<key>/stages.json` holds every position's
  scores, checks and settled frame (`frame`, under the run's `frames/`). The bench UI's Effects tab reads the selected motion run, else the newest; a cell of its coverage
  matrix opens the Explorer on that effect (`fx`) and frontend (`fe`).

- **Shell** (`run --lane shell`) bundles each deck of a motion run (`--from`, default its newest) as the zip a user
  downloads and unzips it under the cache's work folder; the zip's `index.html` must be the one the motion run played,
  and the markup must name only files the zip holds and no absolute or external URL. Its `index.html` is then opened
  from `file://` with no router twice. First as a user opens it, with a script that only watches: the deck's own boot
  script must mount its player under the deck's Content Security Policy, the page must report no policy violation and
  no error, and once at rest its first position must show exactly the motion run's settled frame there (unless that
  slide shows a video or a GIF, whose frame depends on time). Then walked by the motion lane's page script, with media
  still: the page must request nothing beyond `file:`, `data:` and `blob:`, and every position (shown slide, step)
  must show exactly the motion run's settled frame.

## The bench UI

`slidock-bench ui [--port 5290] [--dist apps/bench-web/dist]` serves the built app (`apps/bench-web`) and a read-only
JSON and PNG API over the private bench data. It binds 127.0.0.1 only, answers a request only when its `Host` is
`127.0.0.1:<port>` or `localhost:<port>` (else 421), and allows GET and HEAD only. Holdout ids get the same 404 as
unknown ids, before any file is read.

| Route                         | Answer                                                                                    |
| ----------------------------- | ----------------------------------------------------------------------------------------- |
| `/api/status`                 | `Status`: HEAD's ids, the lanes with a baseline, the latest run, holdout and parity flags |
| `/api/runs`                   | `RunsIndex`: every run folder of the cache, and the baselines                             |
| `/api/runs/<run>/results`     | `Results` without the holdout's rows, pairs and aggregate                                 |
| `/api/runs/<run>/rows/<row>`  | `RowDetail`: the record, known losses, diagnostics, images, explain findings, commands    |
| `/api/runs/<run>/parity`      | `ParityView`: the run's parity report with the allowlist                                  |
| `/api/png/<run>/<row>/<kind>` | a PNG: `oracle`, `render`, `heatmap`, `golden`, `golden-diff`, or a motion row's frames   |

`<run>` is a run key, `latest` or `baseline`. Any other `/api/` path is a 404; every other path is a file of the
app's `dist`. The wire types are `slidock-bench-schema`'s structs, generated with `ts-rs` into
`apps/bench-web/src/schema/`. `slidock-bench report --single-file` writes the same app as one HTML file with the data
inlined, for sharing without the server.

## Publishing the public report

The web app publishes a snapshot of the bench beside itself, at <https://macabeus.github.io/slidock/bench/>: the page
and its images live in `apps/web/public/bench/`, which the app's build copies as they are. A maintainer regenerates it
from the private data, at a commit whose bench UI is built, and commits the folder:

```sh
pnpm --filter @slidock/bench-web build
slidock-bench report --public            # writes apps/web/public/bench/: index.html and images/
```

What it holds:

- The presentations `public.json` lists, and nothing of any other: the list is default-deny, so a deck added to the
  corpus stays out until it is listed. Their rows only; the stamp's pairs and every summary are recomputed over those
  rows, so no count, median or share covers an unlisted deck.
- The `latest` static run (`--runs` names others; only static runs are published). No parity report, no status, no
  other lane: the page leaves out the views it holds no data for.
- No corpus file name or path: a row's source is named `<presentation>.<extension>`, and its images missing from the
  report say so instead of naming a command.
- The oracle, render and heatmap of the `mismatch` rows (`--images` names other classes), at the metric size, as
  lossy WebP (libwebp's `cwebp -q 85 -m 6`, which must be on `PATH`), under `images/<run>/<row>/<kind>.webp`. The
  scores are the run's, computed on the lossless renders; the WebP images are for reading.

Before writing anything, the command checks the payload: no presentation outside the list named anywhere (as a row
id, a pair key, a `presentation` field, or its id as a word), no corpus file name, stem or title of the manifest, no
path (`/Users/`, `~/`, `.slidock`, the cache and corpus folders, …). A finding stops it with nothing written. The
page's Content Security Policy allows its own inline script by hash and images from its folder and `data:` only. The
`public` tests of `slidock-bench` run the same check over the committed report in CI, without the private data, with
the corpus ids the repository names (`bench/budgets.json`, `tests/golden/manifest.json`), and hold its images to the
files the payload names.

## Known limits

- The middle frames of moving elements (paths, fly-ins) can rasterize a pixel differently from one run to the next;
  no verdict depends on it.
- Direction is checked for the effects whose `dir` names where the content moves; strips, checker and 3D transitions
  are held to moving at all, not to a side.
- LibreOffice's PPTX import and export disagree on the horizontal axis of `push`, `cover` and `pull`, so that axis
  stays inferred.
- The fonts lane's equivalence with the static render (identical, or a class that does not drop) is not computed;
  `fonts.equivalence` stays empty.
- Google's answers change when a family's version moves: rerun `fonts lane-lock` and review the diff to follow them.
