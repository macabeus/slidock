// Seeded randomness: random bars, dissolve cells, random stagger orders and the `random` effects pick the same way on
// every replay, so a slide's state and its motion depend only on the deck.

/** A 32-bit FNV-1a hash of a string. */
export function hashSeed(text: string): number {
  let hash = 0x811c9dc5;
  for (let i = 0; i < text.length; i++) {
    hash ^= text.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193);
  }
  return hash >>> 0;
}

/** A mulberry32 generator: numbers in [0, 1) from a 32-bit seed. */
export function generator(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** `0..n-1` shuffled from `seed` (Fisher–Yates). */
export function permutation(n: number, seed: number): number[] {
  const order = Array.from({ length: n }, (_, i) => i);
  const next = generator(seed);
  for (let i = n - 1; i > 0; i--) {
    const j = Math.floor(next() * (i + 1));
    const swap = order[i] as number;
    order[i] = order[j] as number;
    order[j] = swap;
  }
  return order;
}
