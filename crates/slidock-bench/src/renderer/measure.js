// What the page shows, measured in the page after the screenshot: every rendered word with its box and the IR id of
// its element, every element's box, every table cell's box (by its row and its place among the row's cells), and every
// visible image box. A word counts as rendered only when it can be seen:
// its colour's alpha times its ancestors' opacity is above 0.05, every ancestor displays it, and its box survives every
// clipping ancestor and the slide. Boxes are in points, from the slide's top-left corner.
function measureSlide() {
  const PX_TO_PT = 0.75;
  const VISIBLE_ALPHA = 0.05;
  const main = document.querySelector('main');
  if (!main) return { words: [], elements: [], cells: [], images: [] };
  const slide = main.getBoundingClientRect();
  const toBox = (r) => ({
    x: (r.left - slide.left) * PX_TO_PT,
    y: (r.top - slide.top) * PX_TO_PT,
    w: (r.right - r.left) * PX_TO_PT,
    h: (r.bottom - r.top) * PX_TO_PT,
  });
  const intersect = (a, b) => ({
    left: Math.max(a.left, b.left),
    top: Math.max(a.top, b.top),
    right: Math.min(a.right, b.right),
    bottom: Math.min(a.bottom, b.bottom),
  });
  const empty = (r) => r.right - r.left <= 0 || r.bottom - r.top <= 0;
  const alphaOf = (color) => {
    const match = /rgba?\(([^)]+)\)/.exec(color);
    if (!match) return 1;
    const parts = match[1].split(/[\s,/]+/).filter(Boolean);
    return parts.length >= 4 ? Number(parts[3]) : 1;
  };
  const visibleRect = (element, rect) => {
    let clipped = intersect(rect, slide);
    let opacity = 1;
    for (let node = element; node && node !== document.documentElement; node = node.parentElement) {
      const style = getComputedStyle(node);
      if (style.display === 'none' || style.visibility === 'hidden') return null;
      opacity *= Number(style.opacity);
      if (node !== element && (style.overflowX !== 'visible' || style.overflowY !== 'visible')) {
        clipped = intersect(clipped, node.getBoundingClientRect());
      }
    }
    return empty(clipped) ? null : { rect: clipped, opacity };
  };
  const textBoxes = new Map([...main.querySelectorAll('[data-sd-text]')].map((box, index) => [box, index]));
  const raw = [];
  const walker = document.createTreeWalker(main, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const text = node.nodeValue ?? '';
    const parent = node.parentElement;
    if (!parent || !/\S/.test(text)) continue;
    const style = getComputedStyle(parent);
    const colorAlpha = alphaOf(style.color);
    const textBox = parent.closest('[data-sd-text]');
    for (const match of text.matchAll(/\S+/g)) {
      const range = document.createRange();
      range.setStart(node, match.index ?? 0);
      range.setEnd(node, (match.index ?? 0) + match[0].length);
      const rects = [...range.getClientRects()].filter((r) => r.width > 0 && r.height > 0);
      if (rects.length === 0) continue;
      const union = rects.reduce(
        (acc, r) => ({
          left: Math.min(acc.left, r.left),
          top: Math.min(acc.top, r.top),
          right: Math.max(acc.right, r.right),
          bottom: Math.max(acc.bottom, r.bottom),
        }),
        { left: Infinity, top: Infinity, right: -Infinity, bottom: -Infinity },
      );
      const visible = visibleRect(parent, union);
      if (!visible || colorAlpha * visible.opacity <= VISIBLE_ALPHA) continue;
      const start = match.index ?? 0;
      raw.push({
        joinBefore: start === 0,
        joinAfter: start + match[0].length === text.length,
        word: {
          ...toBox(union),
          text: match[0],
          el: parent.closest('[data-el]')?.getAttribute('data-el') ?? null,
          textBox: textBox === null ? null : (textBoxes.get(textBox) ?? null),
          family: style.fontFamily
            .split(',')[0]
            .trim()
            .replace(/^["']|["']$/g, ''),
          size: Number.parseFloat(style.fontSize) * PX_TO_PT,
          color: style.color,
        },
      });
    }
  }
  // Text in adjacent nodes of one text box with no whitespace between (a bullet label and its run, two runs of one
  // word) reads as one word when the glyphs touch, as it does in the oracle's text extraction.
  const words = [];
  let previous;
  for (const item of raw) {
    const last = words.at(-1);
    const gap = last ? item.word.x - (last.x + last.w) : Infinity;
    const sameLine =
      last !== undefined &&
      Math.abs(item.word.y + item.word.h / 2 - (last.y + last.h / 2)) <= Math.max(item.word.h, last.h) / 2;
    if (
      last &&
      last.el === item.word.el &&
      last.textBox === item.word.textBox &&
      previous?.joinAfter &&
      item.joinBefore &&
      sameLine &&
      gap >= -0.5 &&
      gap <= 0.15 * Math.max(last.size, item.word.size)
    ) {
      const right = Math.max(last.x + last.w, item.word.x + item.word.w);
      const bottom = Math.max(last.y + last.h, item.word.y + item.word.h);
      last.text += item.word.text;
      last.x = Math.min(last.x, item.word.x);
      last.y = Math.min(last.y, item.word.y);
      last.w = right - last.x;
      last.h = bottom - last.y;
    } else words.push({ ...item.word });
    previous = item;
  }
  const elements = [...main.querySelectorAll('[data-el]')].map((element) => ({
    ...toBox(element.getBoundingClientRect()),
    el: element.getAttribute('data-el') ?? '',
  }));
  const cells = [];
  for (const table of main.querySelectorAll('table')) {
    const el = table.closest('[data-el]')?.getAttribute('data-el') ?? '';
    [...table.rows].forEach((tr, row) => {
      [...tr.cells].forEach((td, index) => cells.push({ ...toBox(td.getBoundingClientRect()), el, row, index }));
    });
  }
  const images = [];
  for (const image of main.querySelectorAll('img, video')) {
    const owner = image.closest('[data-el]');
    let rect = image.getBoundingClientRect();
    if (owner) rect = intersect(rect, owner.getBoundingClientRect());
    const visible = visibleRect(image, rect);
    if (visible && visible.opacity > VISIBLE_ALPHA) {
      images.push({ ...toBox(visible.rect), el: owner?.getAttribute('data-el') ?? null });
    }
  }
  return { words, elements, cells, images };
}
