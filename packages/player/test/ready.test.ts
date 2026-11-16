// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { READY_TIMEOUT_MS, fontQuery, ready } from '../src/ready';

/** A promise and its resolve/reject, to finish a load when the test says so. */
function deferred() {
  let resolve!: () => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<void>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function root(images: number): { root: HTMLElement; decodes: ReturnType<typeof deferred>[] } {
  const element = document.createElement('div');
  const decodes: ReturnType<typeof deferred>[] = [];
  for (let i = 0; i < images; i++) {
    const image = document.createElement('img');
    const load = deferred();
    decodes.push(load);
    image.decode = () => load.promise;
    element.append(image);
  }
  return { root: element, decodes };
}

const face = { family: 'Avenir Next', weight: 500, style: 'italic' as const };

describe('readiness gate', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it('asks document.fonts for each face', () => {
    expect(fontQuery(face)).toBe('italic 500 16px "Avenir Next"');
    expect(fontQuery({ family: 'A "B"', weight: 400, style: 'normal' })).toBe('normal 400 16px "A \\"B\\""');
  });

  it('opens once the fonts load and the images decode', async () => {
    const font = deferred();
    const fonts = { load: vi.fn(() => font.promise.then(() => [])) };
    const { root: element, decodes } = root(2);
    const result = vi.fn();
    void ready(element, [face], { fonts }).then(result);
    expect(fonts.load).toHaveBeenCalledWith('italic 500 16px "Avenir Next"');
    decodes[0]?.resolve();
    font.resolve();
    await vi.advanceTimersByTimeAsync(100);
    expect(result).not.toHaveBeenCalled();
    decodes[1]?.resolve();
    await vi.advanceTimersByTimeAsync(0);
    expect(result).toHaveBeenCalledWith({ timedOut: false });
  });

  it('opens at 1.5 s when a load hangs', async () => {
    const fonts = { load: () => new Promise<FontFace[]>(() => undefined) };
    const result = vi.fn();
    void ready(document.createElement('div'), [face], { fonts }).then(result);
    await vi.advanceTimersByTimeAsync(READY_TIMEOUT_MS - 1);
    expect(result).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(result).toHaveBeenCalledWith({ timedOut: true });
    expect(READY_TIMEOUT_MS).toBe(1500);
  });

  it('counts a failed load as done', async () => {
    const fonts = { load: () => Promise.reject(new Error('no such face')) };
    const { root: element, decodes } = root(1);
    decodes[0]?.reject(new Error('broken image'));
    const result = vi.fn();
    void ready(element, [face], { fonts }).then(result);
    await vi.advanceTimersByTimeAsync(0);
    expect(result).toHaveBeenCalledWith({ timedOut: false });
  });

  it('a throwing font API counts as loaded', async () => {
    const fonts = {
      load: (): Promise<FontFace[]> => {
        throw new SyntaxError('bad font shorthand');
      },
    };
    const result = vi.fn();
    void ready(document.createElement('div'), [face], { fonts }).then(result);
    await vi.advanceTimersByTimeAsync(0);
    expect(result).toHaveBeenCalledWith({ timedOut: false });
  });

  it('opens at once when aborted', async () => {
    const { root: element } = root(1);
    const abort = new AbortController();
    const result = vi.fn();
    void ready(element, [], { signal: abort.signal }).then(result);
    await vi.advanceTimersByTimeAsync(10);
    expect(result).not.toHaveBeenCalled();
    abort.abort();
    await vi.advanceTimersByTimeAsync(0);
    expect(result).toHaveBeenCalledWith({ timedOut: false });
  });

  it('opens at once with nothing to wait for', async () => {
    await expect(ready(document.createElement('div'), [face])).resolves.toEqual({ timedOut: false });
  });

  it('takes a custom cap', async () => {
    const { root: element } = root(1);
    const result = vi.fn();
    void ready(element, [], { timeoutMs: 200 }).then(result);
    await vi.advanceTimersByTimeAsync(200);
    expect(result).toHaveBeenCalledWith({ timedOut: true });
  });
});
