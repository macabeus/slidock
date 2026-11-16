# The player API

`@slidock/player` (`packages/player`) plays a slidock deck: it mounts each slide in its own shadow root on one stage,
plays builds and transitions with the Web Animations API, and syncs a presenter view in a second window. It has no
dependencies, and it is part of the deck format (`docs/format.md`): every deck embeds its IIFE build and records its
version and SHA-256.

`pnpm --filter @slidock/player build` builds it twice, and the presenter view (`src/presenter/`) twice beside it:

- `dist/player.mjs`, the ES module the web app imports (`import … from '@slidock/player'`);
- `dist/player.iife.js`, a classic script that defines the global `Slidock` with the same exports. `slidock-bundle`
  embeds it in every deck's `index.html`;
- `dist/presenter.mjs` (`@slidock/player/presenter`), which imports the player from `@slidock/player`;
- `dist/presenter.iife.js`, which defines the global `SlidockPresenter` on a page whose `Slidock` is defined (it
  reads the player from it rather than bundling it again). `slidock-bundle` embeds it after the player in a deck that
  ships a presenter view.

The player alone has no presenter view: no P key, no help row, no sync, and `openPresenter` opens nothing. A host
gives it one with `PlayerOptions.presenter` (the presenter build's `presenter`), so a deck without one carries none of
its code. Each build has its gzip budget in `test/build.test.ts`: the player's 39,680 B, the presenter's 6,656 B.

The types the API names (`Deck`, `DeckSlide`, `SlideTemplate`, …) come from `@slidock/types`, generated from the
compiler's Rust types.

## Contents

| Export                                                 | What                                                                                                                   |
| ------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------- |
| `mount(host, deck, options?)`                          | plays a deck the way a deck page does, or, with `options.presenter`, mounts the presenter window on a `#presenter` URL |
| `createPlayer(host, deck, options?)`                   | a player in `host`, with every behaviour chosen by its options                                                         |
| `inlineDeck(document, options?)`                       | the deck of a slidock page: its `#sd-deck` JSON and `<template data-sd-slide>` elements                                |
| `htmlDeck(document, deck, slides, mediaUrl?)`          | a deck from its JSON and its slides' composed markup, as the web app holds them                                        |
| `parseDeck(json)`                                      | reads and checks deck JSON; throws on a format this player does not read                                               |
| `renderStatic(deck, index, step, target, fit?)`        | a still render of one slide at one step, scaled into `target`                                                          |
| `stepCount(slide)`                                     | the click groups the player plays on a slide                                                                           |
| `reduce`, `clickGroups`, `numbering`, `slideForNumber` | the position machine, which the presenter view previews the next click with                                            |
| `DECK_FORMAT`                                          | `'slidock-deck@1'`, the deck JSON version this player reads                                                            |
| `RT`                                                   | the runtime diagnostic codes (below)                                                                                   |
| `CHROME_CSS`                                           | the player's own styles (stage, counter, help overlay)                                                                 |
| `version`                                              | the player's version, recorded in each deck                                                                            |

Types: `Player`, `PlayerOptions`, `PlayerEvents`, `Position`, `Motion`, `Blank`, `MachineModel`, `DeckSource`,
`MediaResolver`, `PresenterSupport`, `PresenterOptions`, `PresenterView`, `JoinOptions`, `Sync`, `SyncRole`,
`SyncState`.

`@slidock/player/presenter` exports:

| Export                                           | What                                                                                                  |
| ------------------------------------------------ | ----------------------------------------------------------------------------------------------------- |
| `presenter`                                      | the presenter view for `PlayerOptions.presenter` and `mount`; a deck page's `SlidockPresenter`        |
| `mountPresenter(host, deck, options)`            | the presenter view alone: current slide, next-click preview, notes, timers, controls; it fills `host` |
| `createPresenterWindow(host)`                    | the presenter window's frame: the header (logo, title, deck title, theme toggle) over its main area   |
| `presenterSession(hash)`                         | the session a `#presenter[=<session>]` hash names, or `null`                                          |
| `nextClick(model, numbers, position)`            | what the next click shows, as the presenter view previews and labels it                               |
| `formatDuration(ms)`                             | a timer's text: `m:ss`, or `h:mm:ss` from an hour                                                     |
| `PRESENTER_CSS`                                  | the presenter window's styles                                                                         |
| `THEME_STORAGE_KEY`, `NEXT_THEME`, `THEME_LABEL` | the theme toggle's rule, the web app's (`@slidock/ui`)                                                |

## A deck page

A deck's `index.html` ends with `Slidock.mount(document.body, Slidock.inlineDeck(document), { presenter:
SlidockPresenter })`, or `Slidock.mount(document.body, Slidock.inlineDeck(document))` without a presenter view. The
same from the ES modules:

```ts
import { inlineDeck, mount } from '@slidock/player';
import { presenter } from '@slidock/player/presenter';

const player = mount(document.body, inlineDeck(document), { presenter });
player.on('ended', () => player.stop());
```

`mount` presents at once, reads keys on the whole document, takes clicks and swipes, shows the counter and help
overlay, and with a presenter view joins its session as the deck window (`createPlayer` with `mode: 'present'`,
`keyboard: 'document'`, `pointer: 'always'`, `chrome: true`, `sync: { role: 'deck' }`; the options passed override
these). With a presenter view, on a page whose hash is `#presenter` or `#presenter=<session>` it mounts the presenter
window instead (`presenter.mount`), removes the page's text view, and returns that view's player, whose `destroy` also
removes the window; without one, such a page shows the deck.

On `document.body` the player's stage is the body's first child, fixed to the window, and the page's text view
(`<article id="sd-text">`, `docs/format.md` §2) starts one window down and scrolls up over it; filling the window or
the screen covers the page, whose `<html>` then does not scroll until the show stops. The chrome then adds a Text
link beside the counter (shown with it, never while presenting) and a help row for the text view. With
`keyboard: 'document'` the keys are the deck's only while it presents or while at least half of the stage is in the
window and not under the text view, and never for a key aimed at the text view; Enter and Space aimed at a link or a
button inside the player (the Text link, a slide's link) keep their own action unless a slide number is being typed.

**Deep links.** `mount` reads `#slide-<n>`, `<n>` a slide's number among the shown slides: the deck starts there
(`startAt`) and the stage is scrolled into view (the fragment is held back until the page has loaded, so the
browser does not scroll to the text view's section of that name). While the hash is empty or of that form, each
shown slide the deck moves to is written to it with `history.replaceState` (no history entry, no `hashchange`; a
hidden slide writes nothing; a refused call, in some `file://` contexts, is ignored). A later `#slide-<n>` (a
heading link of the text view, a typed URL, back and forward) goes to that slide and brings the stage into view, as
does a click on the heading of the slide already in the hash. Any other hash is left alone, `#presenter=` keeping its
meaning. Search engines ignore fragments, so these create no URLs to crawl: every slide's text is already in the
page.

## Embedding a player

```ts
import { type Player, createPlayer, htmlDeck } from '@slidock/player';
import type { Deck, SlideTemplate } from '@slidock/types';

/** A compiled deck: its JSON, its slides' composed markup, and its media files by path (`media/<sha256>.<ext>`). */
export function play(host: HTMLElement, deck: Deck, slides: SlideTemplate[], media: Map<string, Blob>): Player {
  const urls = new Map<string, string>();
  const mediaUrl = (path: string): string => {
    let url = urls.get(path);
    if (url === undefined) {
      const blob = media.get(path);
      url = blob === undefined ? path : URL.createObjectURL(blob);
      urls.set(path, url);
    }
    return url;
  };
  const player = createPlayer(host, htmlDeck(document, deck, slides, mediaUrl), {
    mode: 'view',
    keyboard: 'host',
    pointer: 'off',
    chrome: false,
  });
  player.on('position', ({ position, motion }) => {
    console.log(`slide ${position.slide + 1}, step ${position.step} (${motion})`);
  });
  player.on('diagnostic', ({ code, message }) => console.warn(code, message));
  return player;
}
```

A `DeckSource` is the deck JSON plus `slide(id)`, which returns a fresh, inert copy of a slide's composed
`<main data-slide>` (or `null`), and an optional `mediaUrl` resolver that turns a deck media path into the URL the page
loads. Without one, media load by their relative paths, as in the zip. `inlineDeck(document, { mediaUrl })` reads a
slidock page; `htmlDeck` takes the slides as `SlideTemplate`s (`{id, html}`) or a `Map` from id to markup.

### `PlayerOptions`

| Option           | Default     | What                                                                                                                              |
| ---------------- | ----------- | --------------------------------------------------------------------------------------------------------------------------------- |
| `mode`           | `'view'`    | `view` shows every slide, fully built; `present` skips hidden slides and plays builds                                             |
| `builds`         | `false`     | play builds while viewing                                                                                                         |
| `transitions`    | `false`     | play transitions while viewing                                                                                                    |
| `reducedMotion`  | `'auto'`    | `auto` follows `prefers-reduced-motion`; `always` or `never` force it. Reduced, a transition is a 150 ms fade and a build appears |
| `startAt`        |             | `{ slide, step? }`: the first position                                                                                            |
| `loop`           | `false`     | presenting wraps from the last shown slide to the first                                                                           |
| `keyboard`       | `'host'`    | where keys are read: `document`, `host` (the player's element, focusable; the whole document while it presents) or `off`          |
| `pointer`        | `'present'` | click zones and swipes: while presenting, `always` or `off`. Links in slides work in every case                                   |
| `chrome`         | `true`      | the slide counter and the help overlay                                                                                            |
| `muted`          | `false`     | play every medium muted (the presenter view's player is)                                                                          |
| `sync`           |             | `{ role, session? }`: join a presenter-view session at once (with `presenter`)                                                    |
| `presenterUrl`   |             | the presenter window's URL for a session; the same page with `#presenter=<session>` by default                                    |
| `presenter`      |             | the presenter view (`@slidock/player/presenter`'s `presenter`): the P key, its help row, the sync and `openPresenter`             |
| `readyTimeoutMs` | `1500`      | the readiness gate's cap                                                                                                          |

### `Player`

| Member                                    | What                                                                                                                                                                             |
| ----------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `deck`                                    | the deck JSON                                                                                                                                                                    |
| `position`                                | `{ slide, step, presenting, blank }`: the slide's index in show order (hidden slides included), the click groups played, whether it presents, and `'black'`, `'white'` or `null` |
| `next()`, `previous()`                    | one click forward (animated) or back (a cut)                                                                                                                                     |
| `first()`, `last()`                       | the first or last navigable slide                                                                                                                                                |
| `goTo(slide, step?)`                      | cuts to a slide, by index or id, at a step (0 by default; a slide viewed without builds shows fully built)                                                                       |
| `present()`                               | starts presenting: full screen when the browser allows it, else filling the window; a `stop()` before the browser answers leaves the page as it was                              |
| `stop()`                                  | stops presenting (a presenter view's player ignores it: the deck window's show ends it)                                                                                          |
| `blank(kind)`                             | blanks the screen black or white, or `null` to show it again                                                                                                                     |
| `renderStatic(slide, step, target, fit?)` | a still render of a slide (index or id) at a step into `target`, scaled to `contain` it or to its `width`; returns its disposer                                                  |
| `openPresenter()`                         | opens the presenter view in a second window, presenting from here if not yet; `null` when the browser blocks it                                                                  |
| `follow(state, animate)`                  | follows another window's `{ slide, step, blank }`: animated when it is this player's next move and `animate` is set, else a cut                                                  |
| `on(type, listener)`                      | listens to an event (below); returns the function that stops listening                                                                                                           |
| `destroy()`                               | stops everything, leaves the sync session and removes the player's elements                                                                                                      |

State depends only on the position: a slide mounts from its inert template and fast-forwards every effect before
`step`. Moving forward animates, moving back cuts, and a press while a move plays finishes it without advancing. A
slide is shown, and a transition starts, once its faces have loaded and its images have decoded, or after
`readyTimeoutMs`.

Media play while presenting: a slide's `data-autoplay` media start as it shows, its media steps play, pause, stop or
toggle them, and leaving it stops and rewinds them. The one state a forward move carries from slide to slide is a clip
marked `data-across-slides` (format §3): `next()` onto a slide holding the same clip, also marked, goes on with it
from where it stood. The move shows the incoming slide once that clip stands at the outgoing one's frame (at most 1 s
later), so the clip never shows its first frame; a move dropped before then leaves it at its start. While presenting,
the next slide's marked clips that a marked clip of the shown slide can carry into load ahead; no other clip loads
before it plays.

### Events

| Event        | Payload                          | When                                                                                               |
| ------------ | -------------------------------- | -------------------------------------------------------------------------------------------------- |
| `position`   | `{ position, previous, motion }` | the position changed; `motion` is how the stage reaches it: `none`, `cut`, `build` or `transition` |
| `ready`      | `{ slide, timedOut }`            | a slide is on screen; `timedOut` when the readiness gate's cap ran out first                       |
| `ended`      | `{}`                             | `next` at the end of the show                                                                      |
| `diagnostic` | `{ code, message, slide }`       | a runtime diagnostic (below); `slide` is a slide id or `null`                                      |
| `mediaerror` | `{ slide, src }`                 | a medium failed to load                                                                            |
| `presenter`  | `{ window }`                     | the presenter window opened (`null` when the browser blocked it)                                   |

A listener that throws does not stop the others; its error is rethrown asynchronously. The first render waits a
microtask, so listeners added right after `createPlayer` hear its events.

### Runtime diagnostics (`RT`)

| Code     | Key                | Meaning                                                                                            |
| -------- | ------------------ | -------------------------------------------------------------------------------------------------- |
| `RT0001` | `readyTimeout`     | a slide showed before its fonts and images loaded                                                  |
| `RT0002` | `sanitized`        | the mount's allow-list walk removed something from a slide                                         |
| `RT0003` | `everyHidden`      | every slide is hidden                                                                              |
| `RT0004` | `presenterBlocked` | the browser blocked the presenter window                                                           |
| `RT0005` | `missingSlide`     | a slide has no markup                                                                              |
| `RT0006` | `effectFallback`   | a step or transition names an effect this player cannot play: it plays its fallback                |
| `RT0007` | `timingDefault`    | a timing value or field is out of the contract: it plays the declared default                      |
| `RT0008` | `stepDropped`      | a step's structure is out of the contract (target, kind, trigger, or a slide bound): it is dropped |
| `RT0009` | `moveFailed`       | a move failed; the player stays where it got to and goes on from there                             |

`docs/format.md` §5.4 lists how each timeline field fails.

## Thumbnails

```ts
import { type DeckSource, renderStatic, stepCount } from '@slidock/player';

/** Draws every slide fully built into its own box; returns the function that removes them. */
export function thumbnails(source: DeckSource, rail: HTMLElement): () => void {
  const disposers = source.deck.slides.map((slide, index) => {
    const box = rail.ownerDocument.createElement('div');
    rail.append(box);
    const dispose = renderStatic(source, index, stepCount(slide), box, 'width');
    return () => {
      dispose();
      box.remove();
    };
  });
  return () => disposers.forEach((dispose) => dispose());
}
```

`renderStatic` clamps the step to the slide's click groups and plays no motion. `fit: 'width'` scales the slide to the
target's width and sets the target's height; `'contain'` (the default) letterboxes it in the target.

## Input

With `keyboard` on, the keys follow PowerPoint's conventions:

| Key                                               | Action                                       |
| ------------------------------------------------- | -------------------------------------------- |
| → ↓ PageDown, and Space or Enter while presenting | next                                         |
| ← ↑ PageUp Backspace                              | previous                                     |
| Home, End                                         | first, last                                  |
| a number, then Enter                              | go to that slide number (shown slides only)  |
| B or `.`, W or `,`                                | blank black, blank white                     |
| F                                                 | full screen                                  |
| F5                                                | present                                      |
| P                                                 | open the presenter view (with one)           |
| ?                                                 | help                                         |
| Escape                                            | clear the typed number, else stop presenting |

Keys with Ctrl, Meta or Alt, keys a page already handled, and keys typed in a field are left alone. With `keyboard:
'document'`, the page's keys are the player's only while it fills the window or the screen, or while at least half of
its stage is in view (measured when the key comes) and the key does not come from inside `#sd-text`: otherwise Space,
the arrows and PageDown scroll the page.

With `pointer` on, a click in the left third of the slide goes back and anywhere else goes forward. Links come first,
the nearest one in the click's composed path:

- an `<a href>` in a slide to `#slide=<id>`, `#slide-<n>` or `#<n>` goes to that slide; an `http`, `https` or
  `mailto` link opens in a new window (`noopener,noreferrer`);
- a slide element's or run's `data-link` (`docs/format.md` §3): `url:<u>` opens `<u>` in a new window
  (`noopener,noreferrer`) when it is a link the deck may hold (`isSafeLink`); `slide:<id>` goes to that slide when it
  can be landed on; `action:next`, `action:prev`, `action:first` and `action:last` make that move. Any other value,
  an unsafe URL or a slide it cannot land on is ignored, and the click goes on to the zones.

## The presenter view

With a presenter view, `openPresenter()` (or the P key) opens the same page with `#presenter=<session>` in a second
window; on that page `mount` calls `presenter.mount`, which draws the presenter window: the frame
(`createPresenterWindow`) around the view (`mountPresenter`). Each window runs its own player; only the position is
synchronised, so each window follows the other and either can drive.

```ts
import { inlineDeck } from '@slidock/player';
import { presenter } from '@slidock/player/presenter';

const session = presenter.session(location.hash);
if (session !== null) {
  const view = presenter.mount(document.body, inlineDeck(document), { session });
  addEventListener('pagehide', () => view.destroy());
}
```

The frame is the web app's presenter window, which uses it too: the slidock header card (the logo, "slidock",
"Presenter view · <deck title>", the theme toggle and the brand strip) over the main area, on the app's tokens, dark
by default and light by `data-theme="light"` on `<html>` or the system preference. The toggle cycles system → light
→ dark as the app's does, remembered in `localStorage` (`slidock.theme`). `setStatus` puts a waiting line or a
warning banner before the view (the app's "Waiting for the deck window…" and "Nothing to present here"). The view
keeps the player's dark palette in either theme.

The view shows the current slide, a still preview of what the next click shows (the next build step or the next
slide), the speaker notes, the elapsed and per-slide timers (with a reset), "Slide n of N", previous and next, black
and white, and go to slide. Its player presents, reads keys on the whole document, is muted (the sound is the deck
window's) and has no chrome. `PresenterOptions` takes the `session` and `reducedMotion`, `readyTimeoutMs`, `loop`
and `startAt`.

`nextClick` is the preview's rule, usable on its own:

```ts
import { type Position, stepCount } from '@slidock/player';
import { nextClick } from '@slidock/player/presenter';
import type { Deck } from '@slidock/types';

export function nextLabel(deck: Deck, position: Position): string {
  const model = {
    slides: deck.slides.map((slide) => ({ hidden: slide.hidden, clickGroups: stepCount(slide) })),
    builds: true,
    loop: false,
  };
  return nextClick(
    model,
    deck.slides.map((slide) => slide.number),
    position,
  ).label;
}
```

### The presenter channel

The two windows exchange messages over two transports at once:

- `postMessage` between the deck window and the window it opened (`window.open` and `window.opener`), which works
  whatever the origin, `file://` included;
- a `BroadcastChannel` named `slidock:<deck id>`, for same-origin windows, which lets a reloaded window find the other
  again. `file://` origins are opaque in Chromium and per file in Firefox, so this one is best effort there.

Every message is an object:

```ts
export interface SyncMessage {
  ns: 'slidock-sync@1';
  deck: string; // the deck's `id`
  session: string; // the deck window's session
  role: 'deck' | 'presenter'; // the sender's
  sender: string; // the sending instance
  seq: number; // rising per sender
  kind: 'hello' | 'ping' | 'pong' | 'state' | 'bye';
  state: { slide: number; step: number; blank: 'black' | 'white' | null } | null;
}
```

- A window drops any message that is not of this shape, of another deck, from itself or from its own role, or whose
  `seq` is not above the last one it heard from that sender.
- A window that starts says `hello`; the window already running answers with `state`, which the new one follows.
  After a move made while presenting, a window sends `state` and the other follows it, animated when it is that
  window's next move.
- The presenter pings every 3 s and the deck window answers `pong`. A deck window that leaves (or reloads) says
  `bye` and starts a new session when it comes back. The presenter adopts another deck window's session only once its
  own deck window is gone (it said `bye`, or missed its pongs for 6.5 s) or when it never had one: on that window's
  `hello`, or on the `hello` a deck window sends back to a ping of another session.
- Positions are not secret, so messages go to any origin (`'*'`); the deck id and the session id gate them.
