import type { DeckFormat } from './generated/deck';

// TypeScript types generated from the Rust crates by ts-rs: the deck JSON (`slidock-deck@1`) and the IR values it
// carries, the bundle report and the worker protocol. The generated files are committed; `slidock-wasm`'s
// `typescript` test fails when they drift from Rust and rewrites them with `UPDATE_GOLDEN=1`.
export type * from './generated/bundle';
export type * from './generated/deck';
export type * from './generated/ir';
export type * from './generated/worker';

/** The deck format version this package describes. */
export const DECK_FORMAT: DeckFormat = 'slidock-deck@1';
