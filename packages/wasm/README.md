# @slidock/wasm

The slidock compiler built to WebAssembly (`pkg/`, from `pnpm wasm:build`) and the compile worker that runs it. The
app never calls the compiler on its main thread: it starts a dedicated worker, which loads the wasm module and answers
the messages below.

| Import                     | What it is                                                                                                                   |
| -------------------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| `@slidock/wasm/worker`     | The worker's entry (`src/worker.ts`): loads the module and serves the protocol on `self`.                                    |
| `@slidock/wasm/folder`     | `packFolder`, `droppedFiles`: a Keynote package folder as one `File` to `open` (see [Folders](#folders)).                    |
| `@slidock/wasm/protocol`   | The protocol's types, re-exported from `@slidock/types`.                                                                     |
| `@slidock/wasm`            | The wasm-bindgen module itself (`Compiler`, `zipEntries`, `version`), for tests and tools.                                   |
| `@slidock/wasm/test/ooxml` | Test only: the OOXML pieces (`test/support/ooxml.ts`) the package's tests and the web app's e2e decks write PPTX files from. |

Start the worker from a module of the app, so the bundler builds it as a worker:

```ts
// compile-worker.ts
import '@slidock/wasm/worker';

// in the app
const worker = new Worker(new URL('./compile-worker.ts', import.meta.url), { type: 'module' });
```

The message types are generated from Rust (`crates/slidock-wasm/src/protocol.rs`) into `@slidock/types`:
`ToWorker` (app → worker) and `FromWorker` (worker → app), with the deck JSON (`Deck`, format `slidock-deck@1`) they
carry.

## Requests and replies

Every request carries an `id` the app picks (unique while the request runs). Every message the worker sends about a
request carries that `id` back, and each request ends with exactly one `done` or one `error`.

| App sends                                      | Worker replies, in order                                                         | What it does                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| ---------------------------------------------- | -------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `open { id, file, fonts? }`                    | `progress` (`open`), any number of `diagnostics`, `needs`, `mediaNeeds`, `done`  | Opens `file` (a `File`), replacing any open deck. The worker reads it by ranges with `FileReaderSync`, never whole. It sniffs, lowers, verifies and runs the passes that need no font. With `fonts` (the default) it adds the presentation's embedded fonts and `finish` ships the faces the deck draws; `fonts: false` binds against the built-in fonts only and ships nothing (the parity path, `slidock compile --emit documents`). `mediaNeeds` lists the HEIC photos the slides show, which the compiler does not decode: each with its source file (a `Blob`: a slice of the input, or the bytes the compiler read) and its decode plan (`plan`: the HEVC tiles with their codec strings, `hvcC` records and the colour their pictures' VUI states (`signal`), places and byte ranges of the file, then the transforms and the colour), or why it plans none (`refusal`). |
| `fontsNeeded { id }`                           | `needs`, `done`                                                                  | The faces the text uses that no font provides yet, each with its Google Fonts request (`google`, open-licence families only), its Local Font Access query (`local`) and its metric-compatible `substitute`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| `selectLocal { id, fonts }`                    | `localPicks`, `done`                                                             | Of the installed faces' metadata (`queryLocalFonts()`), the ones worth reading for the open faces, by index. Read each pick's `blob()` and add it as `local`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| `googleFaces { id, css }`                      | `googleFaces`, `done`                                                            | Of a `css2` response fetched from a need's `google.url`, the faces to fetch: the per-script chunks whose `unicode-range` holds a code point the text draws (fetched whole, so the deck's text never leaves the browser). Add each as `google` with its `unicodeRange`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `addFont { id, bytes, source, unicodeRange? }` | `needs`, `done`                                                                  | Adds a font file (`ArrayBuffer`, transferred) from `local`, `google`, `user`, `embedded` or `bundled`, for the next `finish`. `unicodeRange` marks one chunk of a face: the chunks of a face bind as one and ship together. `needs` lists what is still missing. A file that is no font is an `error` of kind `request`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `addMedia { id, need, bytes }`                 | `done`                                                                           | Adds the app's decode of a media need (`need`: its `id`) for the next `finish`: a PNG or JPEG (`ArrayBuffer`, transferred) of the plan's output size. The deck then shows it as `media/<id>.jpg` (or `.png`), with MID5608; unanswered, the photo stays omitted (MID5601). A file of another type or size is an `error` of kind `request`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| `finish { id }`                                | `progress` (`finish`, then `print`), any number of `diagnostics`, `deck`, `done` | Runs the passes from bind-fonts with the fonts added so far and prints the deck. Send it again after more `addFont`s: the new `deck` replaces the old one, and parsing never repeats.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| `imagePlan { id }`                             | `imagePlan { id, images }`, `done`                                               | Lists the pictures of the last finished deck the app may re-encode smaller for a download (`slidock::images`): still PNG, JPEG, WebP and GIF files, each with its size as it shows (its EXIF orientation applied), its `orientation`, its size in bytes and the size to encode it at (`targetWidth` × `targetHeight`: twice the largest box it is drawn in when the slide fills a 1920 × 1080 screen, never larger than it shows). It forgets the pictures handed over before.                                                                                                                                                                                                                                                                                                                                                                                                  |
| `addImage { id, path, bytes }`                 | `done`                                                                           | Hands over the app's re-encode of a planned picture (`path`): WebP, JPEG or PNG (`ArrayBuffer`, transferred), still, of the planned size, stored as it shows (no EXIF turn) and smaller than the original. A bundle with `compressImages` writes it as `media/<sha256>.<ext>` of its type in the original's place, and the slides and the deck JSON name it. Another file is an `error` of kind `request`, and the picture stays as it is.                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| `videoPlan { id }`                             | `videoPlan { id, videos }`, `done`                                               | Lists the videos of the last finished deck (`slidock::videos`): what each movie holds (codec, size as it shows and its `rotation`, length, frames, sound, colour), its size in bytes and the size to re-encode it at (`targetWidth` × `targetHeight`: the pictures' rule, each side even), or `keep`: why it stays as it is (an alpha layer, HDR, a fragmented movie, …). It forgets the videos handed over before.                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| `addVideo { id, path, file }`                  | `done`                                                                           | Hands over the app's re-encode of a planned video (`path`) as a `Blob`, which the worker reads by range: an MP4 with its movie boxes first, one H.264 track of the planned size, unturned, with the original's frames, the original's sound as AAC (none when it has none), no other track, as long as the original and smaller. Another file is an `error` of kind `request`, and the video stays as it is.                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `alternatePlan { id }`                         | `alternatePlan { id, videos }`, `done`                                           | Lists the videos of the last finished deck that get a WebM copy other browsers play (`slidock::alternates`: those kept for their alpha), each with the size to make it at. It forgets the copies handed over before.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| `addAlternate { id, path, file }`              | `done`                                                                           | Hands over the app's copy of a video (`path`) as a `Blob`, which the worker reads by range: a WebM of one VP9 or VP8 track of the planned size with WebM's alpha on every frame, a key frame first, its cues before its media, the original's frames and length, the original's sound (Opus or Vorbis) or none. Another file is an `error` of kind `request`, and the video has no copy.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `bundle { id, settings }`                      | `progress` (`bundle`), one or more `zipChunk`, `done { report }`                 | Bundles the last finished deck as a zip, streamed in chunks of 1 MiB (the last one shorter). `settings.leaveOut` names families from this computer or the user to leave out of the download; `settings.include` names those whose faces' licence restricts embedding to include all the same (left out by default). `settings.compressImages` writes the pictures `addImage` took since the last `imagePlan` in their originals' place, `settings.compressVideos` the videos `addVideo` took since the last `videoPlan`, and `settings.alternateVideos` writes the copies `addAlternate` took since the last `alternatePlan` beside their originals.                                                                                                                                                                                                                            |
| `memory { id }`                                | `memory { id, bytes }`, `done`                                                   | The compiler's linear memory in bytes. It only grows, so it is the most the compiler has held since the worker started (the bench reads it as the wasm leg's peak memory).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| `cancel { id }`                                | nothing of its own                                                               | Drops request `id` if it has not started: it ends with `error { kind: 'cancelled' }`. A request that has started runs to its end.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |

### One request at a time

The worker runs the requests in the order received, one at a time. Each stage (`open`, `finish`, `print`, `bundle`)
is one synchronous call into the compiler, and a dedicated worker reads no message while a call runs. So:

- **Progress is per stage.** `progress { id, phase }` says that request `id` starts `phase`. Within `bundle`, the
  `zipChunk`s themselves show how far it is: the app knows the media from the deck (`deck.media`), and the worker
  posts each chunk as it is written, while the call still runs.
- **`cancel` reaches only queued requests.** The worker takes every message off its queue as it arrives and starts
  the next request in a later task, so a `cancel` sent before its request starts finds it. Use it to drop a stale
  `finish` after the fonts change again. To stop a request that has started (a long `open` or `bundle`), terminate
  the worker and start a new one; the open deck is lost with it. A sink-observed cancel flag would need a
  `SharedArrayBuffer`, which requires cross-origin isolation (COOP/COEP headers) that GitHub Pages cannot send.

When the worker has loaded the wasm module it sends `ready { version }`; requests sent before that wait for it. If
the module cannot load, it sends `error { id: null, error }`, and every request gets an `internal` error.

### Reading the input

`open` reads the `File` by ranges: each read the compiler asks for is one `FileReaderSync.readAsArrayBuffer` of a
`file.slice`, copied into wasm memory. The file is never read whole, and `bundle` pulls each medium from it by range
as the zip reaches it, so the wasm memory stays far below the input's size. Under Node, the worker's module peaked at
63 MiB for `open` and `finish` of the corpus's 1 GB Keynote deck and at 178 MiB with its `bundle` (a 967-chunk zip,
byte-identical to the native one); the 90 MB deck peaked at 293 MiB.

## The deck

`deck { id, deck, slides, media, fonts, faces, fontIssues, substitutes }` is everything the app needs to show and
present the deck:

- `deck`: the deck JSON, the same `slidock-deck@1` data the zip's `index.html` embeds (slides, numbering, notes,
  build steps, transitions, media, fonts, diagnostics summary), except `player`, which is `null`: a bundle records the
  player it embeds. Each slide's `fonts` lists, as indexes into `deck.fonts`, the faces its template draws, for the
  player's readiness gate.
- `slides`: one `{ id, html }` per slide, in show order: the slide composed with its layout, as the zip's
  `<template data-sd-slide>` holds it. Media are referenced as `media/<sha256>.<ext>`.
- `media`: one `{ path, mime, blob }` per file under `media/`. A copied medium's `blob` is a slice of the input
  `File` (no copy); a transcoded one holds the written bytes. The app makes `blob:` URLs from them, resolves the
  `media/…` references through them at mount, and revokes them when the deck closes.
- `fonts`: the shipped faces' `@font-face` rules (`family`, `weight`, `style`, `unicodeRange`, `format`) with their
  files as transferred `ArrayBuffer`s: the rules the zip's `<style id="sd-fonts">` holds.
- `faces`: the fonts panel's faces: every shipped face (`source`, shipped `bytes`, `subset`, `demoted`, `notice` for a
  face from this computer or the user, the deck faces it `serves`, its `licence`), then every face of the fonts added
  that never binds (`restricted` by its `fsType`, or `refused` by browsers' font sanitizer), whose substitute draws.
- `fontIssues`: the problems with the fonts held (`MID54xx` code and message).
- `substitutes`: the faces of the bundled substitutes (Liberation, Carlito, DejaVu Sans) the deck draws with no file
  yet (`family`, `postscriptName`, `weight`, `style`): `addFont` each one's file as `bundled`, then `finish` again,
  and they ship. A bundled file fills no face the text asks for: it replaces its family's metrics-only faces.

## Diagnostics and errors

`diagnostics { id, diagnostics }` batches arrive in output order; each adds to the earlier ones of the request. A
`finish` sends the complete list, the `open`'s diagnostics included, so it replaces the open's rather than adding to
them. The deck JSON also carries a summary (`deck.diagnostics`).

`error { id, error }` carries `error.kind`:

- `refused`: the input is not a presentation, or is encrypted, too large or corrupt;
- `failed`: the presentation failed as a whole, so nothing prints;
- `request`: a request out of order (`finish` before `open`, `bundle` before `finish`) or a file that is no font;
- `bundle`: the zip could not be written;
- `cancelled`: `cancel` dropped the request before it started;
- `internal`: the compiler crashed.

A crash (a wasm trap) leaves the module in an unknown state: the worker answers that request and every later one with
`internal`. Terminate it and start a new one. Debug builds (`wasm-pack build --dev`) print the panic's message to the
console first.

`error.message` is for bug reports; `error.diagnostics` say what a person can act on.

## Browser objects

`File`, `Blob` and `ArrayBuffer` fields cross `postMessage` as themselves: `File` and `Blob` by structured clone,
`ArrayBuffer` in the transfer list (`addFont.bytes`, `zipChunk.bytes`). Everything else is plain JSON-compatible
data. In Rust these fields are placeholders (`JsObject`), which the worker's JavaScript side fills.

## Folders

A Keynote package folder (older Keynote versions save a folder, and a folder drop gives its files) is sent as one
`File`. `packFolder(name, files)` builds it from the folder's files keyed by their path relative to the folder's
parent (`Deck.key/Index/Document.iwa`): a stored zip whose entries keep those paths, which the Keynote frontend reads
like a `.key` file. The zip is a `Blob` of the headers and the files themselves, so building it reads and copies
nothing; the worker then reads it by ranges like any file. `folderName(files)` gives the folder's name to pass as
`name`.

An `<input type="file" webkitdirectory>` gives each `File` its `webkitRelativePath`. A drag-and-drop gives `File`s
with an empty `webkitRelativePath`; `droppedFiles(entry)` walks the dropped `FileSystemEntry` instead and keys each
file by its `fullPath` without the leading `/`.

```ts
import { droppedFiles, folderName, packFolder } from '@slidock/wasm/folder';
import type { FolderFile } from '@slidock/wasm/folder';

const open = (files: FolderFile[]) =>
  send({ type: 'open', id: 1, file: packFolder(folderName(files) ?? 'Presentation.key', files) });

// <input type="file" webkitdirectory>
open([...input.files].map((file) => ({ path: file.webkitRelativePath, file })));

// A drop: the entry must be taken during the `drop` event; reading it may finish later.
const entry = event.dataTransfer?.items[0]?.webkitGetAsEntry();
if (entry?.isDirectory) {
  open(await droppedFiles(entry));
}
```

## Example

```ts
import type { FromWorker, ToWorker } from '@slidock/types';

const worker = new Worker(new URL('./compile-worker.ts', import.meta.url), { type: 'module' });
const send = (message: ToWorker, transfer: Transferable[] = []) => worker.postMessage(message, transfer);

worker.onmessage = ({ data }: MessageEvent<FromWorker>) => {
  switch (data.type) {
    case 'needs':
      // offer Local Font Access, Google Fonts and file drops for data.needs
      break;
    case 'deck':
      // mount data.slides with data.deck in the player; serve data.media as blob: URLs
      break;
    case 'zipChunk':
      // collect data.bytes into a Blob, or write them to a file handle
      break;
  }
};

send({ type: 'open', id: 1, file });
send({ type: 'finish', id: 2 });
```

## Tests

- `pnpm --filter @slidock/wasm test`: the message loop (`test/serve.test.ts`, with a fake compiler) and the built
  module under Node (`test/wasm.test.ts`: stages, typed errors, folders read back with `zipEntries`), on PPTX files
  written in code from `test/support/ooxml.ts`; `test/imports.test.ts` holds that the package imports nothing by path
  from another package's folder.
- `cargo test -p slidock-cli --features golden --test wasm_parity`: native and wasm write the same document
  sets and zips on the corpus, the e2e smoke deck and the font probes, Google Fonts off on both sides. It runs
  `test/node/compile.ts`, which drives the real worker (`src/serve.ts`) in a `worker_threads` Worker with a
  `FileReaderSync` stand-in over file descriptors, and adds the bundled substitutes as the web app does, from
  `apps/web/public/fonts/core/` (the files the CLI builds in).
