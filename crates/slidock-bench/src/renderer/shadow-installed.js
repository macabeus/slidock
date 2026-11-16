async function shadowInstalled(shadowFont) {
  // Every family the mounted slide's text names that no face of the deck's `<style id="sd-fonts">` carries gets an
  // `@font-face` of the shadow face (`shadowFont`, base64 TrueType whose cmap maps nothing). A family an `@font-face`
  // rule names is never looked up among the installed fonts, and a face without a character's glyph passes it to the
  // next family, so the text falls through as it would in a renderer without that font installed: to a shipped
  // substitute, else the generic family. Generic keywords are left alone. Returns the families shadowed.
  const layer = document.querySelector('[data-sd-layer]');
  const root = layer === null ? null : layer.shadowRoot;
  if (root === null) {
    throw new Error('no mounted slide');
  }
  const key = (name) => name.trim().replace(/\s+/g, ' ').toLowerCase();
  const generics = new Set([
    'serif',
    'sans-serif',
    'monospace',
    'cursive',
    'fantasy',
    'system-ui',
    'ui-serif',
    'ui-sans-serif',
    'ui-monospace',
    'ui-rounded',
    'math',
    'emoji',
    'fangsong',
  ]);
  const shipped = new Set();
  for (const face of document.fonts) {
    shipped.add(key(face.family.replace(/^["']|["']$/g, '')));
  }
  // A computed font-family value: names separated by commas, quoted names possibly holding commas.
  const families = (value) => {
    const names = [];
    let current = '';
    let quote = null;
    let quoted = false;
    for (let i = 0; i < value.length; i += 1) {
      const char = value[i];
      if (quote !== null) {
        if (char === '\\' && i + 1 < value.length) {
          current += value[i + 1];
          i += 1;
        } else if (char === quote) {
          quote = null;
        } else {
          current += char;
        }
      } else if (char === '"' || char === "'") {
        quote = char;
        quoted = true;
      } else if (char === ',') {
        names.push({ name: current.trim(), quoted });
        current = '';
        quoted = false;
      } else {
        current += char;
      }
    }
    names.push({ name: current.trim(), quoted });
    return names.filter((entry) => entry.name !== '');
  };
  const shadowed = new Map();
  for (const element of [root.host, ...root.querySelectorAll('*')]) {
    for (const { name, quoted } of families(getComputedStyle(element).fontFamily)) {
      if ((!quoted && generics.has(name.toLowerCase())) || shipped.has(key(name))) {
        continue;
      }
      shadowed.set(key(name), name);
    }
  }
  const css = (text) => `"${text.replace(/\\/g, '\\\\').replace(/"/g, '\\"')}"`;
  const style = document.createElement('style');
  const src = `url(data:font/ttf;base64,${shadowFont}) format("truetype")`;
  style.textContent = Array.from(shadowed.values(), (name) => `@font-face{font-family:${css(name)};src:${src}}`).join(
    '',
  );
  document.head.append(style);
  const loaded = await Promise.all(
    Array.from(shadowed.values(), (name) => document.fonts.load(`16px ${css(name)}`).then((faces) => faces.length)),
  );
  if (loaded.some((count) => count === 0)) {
    throw new Error('the shadow face did not load');
  }
  await document.fonts.ready;
  return Array.from(shadowed.values()).sort();
}
