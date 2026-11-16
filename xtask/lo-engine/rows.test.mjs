// `node --test xtask/lo-engine/rows.test.mjs`, run by `cargo test -p xtask`.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { isRandomPreset, randomStep } from './rows.mjs';

const random = (id) => ({ effect: { id, native: { effect: 'random', id } } });
const box = { effect: { id: 'odp:ooo-exit-box/in', native: { effect: 'iris', id: 'odp:ooo-exit-box/in' } } };

test('only the random choice is a random preset', () => {
  assert.equal(isRandomPreset('ooo-entrance-random'), true);
  assert.equal(isRandomPreset('ooo-exit-random'), true);
  assert.equal(isRandomPreset('ooo-entrance-random-bars'), false);
  assert.equal(isRandomPreset('ooo-exit-random-bars'), false);
  assert.equal(isRandomPreset('ooo-exit-box'), false);
  assert.equal(isRandomPreset(null), false);
  assert.equal(isRandomPreset(undefined), false);
});

test('a step both sides read as random is skipped', () => {
  assert.equal(randomStep(random('odp:ooo-exit-random'), { presetId: 'ooo-exit-random' }), 'skip');
  assert.equal(randomStep(random('pptx:entr.24'), { presetId: 'ooo-entrance-random' }), 'skip');
});

test('a step one side reads as random is a mismatch, never a skip', () => {
  assert.equal(randomStep(box, { presetId: 'ooo-exit-random' }), 'mismatch');
  assert.equal(randomStep(random('odp:ooo-exit-random'), { presetId: 'ooo-exit-box' }), 'mismatch');
  assert.equal(randomStep(undefined, { presetId: 'ooo-exit-random' }), 'mismatch');
});

test('any other step is compared', () => {
  assert.equal(randomStep(box, { presetId: 'ooo-exit-box' }), 'compare');
  assert.equal(randomStep(box, { presetId: 'ooo-entrance-random-bars' }), 'compare');
  assert.equal(randomStep(undefined, {}), 'compare');
});
