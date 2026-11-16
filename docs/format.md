# The deck format

What slidock writes and what its player reads. A deck is a zip (or a folder) holding one HTML page, the media it
shows and the licences of the fonts it ships. Three version strings name the contracts: the deck JSON
`slidock-deck@1`, the IR `urn:slidock:presentation-ir:1` and the player stamp in each deck.

The TypeScript declarations of every type below are generated from the Rust types by `ts-rs` into
`packages/types/src/generated/` (`deck/`, `ir/`, `bundle/`, `worker/`). They are committed, and `slidock-wasm`'s
`typescript` test fails when they drift from Rust. Where this page and those files disagree, the files win.

## 1. The zip

`slidock compile deck.pptx -o deck.zip` and the web app's download write the same bytes for the same deck and
options (`slidock-bundle`). The entries sit at the root of the archive, in this order:

| Entry                  | What                                                                                                                |
| ---------------------- | ------------------------------------------------------------------------------------------------------------------- |
| `index.html`           | The whole deck: its text, slides, deck JSON, fonts and player (§2).                                                 |
| `media/<sha256>.<ext>` | Each image, poster and video the slides show, named by the SHA-256 of its source bytes.                             |
| `LICENSES.txt`         | slidock's licence (the player in `index.html` is part of slidock), then every shipped face's copyright and licence. |

- The zip is deterministic: every entry is dated 1980-01-01 00:00 and made by "MS-DOS" with no attributes, so an
  extractor gives the files its default permissions, and no entry carries an extra field but ZIP64's. Text entries are
  deflated whole (`miniz_oxide`, level 6). Media are stored, streamed with a data descriptor after their bytes.
  Offsets, the directory and the entry count switch to ZIP64 records past the 32-bit (16-bit for the count) limits;
  an entry is at most 4 GiB − 2 bytes.
- Media extensions are `png`, `jpg`, `gif`, `webp`, `avif`, `svg`, `mp4`, `webm` and `mov`: a browser shows them and
  none can run a script. An `svg` file is one that passed the compiler's allow-list (`slidock_io::check_svg`: strict
  UTF-8, no processing instruction and no DOCTYPE but `<!DOCTYPE svg PUBLIC|SYSTEM …>` with no internal subset,
  only plain drawing, text (`text`, `tspan`) and label elements and presentation attributes (and `xml:space`) in the
  SVG namespace, values with no CSS escape and no function but a colour, a transform or `url(…)` to its own fragments,
  a `style` sheet of only `@font-face` rules whose sources are fonts embedded as `data:` URLs (any quote closed by
  its match) and plain class, id or element rules of presentation properties, text drawn only in a family such a rule
  embeds (inside an `<img>` the SVG reaches no other font but the viewer's, so any other text would be drawn in a
  fallback font), a root whose size and `preserveAspectRatio` are read), checked again as it
  is written. It is written as a picture draws it (`slidock_io::picture_svg`): its DOCTYPE dropped, its root's
  `preserveAspectRatio` is `none` and its `viewBox` is what its own viewport showed, so it stretches to fill its `<img>` whatever the box's
  proportions, as Keynote, PowerPoint and LibreOffice stretch a picture. Any other SVG is left out. TIFF and BMP sources are transcoded to PNG. Audio is left out; a video in a codec no
  browser plays, a TIFF or BMP past the transcode's pixel limit, and any other type (EMF, WMF, PDF, …) are left
  out with a diagnostic.
- **HEIC photos** are left out (MID5601) unless the host decodes them. A picture that carries a raster beside its
  HEIC (Keynote's 256-pixel JPEG preview of an inserted photo, the IR's `fallback`) draws that raster instead, a loss
  stated with both sizes (MID5609). The web app has the browser decode each one
  the compiler plans (`slidock_io::read_heif`: one `hvc1` image, or a `grid` of `hvc1` tiles, with its `clap`, `irot`
  and `imir`), as Safari's image decoder or WebCodecs draws it, and hands the compiler a JPEG of the plan's output
  size; the deck then holds it as `media/<sha256 of the HEIC>.jpg`, transcoded (MID5608). Its bytes are the
  browser's: the same deck opened in another browser, or compiled by the CLI (which decodes none), holds another file
  or none. Given the same decoded file, the deck's bytes are the same.
- **Single file** (`--single-file`, the web app's option): when the media add up to at most 50 MiB, each counted once
  per reference in the slide templates (a layout's picture counts on every slide of that layout), they are inlined into `index.html` as `data:` URIs and `media/` is not written.
  Above the threshold they stay files.
- **Compressed images** (the web app's "Compress images", off by default): the compiler lists the still PNG, JPEG,
  WebP and GIF files with the size each needs (`slidock::images`: twice the largest box it is ever drawn in when the
  slide fills a 1920 × 1080 screen, over every use, crop, image fill, background, held scale-up and Morph or Magic
  Move match, never larger than it shows); the browser scales and re-encodes each one (WebP, or JPEG or PNG where its
  canvas encodes no WebP, its EXIF orientation applied) and the compiler takes a file only when it is of that size,
  still, unturned and smaller than the original. The deck then holds it as `media/<sha256 of the source>.<ext>` of its
  type in the original's place, in the slides and the deck JSON alike, inlined too in a single file. Every picture is
  drawn stretched to its box, so the layout is the same. SVG, animated GIF, APNG and WebP, AVIF and video stay as they
  are. Like a decoded HEIC photo, the bytes are the browser's; the CLI compresses nothing.
- **Compressed videos** (the web app's "Compress videos", off by default): the compiler lists the deck's MP4 and
  QuickTime movies with what each holds and the size it needs (`slidock::videos`: the pictures' rule over every use,
  the slides a clip plays across included, as it shows after its track matrix's quarter turn, each side even), or why
  it stays as it is (an alpha layer or channel, HDR, a matrix that mirrors or scales, non-square pixels, an edit list
  of more than one piece, more than one video or sound track, a fragmented or unreadable movie). The browser decodes
  and re-encodes each one with WebCodecs to H.264 at a variable bitrate of its quality (every frame at its own time,
  so the frame rate and the length stay; a sound in AAC carried over as it is, a PCM or MP3 one re-encoded to AAC)
  into an MP4 whose movie boxes come first, and keeps the original when its video already runs at or under that
  bitrate or the new file is not smaller (an encoder that drops frames is tried again, in software, then up a ladder
  of bits per pixel every quality shares, never past the original's). The compiler takes a file only when it is that MP4: one unturned H.264 track of the planned size with
  the original's frames, the original's sound as AAC or none, as long as the original and smaller. It is written
  as `media/<sha256 of the source>.mp4` in the original's place. A trim stays the player's (the file keeps its whole
  length, so a trim's times are unchanged), and so do a clip's poster, loop and mute. The CLI compresses nothing.
- **Copies of transparent videos** (the web app's "Play transparent videos in every browser", on by default): a
  video kept as it is for its alpha (HEVC with an alpha layer, which Safari and Chrome on macOS play with its
  transparency; a 32-bit QuickTime codec, Safari alone) gets a WebM copy that Chromium and Firefox play everywhere
  (`slidock::alternates`). The browser plays the original in a hidden `<video>`, takes each frame at the middle of its
  time with its alpha, scales it to the video's planned size and encodes its colour (BT.709, limited range) and its
  alpha (as luma) each with a VP9 (else VP8) encoder, the same frames key frames in both, every frame at its own time
  (ms); the WebM carries the alpha as `BlockAdditional`s (`AlphaMode` 1) and its cues before its clusters. The
  compiler takes the file only when it is that WebM: one VP9 or VP8 track of the planned size, every frame with its
  alpha, a key frame first, the original's frames, start, end and length, a sound track as the original (Opus or
  Vorbis) or none (none for a sound every use mutes, which is never heard), at most twice the original's size. It is written as `media/<sha256 of the source>.webm` right
  after the original and listed in the deck JSON's `media` (with the original's `sha256`); each `<video>` of the
  original holds two `<source>`s in place of its `src`: the original (or the file "Compress videos" writes in its
  place: a video muted in every use, its track copied without its sound), its type naming its bare sample format
  (`video/quicktime; codecs="hvc1"`, `video/mp4; codecs="hvc1"`), then the copy (`video/webm; codecs="vp9"`). WebKit takes the first; Chromium,
  Chrome and Firefox refuse that type (Chrome and Firefox decode HEVC, but Firefox without its alpha layer) and play
  the copy. A trim, poster, loop and mute are the `<video>`'s, so they hold for both files. A browser that does not
  play the original with its alpha (Chromium, Firefox: no HEVC) makes no copy, and the dialog says the video plays
  only in Safari and Chrome on a Mac. The copy's bytes are the browser's: Safari's copy shows the colours both
  engines show the original in; Chrome's is a few levels darker (its canvas converts an HEVC frame's colour unlike
  its compositor). The CLI makes none.
- Without `.zip` at the end of `-o`, the CLI writes the same files into a folder.

The deck opens from `file://`, from any static host, and inside the web app.

## 2. `index.html`

<!-- prettier-ignore -->
```html
<!doctype html>
<html lang="…">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta http-equiv="Content-Security-Policy" content="…">
<title>…</title>
<meta name="description" content="…">                 ← when the page has a description
<meta property="og:type" content="website">
<meta property="og:title" content="…">
<meta property="og:description" content="…">          ← when the page has a description
<meta name="twitter:card" content="summary">
<link rel="canonical" href="…">                        ← only with a public URL
<meta property="og:url" content="…">                   ← only with a public URL
<style id="sd-chrome">…</style>
<noscript><style>#sd-text{margin-top:0}</style></noscript>   ← with a text view
</head>
<body>
<article id="sd-text" aria-label="Slides as text">…</article>
<style id="sd-fonts">@font-face{…}…</style>
<script type="application/json" id="sd-deck">{…}</script>
<template data-sd-slide="<slide id>"><main data-slide="<slide id>" …>…</main></template>
…
<script>/* @slidock/player's IIFE build */</script>
<script>/* the presenter view's IIFE build */</script>          ← with a presenter view
<script>Slidock.mount(document.body, Slidock.inlineDeck(document), { presenter: SlidockPresenter })</script>
</body>
</html>
```

- Everything an indexer needs comes before anything unbounded: the head and the text view lead, then the fonts, the
  deck JSON, the templates and the player. Googlebot reads the first 2 MiB of a file, so a page made large by whole
  fonts or single-file media keeps its text inside that part (the text view ends within the first ~60 KB of every
  measured deck); the CLI notes an `index.html` past 2 MiB, and the web app's download says so. `<style>` in
  `<body>` is not conforming HTML, but every engine applies it, and readers find `sd-fonts` and `sd-deck` by their
  tags, not by their place. The player's `inlineDeck` moves `sd-fonts` into `<head>` as it reads the page, so a host
  that then replaces the body's content (the bench's lanes mount the deck that way) keeps the deck's faces. The page holds exactly four `<script>` elements, the deck JSON first, or three
  without a presenter view.
  The text view's bytes come before the player's, so they delay the first slide by the time they take to arrive:
  served uncompressed on a slow link that is measurable (the motion probe's 50 KB view, ~290 ms of LCP on Slow 4G),
  served compressed (gzip or brotli, as static hosts do) it is a few KB (8 KB gzip, ~80 ms).
- `lang` is the deck's `lang`, left out while it is `null`.
- The Content-Security-Policy runs only the page's own scripts (the player, the presenter view's, the boot), by
  hash, and makes no request but the deck's own media:

  ```text
  default-src 'none'; script-src 'sha256-…' 'sha256-…'; style-src 'unsafe-inline';
  img-src 'self' data: blob: file:; media-src 'self' data: blob: file:; font-src data: blob:;
  connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'
  ```

- The page's metadata (`slidock_bundle::page_defaults`, the same function in the CLI and the web app):
  - `<title>`, `og:title`, the text view's `h1` and the deck JSON's `title` are the page's title: `--title` (the
    download dialog's Title), trimmed, unless blank; else the first shown slide's title, else the input's name
    without its extension. The manifest of `--emit documents` keeps the input's name.
  - The description is `--description`, trimmed (blank for none), else the shown slides' distinct titles after the page title,
    in order, joined with `·` and cut at the last whole title within 160 characters; none when there is none.
  - `<link rel="canonical">` and `og:url` appear only with `--site-url` (the dialog's Public URL): an absolute
    `http(s)` URL with a host, no user name or password and no fragment. Anything else is refused (exit 2), never
    repaired. Without it the page names no absolute URL.
  - No `og:image`, no robots directive, no date: the page stays deterministic and indexable.
- `<style id="sd-chrome">` makes the page black and lays out the text view as a reading column (46rem wide at most,
  16px system text in `#e5e7eb`, links in `#93c5fd`, bordered tables) on a black band as wide as the window. Nothing
  in it is hidden, off-screen, tiny or of the background's colour. The player styles its stage, counter and overlays
  itself.
- **The page frame.** The player puts its stage first in `<body>`, fixed to the window as on a page without a text
  view (`position: fixed; inset: 0`): the first screen is the slide, letterboxed, and Chromium draws it exactly as
  before (a stage in the page flow, or one on a layer of its own, drew emphasis builds' end states and the slide's
  edges differently at fractional device pixel ratios). The text view starts one window down (`margin-top: 100vh`,
  `100dvh` where supported) and sits above the stage in z-order, opaque, so scrolling brings it up over the stage,
  which stays where it is; the chrome's Text link scrolls to it. The margin is there from the start, so the text view
  never moves when the player, whose script comes last in the file, mounts (no layout shift); without scripts the
  `<noscript>` style in the head drops it and the text view is the page. Only a body that holds no `#sd-text` hides
  its overflow (`body:not(:has(>#sd-text))`): a page without a text view (whose `sd-chrome` hides it outright) and a
  host that replaces the body's content (the bench's lanes) keep the black, full-window page with nothing to scroll,
  and a browser without `:has()` drops the rule and scrolls to the text view. Presenting fills the window or the
  screen over it, and the page does not scroll meanwhile. The presenter window (`#presenter`) removes the text view.
  On systems with classic (non-overlay) scrollbars the page now shows one, so the stage is fitted to a viewport that
  much narrower, exactly as in a window that much narrower: the slide's pixels are scaled, not changed. Headless
  renderers hide scrollbars, so the bench cannot see this; it is accepted.
- **The text view** (`<article id="sd-text">`, `slidock_bundle::text_view`; `--no-text-view` and the dialog's "Text
  version below the slides" leave it out, and the page then has the old `<noscript>` sentence):
  - a header with the page title as `h1` and `<noscript><p>The slides need JavaScript to play; their text
follows.</p></noscript>`;
  - one `<section id="slide-<n>"><h2><a href="#slide-<n>">{title}</a></h2>…</section>` per shown slide (`<n>` its
    number among the shown slides), in show order, headed by its title or `Slide <n>`; hidden slides are left out;
  - what the composed slide shows fully built, its own elements first, then its layout's, the background left out:
    each `<p>` and `<li>` with text (lists as `ul`/`ol`; an item of a numbered list leads with the label its slide
    shows, such as `3.` or `b)`, and the view draws no marker of its own for it), tables as rows and cells' text, `<br>` kept, other inline
    markup as text; `Image: <alt>` for a picture with alt text (or a `role="img"` element's label); nothing for
    `alt=""`, `hidden` elements, `aria-hidden` labels, SVG geometry, media, slidock's markers and slide-number
    fields;
  - links: `http`, `https` and `mailto` links stay; a slide link points at its section (`#slide-<n>`, plain text
    when its target is hidden); action links are unwrapped; an element linked to a URL or a slide links its text,
    or prints one link (its alt text, else the URL, else the target slide's heading);
  - the first run of blocks that makes up the heading (the title's paragraphs) is left out unless it holds a link,
    and so is a block the previous shown slide showed too (by
    text and links, each earlier copy covering one), so a run of build slides shows each block once; a slide left
    with nothing new under the previous section's heading is an anchor (`<span id="slide-<n>">`) at that section's
    end;
  - speaker notes only with `--notes-on-page` (the dialog's "Also on the page"): `<details open><summary>Speaker
notes</summary><p>…</p></details>` after the section's blocks, a line break as `<br>`. Hidden slides' notes
    never reach the view. The next item says where else notes go.
- `<style id="sd-fonts">` holds one `@font-face` rule per weight range of each shipped face: the subset file as a
  `data:font/woff2;base64,…` URL, with the descriptors of the file itself (its true weight and style).
- `<script id="sd-deck">` holds the deck JSON (§4), with every `<` written `\u003c` and U+2028 and U+2029 as escapes,
  so nothing in it can end the element. It parses to the same value.
- One inert `<template data-sd-slide>` per slide, in show order (§3). Its images load only when the player mounts
  the slide.
- The player is `@slidock/player`'s IIFE build, which defines the global `Slidock` (`docs/player-api.md`); the boot
  script mounts it on the page. `#slide-<n>` in the page's URL starts at that slide, and the URL follows the show
  (`docs/player-api.md`, deep links).
- **The presenter view and the speaker notes.** By default the page ships the presenter view: the presenter's IIFE
  build (`dist/presenter.iife.js`, global `SlidockPresenter`) after the player's, handed to `Slidock.mount` by the
  boot. P (and the help's "P Presenter view" row) opens the same page at `#presenter=<session>` in a second window,
  which draws the web app's presenter window: the slidock header with the deck's title and the theme toggle, the
  current slide, the next click's preview, the notes, the timers and the controls (`docs/player-api.md`).
  `--no-presenter` (the dialog's "Presenter view" unticked) leaves the script out: the page has no P key and no help
  row, joins no sync channel, and at `#presenter` shows the deck. Notes can be private, so they ship only where the
  page shows them: in the deck JSON's `notes` for the presenter view, and in the text view with `--notes-on-page`.

  | `--no-presenter` | notes flags         | `<script>`s | `player.presenter` | deck JSON `notes` | text view notes |
  | ---------------- | ------------------- | ----------- | ------------------ | ----------------- | --------------- |
  | (absent)         | (none: the default) | 4           | its SHA-256        | every slide's     | none            |
  | (absent)         | `--notes-on-page`   | 4           | its SHA-256        | every slide's     | shown slides'   |
  | (absent)         | `--no-notes`        | 4           | its SHA-256        | empty             | none            |
  | given            | (none)              | 3           | `null`             | empty             | none            |
  | given            | `--notes-on-page`   | 3           | `null`             | empty             | shown slides'   |
  | given            | `--no-notes`        | 3           | `null`             | empty             | none            |

  `--notes-on-page` needs the text view and the notes (it conflicts with `--no-text-view` and `--no-notes`). The
  dialog says the same with a "Speaker notes" checkbox ("Also on the page" under it) while the presenter view ships,
  and a single "Speaker notes on the page" checkbox while it does not. The CLI, the web app's download and the
  single-file form write the same page for the same choices; the zip's other files (`media/`, `LICENSES.txt`) hold
  no notes.

## 3. Slide markup

A template holds the slide composed with its layout at bundle time: one `<main data-slide="<id>">`. The markup is
static, script-free and fully built: it is what the slide shows once every build has played. Every element is
positioned absolutely in points inside a `<main>` the size of the slide (`width:<w>pt;height:<h>pt`).

| Marker                     | Where                                                                                                         |
| -------------------------- | ------------------------------------------------------------------------------------------------------------- |
| `data-slide="<id>"`        | the slide's root `<main>`                                                                                     |
| `data-layout="<id>"`       | on the root: the layout composed under the slide                                                              |
| `data-show-layout="false"` | on the root: the layout's elements are hidden                                                                 |
| `data-sd-bg`               | the background box, always the root's first child: the slide's own, else the layout's                         |
| `data-el="<element id>"`   | every printed element, one per visible object; the id the timeline's targets name                             |
| `data-kind`                | the element's kind (`shape`, `picture`, `media`, `table`, `group`, `foreign`)                                 |
| `data-sd-text`             | a text box                                                                                                    |
| `data-p="<n>"`             | a paragraph block of a text box, numbered from 0: what a paragraph range builds                               |
| `data-sd-bullet`           | a bullet label: a by-paragraph build animates it with its paragraph, a by-word or by-character build skips it |
| `data-sd-geom`             | an SVG geometry layer                                                                                         |
| `data-sd-mask`             | a picture mask                                                                                                |
| `data-sd-clip`             | the box a shadowed picture clips in: its root casts the shadow, which its own clip would cut                  |
| `data-sd-marker`           | a foreign object's marker                                                                                     |
| `data-sd-match="<key>"`    | an element's Magic Move / Morph match key                                                                     |
| `data-failed="<code>"`     | on the root of a failed slide: the diagnostic code that failed it                                             |
| `data-sd-failed`           | the dashed box a failed container prints in place of its elements, showing the code                           |
| `data-field="slideNumber"` | a field showing the slide's 1-based place in show order, hidden slides counted                                |
| `--sd-color-<name>`        | theme colour variables (`var(--sd-color-tx1,#ffffff)`)                                                        |

Media are referenced as `media/<sha256>.<ext>` (or a `data:` URI in a single-file deck). Text is in text nodes,
lists are real `ul`/`ol`/`li`, and a video is `<video preload="none" poster>`: the poster is its static truth. A video
offered in two files (a transparent video's copy, §1) holds a `<source>` of each instead of a `src`; the player takes
its first `<source>` as its file.

The player reads a video's playback from its attributes: `data-autoplay` (it starts as its slide shows, while
presenting), `data-loop`, `data-muted`, `data-trim="<start>-<end>"` (milliseconds; the end may be absent) and
`data-across-slides`. A clip marked `data-across-slides` plays across a forward move (`next`) onto a slide that holds
the same clip (the same file, its `src` or its first `<source>`'s, and the same `data-trim`), also marked: there it goes
on from where it stood, playing if it played and holding its last frame if it had ended, and neither its autoplay nor
its play steps start it again (a stop step does). A clip that plays goes on through the transition too: once the
incoming slide's copy shows the frame its pair stands at, it plays along, muted, so the move shows the clip going on,
not the frame the move began on. The copy starts behind its pair by the time its seek took (0.1 to 0.3 s in Chromium and
WebKit), and that much of the clip plays again at the hand-over: a seek or a rate change of a playing clip stalls it in
WebKit for longer. Each clip pairs once, in document order, and only with one that has played; any other move starts
every clip from its trim start. It is Keynote's "play across slides" (`TSD.MovieArchive.playsAcrossSlides` on both
movies); PowerPoint and ODF have no such link between two slides' clips.

The lint (`slidock-format`) runs over every document before the page is written. It allows only an allow-list of
HTML and SVG elements, and refuses scripts, `<style>` beyond the reset rule, inline event handlers, `javascript:`
URLs, links other than a fragment or an `http`, `https` or `mailto` URL, any load but the deck's own media,
`position: fixed` and viewport units. The player checks the markup again when it mounts a slide (an allow-list walk;
what it removes is reported as `RT0002`).

## 4. The deck JSON (`slidock-deck@1`)

The manifest the player reads. It is canonical JSON: keys in UTF-16 order, numbers printed as JavaScript prints them,
so the same compile writes the same bytes on every host.

### 4.1 `Deck`

| Field         | Type                                     | What                                                                                                                                                                                                                                                                   |
| ------------- | ---------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `format`      | `"slidock-deck@1"`                       | the format version                                                                                                                                                                                                                                                     |
| `player`      | `{version, sha256, presenter}` or `null` | the player embedded beside it: its version and the SHA-256 (lowercase hex) of the script, and `presenter`, the SHA-256 of the presenter view's script, `null` on a page without one (§2). `null` in a deck no bundle wrote (the web app's, which its own player shows) |
| `id`          | string                                   | the deck's content hash (below); presenter windows key their channel on it                                                                                                                                                                                             |
| `title`       | string                                   | the input's name without its extension; in a bundle, the page's title (§2)                                                                                                                                                                                             |
| `lang`        | string or `null`                         | the BCP 47 language of the deck's text; `null` while no frontend reads one                                                                                                                                                                                             |
| `size`        | `{w, h}`                                 | the slide size in points                                                                                                                                                                                                                                               |
| `source`      | `{format, name, sha256, bytes}`          | the input: `pptx`, `odp`, `keynote` or `ir`, its file name, SHA-256 and size                                                                                                                                                                                           |
| `slides`      | `DeckSlide[]`                            | show order, hidden slides included (§4.2)                                                                                                                                                                                                                              |
| `media`       | `DeckMedia[]`                            | the files under `media/`: `path` (`media/<sha256>.<ext>`), `mime`, `sha256` (of the source item) and `bytes` (the source's size for a copy, `null` for a transcode)                                                                                                    |
| `fonts`       | `DeckFont[]`                             | the faces the deck's text draws (§4.3); slides list theirs by index                                                                                                                                                                                                    |
| `diagnostics` | `DiagnosticSummary`                      | `total`, then one count per severity (`error`, `decline`, `lossy`, `warning`, `info`), and `codes`: `{code, severity, count}` per code                                                                                                                                 |

`id` is the SHA-256 of the canonical JSON of the deck with `id` empty and `player` `null`, together with the SHA-256
of every text document of the set (`presentation.json` and each slide and layout document, §7): the same compile has
the same id, whatever player it ships with.

### 4.2 `DeckSlide`

| Field        | Type                            | What                                                                                                                                                                                                            |
| ------------ | ------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `id`         | string                          | the slide id (`pp-256`, `od-p1`, `kn-2652174`), as its template names it                                                                                                                                        |
| `layout`     | string or `null`                | the layout composed under it                                                                                                                                                                                    |
| `hidden`     | boolean                         | presenting skips it; viewing shows it                                                                                                                                                                           |
| `number`     | number or `null`                | its 1-based number among the shown slides; `null` for a hidden slide                                                                                                                                            |
| `title`      | string                          | the title for navigation and assistive technology; `""` when it has none                                                                                                                                        |
| `notes`      | string[]                        | speaker notes as plain paragraphs: breaks as `\n`, tabs as `\t`, fields as their values, trailing blank paragraphs and whitespace removed; empty in a bundle without a presenter view or with `--no-notes` (§2) |
| `failed`     | string or `null`                | the code of the diagnostic that failed the slide; its template then holds the failed marker                                                                                                                     |
| `timeline`   | `DeckStep[]`                    | its build steps, in order (§5)                                                                                                                                                                                  |
| `transition` | `DeckTransition` or `null`      | the transition into it (§5.3)                                                                                                                                                                                   |
| `matchKeys`  | `{[element id]: key}`, optional | the Magic Move / Morph keys of its own elements (group children and foreign fallbacks included); absent when it has none                                                                                        |
| `fonts`      | number[]                        | the faces it draws, its layout's included, as ascending indexes into the deck's `fonts`: what the player loads before it shows the slide                                                                        |

### 4.3 `DeckFont`

| Field     | What                                                                                                                                                                                                                                                                                           |
| --------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `family`  | the family the slides name                                                                                                                                                                                                                                                                     |
| `weight`  | the weight drawn, 100 to 900                                                                                                                                                                                                                                                                   |
| `style`   | `normal` or `italic`                                                                                                                                                                                                                                                                           |
| `source`  | where the shipped file came from: `embedded` (in the input), `local` (installed on the computer that made the deck), `google`, `user` (a file the user added), `bundled` (one of slidock's substitutes), or `system` (no file ships: the viewer's installed font, or its substitute, draws it) |
| `licence` | the shipped file's copyright and licence; `null` when no file ships                                                                                                                                                                                                                            |

A face whose file was left out of the download (the per-family toggle for faces from this computer or the user) or
whose licence keeps it out is listed as `system` with no licence.

### 4.4 Versioning

A reader ignores the fields it does not know, so `slidock-deck@1` grows by new optional fields. A new value of an
existing field (an enum variant, a meaning) makes a new format version, and a reader refuses a version it does not
know (`parseDeck` throws). The effect vocabulary is the exception: an `effect` a reader does not know reads as none,
and the step or transition plays its canonical `fallback`, so the vocabulary grows within a version.

## 5. The timeline the player reads

A slide's `timeline` lists its build steps in order. The player groups them: group 0 is the run of automatic steps
at the start, every `click` step opens a group, and the `withPrevious` and `afterPrevious` steps after it join it. A
position is `(slide, step)`, where `step` counts the click groups played (0 to the slide's click-group count), and a
slide's state depends only on its position.

### 5.1 `DeckStep`

| Field              | Type                                              | What                                                                                                                                                                                      |
| ------------------ | ------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `target`           | `{element, paragraphs, background?}`              | the element (`data-el`); `paragraphs` `[first, last]` (inclusive, from 0) or `null` for the whole element; `background: true` builds the shape without its text (PPTX `animBg`)           |
| `trigger`          | `click`, `withPrevious`, `afterPrevious`          | what starts it                                                                                                                                                                            |
| `kind`             | `entrance`, `exit`, `emphasis`, `motion`, `media` | what it does to its target                                                                                                                                                                |
| `delivery`         | `all`, `byParagraph`, `byWord`, `byChar`          | how its text builds                                                                                                                                                                       |
| `delayMs`          | number                                            | its delay                                                                                                                                                                                 |
| `durationMs`       | number                                            | its length                                                                                                                                                                                |
| `native`           | string                                            | the source's own effect, `<format>:<name>` (`pptx:entr.2/4`, `odp:ooo-entrance-fly-in`, `keynote:apple:move in`): provenance, never decoded by a player                                   |
| `fallback`         | `appear`, `dissolve`, `none`                      | what every player can play                                                                                                                                                                |
| `effect`, `params` | optional                                          | the effect in the common vocabulary (§5.2), when the source's maps to one                                                                                                                 |
| `offsetMs`         | optional, ms                                      | its absolute start inside its click group, delay included (PPTX); a player uses it instead of chaining `afterPrevious` ends                                                               |
| `easing`           | optional                                          | `linear`, `in`, `out`, `in-out`, `{bezier: [x1, y1, x2, y2]}` or `{accelDecel: {accel, decel}}` (fractions of the length spent speeding up and slowing down)                              |
| `stagger`          | optional                                          | per-unit start offsets of a by-paragraph, word or character delivery: `perUnit` (`{ms}` or `{fraction}`) and `order` (`forward`, `backward`, `fromCenter`, `fromEdges`, `{random: seed}`) |
| `repeat`           | optional                                          | `{count}`, `untilNextClick` or `untilEndOfSlide`                                                                                                                                          |
| `autoReverse`      | optional, boolean                                 | each repeat plays forward, then backward                                                                                                                                                  |
| `after`            | optional                                          | what happens to the target after the step: `keep`, `hide` or `{dim: "#rrggbb"}`                                                                                                           |

### 5.2 The effect vocabulary

`effect` names the effect and `params` holds its parameters (`slidock-ir::effects`; the parameter types are in
`packages/types/src/generated/ir/`). A direction is always the one the content moves, in OOXML's tokens
(`l r u d lu ru ld rd`): "fly in from the left" is `r`.

Build effects: `appear`, `fade`, `dissolve`, `flyIn {dir, distance}`, `floatIn {dir}`, `peek {dir, slow}`,
`wipe {dir, soft}`, `split {orient, inOut}`, `iris {shape, inOut}`, `wheel {spokes, cw}`, `wedge`, `clock {cw, from}`,
`blinds {orient}`, `checker {dir}`, `randomBars {orient}`, `strips {corner}`,
`zoom {inOut, origin, slight, fade, overshoot}`, `grow`, `expand`, `compress`, `stretch`, `swivel`, `flip` (each
`{axis}`), `growTurn`, `spinIn {deg, cw}`, `bounce`, `drop`, `pop`, `boomerang`, `sling`, `whip`, `swish`,
`typewriter`, `credits`, `driftAndScale {dir}`, `fadeAndMove {dir}`, `random`, `pulse {scale}`,
`growShrink {scale, hold}`, `spin {deg, cw, hold}`, `teeter`, `jiggle`, `wave` (each `{intensity}`), `blink`,
`flash`, `boldFlash` (each `{repeat}`), `transparency {alpha, hold}`, `colorChange {property, to}`,
`path {d, autoRotate, hold}`, `play {from}`, `pause`, `stop`, `toggle`, and `behaviors`, whose `params` is a list of
keyframe tracks (an unknown preset whose primitives are known: `opacity`, `translate`, `rotate`, `scale`, `blur`,
`clipPath`, `text`, `fill`, `line`, `bold`, `italic`, `underline`, and `set` switches).

Transition effects: `none`, `cut {throughBlack}`, `fade {through, color?}`, `dissolve`, `push {dir}`,
`cover {dir}`, `uncover {dir}`, `wipe {dir, soft}`, `split {orient, inOut}`, `iris {shape, inOut}`,
`wheel {spokes, cw}`, `clock {cw, from}`, `wedge`, `blinds`, `randomBars`, `comb`, `doors`, `window` (each
`{orient}`), `checker`, `flip`, `cube`, `gallery`, `conveyor`, `ferris`, `pan`, `switch`, `rotate3d`, `swap`,
`pageFlip`, `twist`, `mosaic` (each `{dir}`), `strips {corner}`, `zoom {inOut, rotate}`, `reveal {dir, throughBlack}`,
`flythrough {inOut, bounce}`, `morph {by}`, `magicMove {fadeUnmatched}` and `random`.

### 5.3 `DeckTransition`

| Field              | What                                                                                                                                                                                |
| ------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `native`           | the source's own effect, `<format>:<name>`                                                                                                                                          |
| `fallback`         | `none`, `dissolve`, `push` or `wipe`                                                                                                                                                |
| `durationMs`       | its length                                                                                                                                                                          |
| `advance`          | `{onClick, afterMs}`: whether a click advances, and the delay after the transition's end before the slide advances by itself (`null` for none); `null` when the source says nothing |
| `effect`, `params` | optional: the effect in the common vocabulary (§5.2)                                                                                                                                |
| `easing`           | optional, as for a step                                                                                                                                                             |

### 5.4 How the player reads it

The deck JSON is untrusted input; the player reads it fail-closed and never throws on a timeline:

- a step that is not an object, whose target names no element, or whose `kind` or `trigger` is not in the contract
  is dropped (`RT0008`); an unknown trigger is an interactive one, which the compilers leave out too;
- an unknown `delivery` builds the target whole, a malformed paragraph range builds the whole element, an unknown
  `fallback` plays `appear`, and a `native` that is not a string reads as empty (`RT0007`);
- a slide plays at most 2,000 steps and addresses at most 10,000 build units (whole elements, backgrounds, paragraphs
  of a range); a step past either is dropped, and one step staggers at most 10,000 units;
- an `effect` the player does not know, or one it cannot play there, plays the step's or transition's `fallback`
  (`RT0006`); a timing value out of the contract plays its declared default (`RT0007`).

Moving forward animates and moving back cuts. A step's start is its `offsetMs` when given, else the previous step's
start (`withPrevious`) or end (`afterPrevious`), plus its delay; its end includes its repeats, auto-reverse and
stagger, and an endless repeat counts one round. With reduced motion, transitions become a 150 ms fade and builds
`appear`.

## 6. The IR (`urn:slidock:presentation-ir:1`)

The presentation IR is what frontends write and passes refine. Its types are `slidock-ir`'s (`crates/slidock-ir`),
with their TypeScript declarations in `packages/types/src/generated/ir/`; this section is an overview. It appears as a
file in two places:

- `slidock compile <input> --emit ir` writes the canonical dump, `presentation-ir.json` in the `-o` folder or on
  stdout. It is the resolved phase unless `--phase source` stops after lowering. `--native-motion` (with `--emit ir` and no
  documents) keeps the motion a deck plays (the vocabulary effects, timing, match keys); without it the dump is the
  parity path's.
- The CLI reads an IR file named `*.presentation-ir.json` as an input, with its media from `--media` (default:
  `media/` beside it). This is a test input (the `ir-input` feature); the web app's wasm build refuses it (IR0902).
  Only `urn:slidock:presentation-ir:1` is read: a document whose `$schema` is absent, null or anything else is
  refused with exit 2 and one diagnostic, IR0001 (stage `frontend`, emitter `ir`, data `{path: "$schema", reason}`),
  and `report.json` has `provenance: null`.

The dump is canonical JSON with two-space indentation, keys in UTF-16 order. A presentation holds:

| Field                | What                                                                                                                                                                 |
| -------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `$schema`            | `urn:slidock:presentation-ir:1`, in every dump; the IR frontend recognises a document by it and refuses one naming another or none (IR0001)                          |
| `phase`              | `source` (as a frontend wrote it) or `resolved` (after the passes)                                                                                                   |
| `size`               | `{w, h}`, the slide size in points                                                                                                                                   |
| `defaults`           | `{text, body}`: how the format states its default text styles and body properties                                                                                    |
| `themes`             | by theme id: `id`, `origin`, `name`, `colors` (the scheme), `fonts` (`major` and `minor`, each `latin`, `ea`, `cs`), `ext`                                           |
| `masters`, `layouts` | by id: the containers slides inherit from                                                                                                                            |
| `slides`             | show order, hidden slides included                                                                                                                                   |
| `fonts`              | keyed by the family's key: each family's `request` (family, faces, PostScript names, embedded) and, resolved, its `binding` (the family drawn, its kind and metrics) |
| `media`              | keyed by SHA-256: each item's type, size, origin and, resolved, its `plan` (copy, transcode or omit, and the output file)                                            |
| `ext`                | optional, a frontend's own data                                                                                                                                      |

A slide holds `id`, `origin`, `layout`, `hidden`, `showLayout`, `colorMapOverride`, `notes`, `transition`,
`timeline`, `ext`, `background`, `elements`, `failed` and, written by the a11y pass, `title` and `readingOrder`. An
element is a `shape`, `picture`, `media`, `table`, `group` or `foreign` (its `kind`), with an `id`, an `origin` and
a `frame`. A PPTX element's id extends its slide's (`pp-257.3`).

- **Units.** Lengths are points (1/72 in, y down) and angles degrees (clockwise), on a grid of thousandths; ratios
  (opacities, fractions, colour components, scales, crops) on a grid of hundred-thousandths; milliseconds are
  integers up to an hour. A number off its grid fails verification (IR0007) and one past its bounds (IR0008);
  passes quantise with JavaScript's `Math.round` semantics, so every host prints the same numbers. Slide pixels
  (`px`, for motion paths and keyframe translations) are 0.75 pt.
- **Timeline.** A step is `{target, trigger, delayMs, durationMs, kind, effect, delivery, timing?}`, where `effect`
  is `{id, fallback, native?}`: `id` is the opaque `<format>:<native>` id, `fallback` the canonical one, and `native`
  the vocabulary effect (`{id, effect, params}`). `timing` holds `offsetMs`, `easing`, `stagger`, `repeat`,
  `autoReverse` and `after`. A transition is `{effect, durationMs, advance?, easing?}`. The deck JSON flattens each:
  its `native` is the vocabulary effect's id, else the IR's `id`, and its `effect`/`params` come from `native`.
- **Limits** (the verifier's): 300 slides, 5,000 elements per container, group depth 16, an inheritance chain of 8,
  1,000,000 UTF-16 units of text per body, 16 KiB of `ext` per node and 1 MiB per presentation, payloads of 256 KiB,
  10,000 table cells per container and 50,000 per presentation.
- **Growth.** Version 1 grows only by optional fields, left out when unset, so an IR without them round-trips byte
  for byte.

## 7. The document set (`--emit documents`)

For byte parity and debugging, not part of the product format. `slidock compile <input> --emit documents -o <dir>`
writes:

| Path                     | What                                                                                                                                                                                                                                                                                                                                    |
| ------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `presentation.json`      | the manifest: `title`, `size` (`{w, h, unit: "pt"}`), `slides` (id, file, title, hidden, and when set layout, notes, transition `{effect, fallback, durationMs, advance?}`, failed), `layouts` (id, file, name, theme), `themes`, `source` and `generator` (`{name: "slidock", version}`); no timestamps, absolute paths or diagnostics |
| `slides/<file id>.html`  | one standalone HTML document per slide: the slide's own elements, referencing its layout                                                                                                                                                                                                                                                |
| `layouts/<file id>.html` | one per layout                                                                                                                                                                                                                                                                                                                          |
| `media/<sha256>.<ext>`   | the media the documents reference                                                                                                                                                                                                                                                                                                       |

A slide document's root is `<main data-slide="<slide id>">`, carrying `data-layout`, `data-show-layout` and
`data-failed` as in §3. A layout document's root is `<main data-layout-root="<layout id>">`, with `data-failed` when
the layout failed; it holds the layout's background (else its master's) and the non-placeholder elements of its
shown master, then its own. Compose finds a slide's layout through the slide root's `data-layout` and the layout
file's `data-layout-root`. The lint refuses a document whose `<main>` does not carry its entry's id under the
attribute of its kind (`document-root`), and a slide naming a layout that has no layout file (`layout-missing`).

`--emit report` adds `report.json` (provenance, every diagnostic with its code, severity, container, emitter, stage
and data, the features the frontend detected, and which containers each slide shows), and `--emit ir` adds
`presentation-ir.json`. The bench's static lane and the goldens use `--emit documents,report,ir`, with font sources
and native motion off.
