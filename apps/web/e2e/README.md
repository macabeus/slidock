# End-to-end tests

Playwright (`@playwright/test` 1.60.0) in Chromium, Firefox and WebKit. The specs:

- `app.spec.ts`: on the production build (`vite preview`), each deck uploads, compiles with progress, shows every
  slide (stepped with the keys: slide count, mounted markup, the player's counter and the toolbar numbering shown
  slides only, media loaded, a non-blank render), presents, and downloads a zip that is byte-identical to the CLI's
  zip of the same input, both with Google Fonts off (the app's switch remembered off, the CLI with `--no-google`; no
  request reaches Google), since Google serves the CLI static files and a browser per-script chunks. A deck whose HEIC
  photos the browser decoded (the CLI omits them) is held instead to the zip the compiler writes under Node through
  the compile worker with the same decoded files (`support/heic.ts`). On the smoke deck the page options (title,
  description, public URL, notes on the page), then each presenter view and speaker notes combination of the dialog,
  give zips byte-identical to the CLI's with the same flags.
- `file.spec.ts`: the CLI's zip, unpacked and opened from `file://`, presents every shown slide (each click group
  played, then a non-blank render), its images decode and its videos load, with no console error and no CSP violation.
- `notes.spec.ts`: the notes panel keeps one height across slides whose notes differ (the smoke deck), so the stage
  never resizes when the slide changes.
- `presenter.spec.ts`: the presenter window opens, follows the deck window and drives it (next, previous, go to, blank
  black and white, both ways), in the app and from `file://`; in the app it opens from the button and from the
  player's P key (presenting, and viewing with the stage focused). Reconnection after a reload is recorded (table
  below). The app's presenter window and a downloaded deck's draw the same frame and view: the same elements, roles,
  labels, boxes and computed styles, in the dark and the light scheme, the deck's title aside.
- `drawn.spec.ts`: the non-blank check itself fails on a slide whose elements render nothing, and on a background-only
  slide whose background does not paint.
- `present.spec.ts`: Escape while the browser has yet to answer the Present click's full screen request (dispatched in
  the click itself, which every engine still has pending, and pressed right after it, which Chromium often has) and
  Escape once the show fills the screen leave the app as before: not full screen, nothing filling the window a second
  later, the viewer on the same slide, the deck still moving.
- `progress.spec.ts`: the suite's compile-progress reader (`support/progress.ts`) on a page that stands in for the
  app: each poll reads the conversion panel in one call, so the commit that swaps the panel for the deck's toolbar
  between two calls never stalls it.
- `fonts.spec.ts`: the fonts panel on the demo page (dev server, fake worker telling a font story): every family and
  source in both themes (screenshots under `<cargo target>/e2e/screens/`), Google Fonts answered from the recorded
  `css2` response with no request carrying `text=`, the switch remembered, a dropped font changing the render at the
  current slide, picked and refused files, the substitutes from `public/fonts/core/`, and each Local Font Access state
  (the browser's own, then a stand-in: prompt, granted, denied, denied at the prompt, error). Then the smoke deck in
  the production app with the real worker: the panel, and a picked font file taken.
- `font-matrix.spec.ts`: the font packaging matrix, compiled with shipped fonts and nothing downloaded, opened from
  `file://` and from HTTP: the font probe ODP (embedded faces), the font probe PPTX (the same faces as dropped files),
  a synthetic deck written in code (`fixture/fontsPptx.ts` `MATRIX_DECK`: an MTX EOT and a plain EOT embedded;
  TrueType, variable, CFF and collection files dropped), a copy of it with one paragraph shown on a click (the step
  written into the compiled deck's timeline), and a stack deck (`STACK_DECK`) whose runs request families that bind to
  a differently named shipped family. On every shown slide, with every click build played (text a build shows must
  appear), each text-bearing element names a shipped family first, every `@font-face` loads, and the render of its
  visible text equals the render through aliases that can only resolve to the shipped files (then a generic other than
  the page's own fallback, so a glyph the faces lack shows) and differs from the render with only the stack's
  generics; the visible text is where pixels change when the element's own text turns transparent (and stay put when
  nothing changes), with the slide's GIFs hidden and its videos paused so that two renders compare (the text under
  them is then compared too), so text a picture or an opaque box in front of it covers entirely is recorded, not
  compared, and text under a translucent picture is compared. Chromium also reports (CDP
  `CSS.getPlatformFontsForNode`) only web fonts that are files of a shipped family of the element's stack; the one
  exception, recorded, is a platform colour emoji font drawing emoji. No console error, no CSP violation; the
  presenter window shows the same faces at every build step, and its next-slide preview passes the same checks. The
  checks themselves fail on copies of the synthetic deck or the probe ODP whose injection is broken (CSP blocking
  fonts, a face covering none of its text, a renamed family, a font of this computer drawing instead, a serif face
  missing one glyph), skip only the covered part of a title a picture covers, and compare a title under a translucent
  picture, under an animated GIF, or over a box behind it. `SLIDOCK_E2E_FONTS_CORPUS=1` adds the corpus deck
  `pptx-talk`, which embeds fonts, compiled as a user would (embedded, Google Fonts, bundled substitutes; downloads
  cached under `<cargo target>/e2e/font-matrix/corpus-cache/`).
- `heic.spec.ts`: the HEIC probe deck (`probes/out/heic-probe.pptx`: a grid photo tagged Display P3, the grid turned
  by its `irot`, a single-picture photo, that photo mirrored by its `imir`, and a photo cut by its `clap`): where the
  browser decodes HEIC (WebKit, with its image decoder; Chrome, `SLIDOCK_E2E_CHROME=1`, with WebCodecs) the Converting
  view shows the decoding step, each photo shows at its plan's size with its tiles in place, turned, mirrored and
  cropped (colours sampled), MID5608 replaces MID5601, and the zip equals the compiler's under Node with the same
  decoded files; where it cannot (Playwright's Chromium) each photo keeps its marker and its diagnostic says to open
  the deck in Chrome or Safari or to export it to PPTX.
- `videos.spec.ts`: the video probe deck (`probes/out/video-probe.pptx`), downloaded with "Compress videos" off and on
  (High): the zip is smaller; each planned movie is an H.264 MP4 of its planned size, movie boxes first, with an AAC
  sound track where the original has sound (AAC carried over, PCM re-encoded) and none otherwise; the movie already
  small and the HEVC-with-alpha one stay as they are; both decks, unpacked and opened from `file://`, play each video,
  its frames alike (SSIM) at three times; the choice is remembered.
- `copies.spec.ts`: the video probe deck's HEVC-with-alpha clip, downloaded from the app: WebKit (which plays it with
  its alpha) makes its WebM copy, the zip holds it right after the original and the slide's `<video>` offers both, and
  the deck opened from `file://` plays the clip with its transparency in WebKit (the original) and Chromium (the
  copy): two backgrounds show through its half at alpha 96 and not through its opaque half, nor through the opaque
  H.264 clip beside it (the check's negative control). Chromium, which plays no HEVC, makes no copy, says the clip
  plays only in Safari and Chrome on a Mac and how to fix it, and its deck does not play the clip in Chromium.

```sh
pnpm --filter @slidock/web e2e          # the smoke deck and every corpus deck
pnpm --filter @slidock/web e2e:smoke    # the smoke deck only: no private data needed
pnpm --filter @slidock/web e2e font-matrix.spec   # the font packaging matrix only
SLIDOCK_E2E_FONTS_CORPUS=1 pnpm --filter @slidock/web e2e font-matrix.spec   # and a corpus deck with its fonts
SLIDOCK_E2E_FIREFOX=docker pnpm --filter @slidock/web e2e   # Firefox's browser in the pinned Playwright image
SLIDOCK_E2E_WEBKIT=docker pnpm --filter @slidock/web e2e --project=webkit-docker   # WebKit's, in the same image
pnpm --filter @slidock/web e2e design.spec   # the design rules on every screen of the demo page
SLIDOCK_E2E_CHROME=1 pnpm --filter @slidock/web e2e heic.spec --project=chrome   # HEIC photos through Chrome's HEVC decoder
pnpm --filter @slidock/web e2e:pages   # the GitHub Pages dry run (below)
pnpm --filter @slidock/web exec playwright test --config e2e/review/review.config.ts   # review screenshots
```

- **Decks.** `fixture/pptx.ts` writes the smoke deck (four slides, one hidden, a PNG drawn in code, notes, a fade)
  from the OOXML pieces `@slidock/wasm/test/ooxml` exports, which the wasm package's tests share; it holds our own
  content only. The corpus is `SLIDOCK_CORPUS_DIR` (default `$SLIDOCK_PRIVATE/corpus`, `~/.slidock/corpus`), and a deck
  named by its id is found through the corpus manifest (`SLIDOCK_CORPUS_MANIFEST`, as the bench reads it); the full
  run fails when the corpus is missing. The corpus is the maintainer's: contributors run `e2e:smoke`.
- **Setup.** The web server step builds the player, the wasm module and the app, then serves `dist/` with `vite preview`
  on port 4317. The global setup builds the CLI (`cargo build --release -p slidock-cli`), writes the smoke deck and
  compiles every deck to `<cargo target>/e2e/decks/<deck>/deck.zip`, unpacked next to it. `SLIDOCK_E2E_SKIP_BUILD=1`
  skips the builds and serves the last one. The global setup first checks that the server on the app's port serves a
  build for `/`, and stops the run otherwise (a build the Pages dry run left in `dist/` is one for `/slidock/`).
  Results, traces and downloads stay under `<cargo target>/e2e/`.
- **Browsers in Docker.** Playwright 1.60's Firefox (150.0.2, build 1522) does not start on macOS 27: its main thread
  spins at startup and Juggler never connects, headless or headed. Its macOS WebKit (build 2287) sometimes stops
  launching too (no WebContent process ever spawns). `SLIDOCK_E2E_FIREFOX=docker` runs the `firefox` project's
  browser, and `SLIDOCK_E2E_WEBKIT=docker` adds a `webkit-docker` project beside the macOS `webkit` one, in
  `mcr.microsoft.com/playwright:v1.60.0-noble` (the same builds, on Linux) through one `playwright run-server`, bound
  to `127.0.0.1:3555` (`packages/player/e2e/support/browserServer.ts`, shared with the player's suite): the tests stay
  on this machine, the browser reaches the app on this machine's loopback through the connection (`exposeNetwork`),
  and opens the `file://` decks from the worktree and the suite's output, mounted read-only at the same paths. The
  global setup starts and stops the container. The container's one font of its own beside the image's is DejaVu Sans
  (`public/fonts/core/DejaVuSans.woff2`, mounted read-only and indexed by `fc-cache` before the server starts): the
  font matrix's "a font of this computer" check names it in `local()`, so the check catches a platform font drawing
  in place of a shipped file on Linux as it does on macOS.
- **Ports.** `SLIDOCK_E2E_PORT` (the production build, default 4317), `SLIDOCK_E2E_DEMO_PORT` (the demo page, 4318) and
  `SLIDOCK_E2E_BROWSER_PORT` (the browser server in Docker, 3555) let runs in parallel worktrees keep their own
  servers. A server already listening on a port stops the run, since one left over from another run may serve another
  build; `SLIDOCK_E2E_REUSE=1` reuses it. The browser server's container is the run's own: it is named
  `slidock-e2e-browsers-<port>-<pid>-<random>` and labelled `dev.slidock.e2e.browser-server=<pid>-<random>`, and the
  teardown removes only the containers with that label. A container the run did not start, live or left over, is
  never removed: when it holds the browser port, the run stops and names it (give the run another port, or remove the
  container with `docker rm -f <name>` once no run owns it).
- **No deck is skipped.** A 1 GB Keynote deck runs everywhere too: its app test takes about 2 min in Chromium, 3 min
  in WebKit and 6 min in Firefox (the file crosses the browser-server connection), and the downloaded zip (a ~1 GB
  `Blob` in the page) is compared with the CLI's.
- **Non-blank render.** `expectDrawn` screenshots the shown slide's layer with the player's chrome hidden, then again
  with every element but the slide's background transparent, and requires the two to differ. A slide that is only a
  background must look the same over two stage colours (its background covers the stage).
- **Videos** a browser cannot decode (QuickTime `.mov` outside WebKit, `canPlayType` says no) are recorded as
  `unsupported media` annotations instead of failing.
- **Copies of transparent videos** are made only where the browser plays the original with its alpha: WebKit here
  (and Google Chrome on macOS); Playwright's Chromium and Firefox have no HEVC decoder.

## Design rules

`design.spec.ts` audits every screen and state of `support/screens.ts` on the demo page (the app over a fake worker
scripted per screen, with the real player) in both themes at 375 px and 1280 px, with `packages/ui/src/audit.ts`:
text contrast at least WCAG AA (4.5:1, 3:1 for large text; a form field's value, placeholder and selected option
too), no horizontal page scroll and every text, control and image inside the 16 px side gutter (12 px in present mode
and the presenter window, the player's own chrome), a visible focus indicator of at least 3:1 against what it is drawn
over on every control the Tab key reaches, and an accessible name on every control. The audit's own tests come first:
it reports a page that breaks each rule, a control that appears during the Tab walk, a walk cut short and a page that
navigates under it, and nothing on a page that keeps the rules. The dev server's HMR socket is held open by a stub in
every page (`holdDevServerSocket`): Vite's client reloads a page whose socket drops, which a test shows.
`review/review.config.ts` shoots the same screens for a design review into `target-review/design/web/`.

## Pages dry run

`e2e:pages` (`pages/pages.config.ts`) builds the app exactly as `.github/workflows/pages.yml` does (`pnpm wasm:build`,
then `pnpm --filter @slidock/web build` with `VITE_BASE_URL=/slidock/`), checks that `dist/` is a build for
`/slidock/`, serves it under `/slidock/` on a free loopback port (every other path answers 404), and runs the real flow
in Chromium on the motion probe decks (`probes/out/motion-probe.pptx` and `.odp`), and on the deck
`SLIDOCK_PAGES_DECK` names (a path) when it is set: open, view every slide, present, open the presenter window, download the zip and play it from
`file://`. Every request stays under `/slidock/` or is the deck's own blob or data URL or the unpacked zip's folder,
and no page logs a console error, throws or violates its CSP. `SLIDOCK_PAGES_SKIP_BUILD=1` serves the last build.

## Presenter reconnection

The presenter window reloaded while the deck window runs, and a presenter page opened by hand with no opener, so only
the `BroadcastChannel` (keyed by the deck id) can reach the deck window. Each run records them as the annotations
`presenter reload` and `broadcastchannel only`.

| Browser                     | Where     | Presenter window reloaded (opener kept) | `BroadcastChannel` only (no opener)                    |
| --------------------------- | --------- | --------------------------------------- | ------------------------------------------------------ |
| Chromium 148                | `file://` | reconnects and drives                   | follows and drives                                     |
| Chromium 148                | app       | reconnects and drives                   | n/a: the app's presenter gets the deck from its opener |
| Firefox 150 (Linux, Docker) | `file://` | reconnects and drives                   | follows and drives                                     |
| Firefox 150 (Linux, Docker) | app       | reconnects and drives                   | n/a                                                    |
| WebKit 26.4                 | `file://` | reconnects and drives                   | does not connect                                       |
| WebKit 26.4                 | app       | reconnects and drives                   | n/a                                                    |

A reloaded presenter window keeps `window.opener`, so `postMessage` finds the deck window again in every browser. The
`BroadcastChannel` alone connects the two `file://` pages (the same `index.html`) in Chromium and Firefox; WebKit
delivers no message between them. In the app the channel is same-origin, but a presenter page without an opener has no
deck to show (`Nothing to present here`).

Recorded on macOS 27.0.1 with Playwright 1.60.0, 2026-10-30.
