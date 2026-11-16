// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest';

import { SPLIT_CHARS, pieces, splitText } from '../src/delivery';

function box(html: string): HTMLElement {
  const div = document.createElement('div');
  div.innerHTML = html;
  document.body.append(div);
  return div;
}

const units = (split: ReturnType<typeof splitText>) => split?.units.map((unit) => unit.textContent) ?? [];

describe('delivery', () => {
  it('splits words, each carrying the spaces and punctuation after it', () => {
    const div = box('<p data-p="0">Hello, big world!</p>');
    const split = splitText([div], 'word');
    expect(units(split)).toEqual(['Hello, ', 'big ', 'world!']);
    expect(div.textContent).toBe('Hello, big world!');
  });

  it('keeps graphemes whole: emoji ZWJ sequences, flags and combining marks', () => {
    const family = '👩‍👩‍👧';
    const text = `é${family}🇧🇷a`;
    const div = box(`<p>${text}</p>`);
    const split = splitText([div], 'grapheme');
    expect(units(split)).toEqual(['é', family, '🇧🇷', 'a']);
    expect(div.textContent).toBe(text);
  });

  it('leaves spaces between characters as text, not units', () => {
    expect(pieces('a b', 'grapheme')).toEqual([
      { text: 'a', unit: true },
      { text: ' ', unit: false },
      { text: 'b', unit: true },
    ]);
  });

  it('restores the original text nodes, byte for byte, and only once', () => {
    const html =
      '<p data-p="0"><span data-sd-bullet="">1.</span><span style="color:red">Read the </span><a href="#x"><span>spec</span></a>\t<b>now</b></p><p data-p="1">Second <i>line</i></p>';
    const div = box(html);
    const serialized = div.innerHTML;
    const before = Array.from(div.querySelectorAll('span, b, i')).map((el) => el.firstChild);
    for (const granularity of ['word', 'grapheme'] as const) {
      const split = splitText([div], granularity, { inlineBlock: true });
      expect(div.innerHTML).not.toBe(serialized);
      expect(div.querySelectorAll('[data-sd-unit]').length).toBe(split?.units.length);
      expect((split?.units[0] as HTMLElement).style.display).toBe('inline-block');
      split?.restore();
      expect(div.innerHTML).toBe(serialized);
      // The original text node objects are back, not copies.
      const after = Array.from(div.querySelectorAll('span, b, i')).map((el) => el.firstChild);
      expect(after.every((node, i) => node === before[i])).toBe(true);
      split?.restore();
      expect(div.innerHTML).toBe(serialized);
    }
  });

  it('splits only the given roots (a paragraph range)', () => {
    const div = box('<p data-p="0">One two</p><p data-p="1">Three four</p>');
    const second = div.querySelector('[data-p="1"]') as Element;
    const split = splitText([second], 'word');
    expect(units(split)).toEqual(['Three ', 'four']);
    expect(div.querySelector('[data-p="0"]')?.innerHTML).toBe('One two');
    split?.restore();
  });

  it('does not split text that is already split, or more than the bound', () => {
    const div = box('<p>alpha beta</p>');
    const first = splitText([div], 'word');
    expect(splitText([div], 'word')).toBeNull();
    first?.restore();
    const long = box(`<p>${'x'.repeat(SPLIT_CHARS + 1)}</p>`);
    expect(splitText([long], 'grapheme')).toBeNull();
    expect(splitText([box('<p>   </p>')], 'word')).toBeNull();
  });

  it('builds the text as one unit where Intl.Segmenter is missing', () => {
    const intl = Intl as { Segmenter?: unknown };
    const saved = intl.Segmenter;
    intl.Segmenter = undefined;
    try {
      expect(pieces('ab cd', 'word')).toEqual([{ text: 'ab cd', unit: true }]);
      expect(pieces('ab', 'grapheme')).toEqual([{ text: 'ab', unit: true }]);
    } finally {
      intl.Segmenter = saved;
    }
  });
});
