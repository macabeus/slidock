# Security policy

## Reporting a vulnerability

Please report vulnerabilities privately, through GitHub's private vulnerability reporting: open the repository's
**Security** tab and choose **Report a vulnerability**. Do not open a public issue, pull request or discussion for a
vulnerability until a fix is released.

A useful report says which component is affected (the CLI, the web app, the player in an exported deck, the Rust
crates), the version or commit, and how to reproduce it: ideally the smallest input file that shows it, or a script that
generates one. Reports are acknowledged within a week; a fix and an advisory follow as soon as the problem is
understood, crediting the reporter unless they ask otherwise.

## Supported versions

slidock is pre-1.0. Only the latest release is supported: security fixes land on `main` and ship in the next release.
There are no backports to older releases.

## Threat model

slidock compiles presentation files it did not write into decks other people open:

- **Inputs are untrusted.** A PPTX, ODP or Keynote file (and, for the CLI, an IR JSON document) may come from anyone and
  may be crafted: malformed zips and XML, Keynote object graphs that loop, media whose headers lie, text and geometry
  sized to exhaust memory or time. Reading one must not run code, read or write files other than the input and the
  output, reach the network beyond what is described under [Network use](#network-use), or crash the compiler (the
  library crates never panic; errors are diagnostics).
- **Outputs are published.** A deck (`index.html`, its media, or a single file) is meant to be hosted publicly and opened
  by anyone. Whatever the input held, opening the deck must not run script other than slidock's own player, load
  anything from outside the deck, submit forms, or navigate the viewer anywhere but the links the slides show.
- **Hosts.** The CLI runs on the user's machine or as a service; the web app runs the compiler (WebAssembly) in a Web
  Worker in the visitor's browser, so the file never leaves their computer.

Out of scope: what a deck _says_ (a slide can show any text, picture or link, including a misleading one; that is
content, not a vulnerability), bugs in browsers themselves, and resource use within the limits below when the host
does not enforce its own (see [Running slidock as a service](#running-slidock-as-a-service)).

## How the output is kept safe

A deck is HTML that slidock prints from its own intermediate representation; no markup, script or style from the input
is passed through. Four layers stand between an input and the viewer:

1. **SVG allow-list at compile time.** An SVG picture is shown only when it passes `slidock_io::check_svg` (elements,
   attributes and URL forms from a fixed allow-list; no script, no event handlers, no external references), and it is
   rewritten to the picture form slidock writes (`picture_svg`) before it is copied. Any other SVG is drawn from its
   raster preview or left out, with a diagnostic.
2. **Lint at compile time.** Every document the backend prints is linted against the format's element and attribute
   vocabulary (`slidock-format`'s lint, run by the driver in `crates/slidock/src/driver.rs`). A slide whose document fails
   is replaced by a failed slide (HTML6900); a problem no slide owns fails the compile.
3. **Sanitizer at mount time.** The player walks each slide's markup before adopting it (`packages/player/src/sanitize.ts`):
   an allow-list of elements, no event handlers, links only to `#`, `http:`, `https:` and `mailto:`, and every other URL,
   in attributes and in CSS (`isSafeResource`, `isSafeCss`), restricted to the deck's own files and inlined `data:`
   media. It is tested with generated hostile URLs and CSS (`packages/player/test/sanitize.property.test.ts`).
4. **Content-Security-Policy.** `index.html` carries a hash-only policy: `default-src 'none'`, scripts only by the
   SHA-256 of slidock's own inline scripts, images and media only from the deck's files, `data:` and `blob:`, fonts only
   inline, and no `connect-src`, frames, objects, forms or `<base>`.

Links and media the input points to outside itself (a linked picture, a remote video) are never fetched; they are
reported and left out.

### Presenter view

The presenter window and the deck window keep each other in step (slide, build step, blank screen) with `postMessage`
between the opener and the window it opened, and a `BroadcastChannel` named after the deck. This sync is
**unauthenticated by design**: positions are not secret, and a deck opened from `file://` has an opaque origin, so no
origin can be checked. Every message carries the deck's id and a random session id, and any other deck's or session's
message is ignored. A deck served over HTTP(S) posts only to its own origin and ignores messages from other origins, so
a page that opens or frames a hosted deck can neither read its position nor drive it. A deck opened from `file://`
accepts messages that name its deck and session from any window; the worst such a message can do is move the slides.

## Resource limits

Every limit is checked before the bytes it bounds are read, and defaults are set far above what real decks use (the
largest real decks we measure, up to 1 GB Keynote talks of 200 slides, peak under 500 MB). A presentation over a limit
is refused (its diagnostic names the limit) or, where only part of it is over, compiles without that part: a slide whose
output would pass the output budget is left out as a failed slide (IR0914), a raster over its pixel budget is left out
(MID5603, MID5610), and the rest of the deck compiles.

| What                                   | Default                                                                                   |
| -------------------------------------- | ----------------------------------------------------------------------------------------- |
| Input file                             | 2 GiB                                                                                     |
| Zip                                    | 20,000 entries, 64 MiB directory, 64 MiB per entry read, 512 MiB read in all              |
| XML                                    | depth 256, 1,000,000 nodes a part, 4,000,000 nodes a presentation, 512 attributes, no DTD |
| Slides                                 | 300; a slide part (or Keynote slide) listed twice is read once (PX2013, KN1012)           |
| Elements                               | 5,000 a slide, group depth 16; table cells 10,000 a slide, 50,000 a presentation          |
| Text                                   | 1,000,000 UTF-16 units a text body                                                        |
| Paths                                  | 100,000 segments a path                                                                   |
| Output budget (all slides, as lowered) | 25,000 elements, 1,000,000 path segments, 100,000 text runs, 500,000 gradient stops       |
| Media                                  | 512 MiB an item, 2 GiB a presentation                                                     |
| Rasters transcoded (TIFF, BMP)         | 25,000,000 pixels each, 200,000,000 a presentation; a HEIC a browser decodes: 50,000,000  |
| Time (with a clock)                    | analyze 30 s, lower 300 s, the whole compile 600 s                                        |

Time limits are cooperative: the compiler reads no clock of its own, so they apply only when the host gives it one.
The CLI does with `--timeout <SECS>`, which bounds the whole compile, from opening the input to the last file it writes,
and fails it with IR0905. Without `--timeout`, a CLI compile has no time limit.

Within these limits a deliberately hostile input still costs memory and time: the worst inputs we know peak at about
2 GB of resident memory and run for tens of seconds.

## Running slidock as a service

If you compile files from people you do not trust on a server, the limits above are not enough on their own: run each
compile in its own process and enforce memory, time, network and disk limits from outside.

- **One process per compile**, with the CLI (`slidock compile <input> --out <deck.zip>`), so a compile that is killed
  takes nothing else with it.
- **Memory**: cap it at about 4 GB. With systemd, `systemd-run --scope -p MemoryMax=4G -p MemorySwapMax=0 slidock …`;
  with cgroups v2 directly, `memory.max`; with Docker, `--memory 4g --memory-swap 4g`; on Linux without cgroups,
  `ulimit -v 4194304` (macOS ignores `ulimit -v`).
- **Time**: pass `--timeout` (for example `--timeout 300`) and also kill from outside after a little longer
  (`timeout -s KILL 330 slidock …`), since the CLI's limit is checked between stages.
- **Network**: pass `--offline`, so the compile makes no request (see below), and run without network access
  (`docker run --network none`).
- **Files**: give the process read access to the input only and a fresh output folder on a volume with a size limit (a
  deck copies the input's media, up to 2 GiB), and run it as an unprivileged user.

For example, with an image of your own that holds the release binary (here tagged `slidock`):

```sh
docker run --rm --network none --memory 4g --memory-swap 4g --cpus 2 --pids-limit 64 \
  --read-only -v "$PWD/in:/in:ro" -v "$PWD/out:/out" slidock \
  timeout -s KILL 330 slidock compile /in/talk.pptx --out /out/talk.zip --offline --timeout 300
```

The web app needs none of this: it compiles in the visitor's own browser, in a Web Worker it terminates when the visitor
cancels, so a hostile file can only exhaust that tab.

## Network use

- **CLI.** By default, a deck's faces that are still missing after the embedded and installed fonts are fetched from
  Google Fonts (`fonts.googleapis.com` and `fonts.gstatic.com`) for open-licence families, and cached in the user's
  cache folder. Those requests tell Google the font families the deck uses, and the requesting machine's address.
  `--offline` makes no network request at all (use it on servers); `--no-google` takes no face from Google Fonts, not
  even from the cache. The CLI makes no other request: links and media an input points to are never fetched.
- **Web app.** The file is compiled in the browser and never uploaded. The app fetches its own bundled font substitutes
  from its origin and, unless switched off in the fonts panel, missing open-licence faces from Google Fonts, as the CLI
  does.
- **Exported decks** load nothing from the network: the CSP above forbids it, and every font and medium is in the deck.
