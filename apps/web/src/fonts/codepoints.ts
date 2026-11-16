// The characters the deck draws, read from its compiled slides in this tab: they choose which Google Fonts chunks to
// fetch. The text itself never leaves the browser.
import type { SlideTemplate } from '@slidock/types';

/** Drawn by every deck: spaces, hyphens and the ellipsis a cut line may end with. */
const ALWAYS = [0x20, 0xa0, 0x2d, 0x2010, 0x2026];
/** Slide-number fields may be renumbered by the player. */
const DIGITS = Array.from({ length: 10 }, (_, digit) => 0x30 + digit);

/** Adds `text`'s code points, and their uppercase forms (text set in all caps or small caps draws those). */
function addText(points: Set<number>, text: string) {
  for (const char of text) {
    const point = char.codePointAt(0);
    if (point === undefined || point < 0x20) {
      continue;
    }
    points.add(point);
    const upper = char.toUpperCase();
    if (upper !== char) {
      for (const part of upper) {
        const value = part.codePointAt(0);
        if (value !== undefined) {
          points.add(value);
        }
      }
    }
  }
}

/** Every code point the slides' text draws, sorted. */
export function deckCodepoints(slides: readonly SlideTemplate[], parser: DOMParser = new DOMParser()): number[] {
  const points = new Set<number>([...ALWAYS, ...DIGITS]);
  for (const slide of slides) {
    // A parsed document is inert: its images and scripts never load or run.
    const document = parser.parseFromString(slide.html, 'text/html');
    addText(points, document.body.textContent);
  }
  return [...points].sort((a, b) => a - b);
}
