// Which steps of the motion probes `check.mjs` compares with LibreOffice's engine.

/**
 * Whether a LibreOffice preset id names its random choice (`ooo-entrance-random`, `ooo-exit-random`): the engine
 * draws another preset for the step each time it loads the deck, so the clip it plays is not the deck's.
 * `ooo-entrance-random-bars` and its kin are ordinary presets.
 */
export function isRandomPreset(presetId) {
  return typeof presetId === 'string' && /^ooo-[a-z]+-random$/.test(presetId);
}

/** Whether slidock reads a step as the vocabulary's random build. */
export function isRandomBuild(step) {
  return step?.effect?.native?.effect === 'random';
}

/**
 * How the check treats a step: `skip` when both sides read it as a random choice, `mismatch` when only one does
 * (a reading to fix, never a skip), else `compare`.
 */
export function randomStep(ours, theirs) {
  const lo = isRandomPreset(theirs?.presetId);
  const sd = isRandomBuild(ours);
  if (lo && sd) {
    return 'skip';
  }
  return lo || sd ? 'mismatch' : 'compare';
}
