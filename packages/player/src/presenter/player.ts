// The player API the presenter view builds on. The presenter's builds do not bundle the player again: the IIFE reads
// these from the deck page's `Slidock` global and the ESM build imports them from `@slidock/player`
// (esbuild.config.mjs), so this list is all the presenter takes from the player at run time.
export { clickGroups, createPlayer, numbering, reduce, slideForNumber, stepCount } from '../index';
