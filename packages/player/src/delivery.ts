// Delivery by word and character: the target's text nodes are wrapped in `<span data-sd-unit>`
// at mount, with `Intl.Segmenter` (words, or graphemes so emoji and combining marks stay whole); a word unit carries
// the spaces and punctuation after it. Wrapping preserves the text exactly, and `restore` puts the original text
// nodes back, so the DOM is byte-identical once the step is over or the slide unmounts.

export type Granularity = 'word' | 'grapheme';

export interface Split {
  /** The unit spans, in document order. */
  readonly units: readonly HTMLElement[];
  /** Puts every original text node back in place of its pieces. */
  restore(): void;
}

/** Text a step splits at most; longer targets build whole. */
export const SPLIT_CHARS = 20_000;

interface Piece {
  readonly text: string;
  readonly unit: boolean;
}

type SegmenterLike = { segment(text: string): Iterable<{ segment: string; isWordLike?: boolean }> };

function segmenter(granularity: Granularity, lang: string | undefined): SegmenterLike | null {
  const Segmenter = (Intl as { Segmenter?: new (locale?: string, options?: object) => SegmenterLike }).Segmenter;
  if (typeof Segmenter !== 'function') {
    return null;
  }
  try {
    return new Segmenter(lang, { granularity });
  } catch {
    return new Segmenter(undefined, { granularity });
  }
}

/** A text node's pieces: units and the text between them. */
export function pieces(text: string, granularity: Granularity, lang?: string): Piece[] {
  const seg = segmenter(granularity, lang);
  // Without `Intl.Segmenter` the text is one unit.
  const segments = seg === null ? [{ segment: text, isWordLike: true }] : Array.from(seg.segment(text));
  const out: { text: string; unit: boolean }[] = [];
  for (const { segment, isWordLike } of segments) {
    const blank = /^\s+$/u.test(segment);
    const isUnit = granularity === 'grapheme' ? !blank : isWordLike === true;
    const last = out[out.length - 1];
    if (!isUnit && granularity === 'word' && last?.unit === true) {
      // Spaces and punctuation travel with the word before them.
      last.text += segment;
    } else if (!isUnit && last?.unit === false) {
      last.text += segment;
    } else {
      out.push({ text: segment, unit: isUnit });
    }
  }
  return out;
}

function textNodes(root: Element): Text[] {
  const document = root.ownerDocument;
  const walker = document.createTreeWalker(root, 4 /* NodeFilter.SHOW_TEXT */);
  const out: Text[] = [];
  for (let node = walker.nextNode(); node !== null; node = walker.nextNode()) {
    const text = node as Text;
    if (text.data.trim() !== '' && text.parentElement?.closest('[data-sd-unit]') === null) {
      out.push(text);
    }
  }
  return out;
}

/** Splits the text of `roots` into units; `inlineBlock` lets transforms move them. */
export function splitText(
  roots: readonly Element[],
  granularity: Granularity,
  options: { inlineBlock?: boolean; lang?: string } = {},
): Split | null {
  const nodes = roots.flatMap(textNodes);
  const total = nodes.reduce((sum, node) => sum + node.data.length, 0);
  if (nodes.length === 0 || total > SPLIT_CHARS) {
    return null;
  }
  const replaced: { original: Text; parts: Node[] }[] = [];
  const units: HTMLElement[] = [];
  for (const original of nodes) {
    const document = original.ownerDocument;
    const parts: Node[] = [];
    for (const piece of pieces(original.data, granularity, options.lang)) {
      if (piece.unit) {
        const span = document.createElement('span');
        span.setAttribute('data-sd-unit', '');
        if (options.inlineBlock === true) {
          span.style.display = 'inline-block';
          span.style.whiteSpace = 'pre';
        }
        span.textContent = piece.text;
        units.push(span);
        parts.push(span);
      } else {
        parts.push(document.createTextNode(piece.text));
      }
    }
    const parent = original.parentNode;
    if (parent === null || parts.length === 0) {
      continue;
    }
    for (const part of parts) {
      parent.insertBefore(part, original);
    }
    parent.removeChild(original);
    replaced.push({ original, parts });
  }
  let restored = false;
  return {
    units,
    restore: () => {
      if (restored) {
        return;
      }
      restored = true;
      for (const { original, parts } of replaced) {
        const first = parts[0];
        const parent = first?.parentNode;
        if (first === undefined || parent === null || parent === undefined) {
          continue;
        }
        parent.insertBefore(original, first);
        for (const part of parts) {
          part.parentNode?.removeChild(part);
        }
      }
    },
  };
}
