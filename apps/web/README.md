# @slidock/web

The web app: drop or pick a `.pptx`, `.key` or `.odp` (or a Keynote package folder), compile it in a
worker, view it, present it and download the standalone zip. Nothing leaves the browser.

```sh
pnpm wasm:build                          # from the repo root, once
pnpm --filter @slidock/web dev           # http://localhost:5173/
VITE_BASE_URL=/slidock/ pnpm --filter @slidock/web build
```

`/demo.html` on the dev server runs the app over a fake compile worker that "compiles" a synthetic sample deck, with
@slidock/player: `?theme=light|dark`, `?view=landing`, `?panel=fonts|download|help`. The build leaves it out.

## Layout

| Path                           | Holds                                                                                                                                                  |
| ------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `src/compile/`                 | the worker script (`@slidock/wasm/worker`), `CompileClient` (the protocol of `packages/wasm/README.md`), `DeckSession` (the store)                     |
| `src/compile/input.ts`         | accepted file types, drops (folders through `webkitGetAsEntry`), Keynote packages packed as a stored zip (`@slidock/wasm/folder`)                      |
| `src/player/types.ts`          | the player API the app codes against                                                                                                                   |
| `src/player/slidock-player.ts` | that API over `@slidock/player`                                                                                                                        |
| `src/presenter/`               | the `#presenter` window the player opens: it gets each deck its opener serves, and each close, by `postMessage`; the two players sync their positions  |
| `src/views/`                   | landing, conversion, viewer (toolbar, rail, stage, notes and diagnostics), fonts, download, shortcuts                                                  |
| `src/fonts/`                   | the fonts flow (`FontsController`): Local Font Access, Google Fonts, dropped files, bundled substitutes, the re-finish (see [Fonts](#fonts))           |
| `src/media/`                   | HEIC photos: the decode worker and its decoder (`heic.ts`), the client, the open → decode → `addMedia` → finish step (see [HEIC photos](#heic-photos)) |
| `public/fonts/core/`           | the bundled substitutes as WOFF2, their licences and `core.json`, written by `scripts/substitutes.mjs`                                                 |
| `public/bench/`                | the fidelity bench's public report the landing links to, written by `slidock-bench report --public` (`bench/README.md`)                                |
| `src/testing/`                 | the fake worker, fake player and sample deck the tests and the demo page use                                                                           |
| `csp.ts`                       | the Content Security Policy the build writes into `index.html`                                                                                         |
| `e2e/`                         | the Playwright suite (`e2e/README.md`)                                                                                                                 |

The app talks to the worker and the player only through `AppServices` (`src/services.ts`); `main.tsx` passes the real
ones, the tests and the demo page pass fakes.

URL state lives in the fragment: `#slide=<n>` (1-based, hidden slides counted), `#step=<k>` when a slide is not fully
built, `#present=1` while presenting, and `#presenter` (with or without a value) for the presenter window. The position
belongs to one deck: the first deck opened in a page load starts there (reload, then open the same deck again), and
every later one starts at its beginning.

## Fonts

The fonts panel lists each family the slides draw, where each face
comes from (embedded, this computer, Google Fonts, your file, substitute), the mislabeled embedded faces and the faces
whose licence keeps them out of the download. `src/fonts/controller.ts` starts the sources once the deck shows: the
substitutes at once, and beside them installed fonts (when already granted), then Google Fonts. The worker ranks faces
by source, so the order files arrive in does not change the deck. Each change adds files to the worker (`addFont`),
one at a time and each read only when its turn comes, and ends with one re-finish, which swaps the slides in place at
the current slide (while presenting, the swap waits until the presentation ends; the media URLs stay the same).

Once the sources that need no click have settled, `src/views/FontsPrompt.tsx` asks for what is still missing: a banner
above the stage names the families a substitute draws (the panel's own count) and offers this computer's fonts (the
click Local Font Access needs, only where a click can still read them), font files, the panel, or keeping the
substitutes for this deck. It stays while what it started runs and goes once nothing is missing.

- **This computer** (`localFonts.ts`): Local Font Access, Chromium only. It is read on open only when the permission is
  already `granted`, otherwise from the button's click. The worker picks the faces worth reading from their names
  (`selectLocal`); only those files are read, at most 48 files and 96 MB per deck (48 MB per file unless the PostScript
  name is one the deck asks for), each once. Each state (unsupported, prompt, granted, denied, a click needed, an
  error) shows its own text. A worker that answers an empty `selectLocal` with no `localPicks` cannot match installed
  fonts: the section then says so, with no button, and the browser is never asked.
- **Google Fonts** (`google.ts`, `css2.ts`): on by default, switchable off (remembered in `localStorage`). Only the
  `css2` URLs the worker plans from its catalog of open families are fetched, never with `text=`: the per-script
  WOFF2 chunks holding a character the slides draw are fetched whole, and the worker subsets them. The deck's text
  never leaves the browser. A request (its body included) that takes over 15 s is aborted and reported. Turning it off
  opens the deck again and adds back every other file.
- **Your files** (`dropped.ts`): `.ttf .otf .ttc .otc .woff .woff2` dropped anywhere on an open deck's page, picked, or
  picked as a folder: at most 64 files and 256 MB per deck, 64 MB per file. The app keeps the `File`, not a copy of
  its bytes, to add it again when the deck reopens.
- **Substitutes** (`bundled.ts`): the faces each deck draws with no file yet (the deck message's `substitutes`),
  fetched from `public/fonts/core/` and checked against the SHA-256 in `core.json`, then added as `bundled`.
- **Download**: families from this computer or your files ship with a notice and a per-family toggle
  (`BundleSettings.leaveOut`), in the panel and in the dialog. A family whose face's licence restricts embedding it
  (`fsType`) draws in the slides here and is left out of the download unless the same toggle includes it
  (`BundleSettings.include`). Changing either forgets a zip already built.
- **Preview**: the faces the deck ships are added to the document (`documentFonts.ts`), which reaches every slide's
  shadow root and the presenter window.

The substitutes are committed (24 faces, 3.4 MiB): Liberation 2.1.5 (OFL-1.1), DejaVu Sans 2.37 (Bitstream Vera),
Carlito (OFL-1.1, googlefonts/carlito@3a810cab) and Caladea (OFL-1.1, google/fonts@6c1dbe27), each download and each
TTF pinned by SHA-256. To rebuild them:

```sh
SLIDOCK_FONTTOOLS_PYTHON=<python with fonttools==4.60.1 and brotli==1.1.0> node apps/web/scripts/substitutes.mjs
node apps/web/scripts/substitutes.mjs --check   # the committed files match core.json
```

## HEIC photos

Keynote keeps an iPhone's photos as HEIC, which no browser but Safari draws and the compiler does not decode. After
`open`, the worker lists them (`mediaNeeds`), each with the compiler's decode plan (`slidock_io::read_heif`: the HEVC
tiles of the image, their place, the crop, quarter turns and mirror, the colour), or why it plans none. The session
(`src/compile/session.ts`) has each planned photo decoded in a worker (`src/media/decode-worker.ts`), one at a time,
before `finish`, and the Converting view shows the step ("Decoding 2 HEIC photos"). No permission is asked. A cancel
or another file stops the worker, and a photo that takes longer than 60 s is given up. The decoder
(`src/media/heic.ts`) tries, in order:

1. **The browser's image decoder**: `createImageBitmap` of the file, drawn on an sRGB canvas. Safari has it (ImageIO);
   it applies the file's crop, quarter turns and mirror itself, so only a bitmap of the plan's output size is taken.
2. **WebCodecs**: a `VideoDecoder` per HEVC configuration, configured with its codec string and `hvcC` record, one key
   chunk per tile, each frame drawn at its place on a canvas of the coded size, then the plan's transforms in their
   order, each full-size canvas released once the next holds it. The frames are labelled with the photo's colour space
   (its `nclx`, or the space its ICC profile matches: an iPhone's Display P3, and what neither names as the coded
   pictures' own VUI states it), so the browser converts them to sRGB as it draws them. Chrome has it (an HEVC decoder of
   the platform); Playwright's Chromium does not.

Each photo is encoded as a JPEG (quality 0.92) of the plan's output size and handed to the compiler (`addMedia`),
which checks its type and size and shows it (MID5608). A photo neither path decodes is drawn from the preview Keynote
keeps beside it where there is one (MID5609), and stays omitted otherwise (MID5601); either diagnostic, in the
diagnostics panel, says to open the deck in Chrome or Safari, or to export it to PPTX from Keynote (which converts its
photos to JPEG). The decoded files stay with the open deck and are added again when the fonts flow reopens it.

The deck's file for a decoded photo is the browser's: the zip is the same for the same input and the same decoded
files (the e2e suite holds it to the zip the compiler writes under Node for those files), but another browser, or the
CLI, which decodes none, writes another one.

Measured on the corpus's two iPhone photos (`retro-fidelity.key`, a 3022 × 3422 and a 4032 × 3024 grid of 512 × 512
tiles), against Keynote's JPEG export of the same photos: WebKit's image decoder is at ΔE2000 0.9 on average, Chrome's
WebCodecs at 1.0 (a JPEG at 0.92 of the photo itself is at 1.0); an sRGB canvas is closer than a Display P3 one (0.9
against 1.0 and 1.0 against 1.1). The 4032 × 3024 photo decodes in about 0.15 s in Chrome and 0.22 s in WebKit.
