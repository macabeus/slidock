// The IIFE build's entry (`dist/presenter.iife.js`, which slidock-bundle inlines after the player in decks that ship a
// presenter view): the presenter view as the page's `SlidockPresenter` global, for `Slidock.mount`.
import { presenter } from './support';

(globalThis as { SlidockPresenter?: typeof presenter }).SlidockPresenter = presenter;
