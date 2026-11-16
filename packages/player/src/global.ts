// The IIFE build's entry (`dist/player.iife.js`, which slidock-bundle inlines in decks): the package's exports as the
// page's `Slidock` global.
import * as player from './index';

(globalThis as { Slidock?: typeof player }).Slidock = player;
