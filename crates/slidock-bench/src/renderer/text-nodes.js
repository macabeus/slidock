function textNodes(ownLabels) {
  // Every text node of the slide the fonts lane mounted, in tree order, with the computed font-family of the element
  // that holds it, and whether it is slidock's own label (inside an element `ownLabels` selects); the host walks the
  // same shadow tree over CDP and pairs the nodes by position and text. CDP's DOM leaves out text nodes of white space
  // only (Blink's `LengthWithStrippedWhiteSpace() == 0`), so this list does too.
  const layer = document.querySelector('[data-sd-layer]');
  const root = layer === null ? null : layer.shadowRoot;
  if (root === null) {
    throw new Error('no mounted slide');
  }
  const spaces = [0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x20, 0x1680, 0x2028, 0x205f, 0x3000];
  const blank = (text) => {
    for (const char of text) {
      const code = char.codePointAt(0);
      if (!spaces.includes(code) && !(code >= 0x2000 && code <= 0x200a)) {
        return false;
      }
    }
    return true;
  };
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  const nodes = [];
  for (let node = walker.nextNode(); node !== null; node = walker.nextNode()) {
    if (blank(node.data)) {
      continue;
    }
    const parent = node.parentElement;
    nodes.push({
      text: node.data,
      family: parent === null ? '' : getComputedStyle(parent).fontFamily,
      ownLabel: parent !== null && parent.closest(ownLabels) !== null,
    });
  }
  return nodes;
}
