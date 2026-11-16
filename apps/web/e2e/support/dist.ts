// The base path a build of the app was made for, read from its index.html. The e2e suite serves a build for `/`, the
// Pages dry run one for `/slidock/`, and both write apps/web/dist: each refuses to run on the other's build.

/** The base path of a built index.html: the folder its entry script loads from, less `assets/`; null without one. */
export function builtBase(html: string): string | null {
  const entry = /<script\b[^>]*\btype="module"[^>]*\bsrc="([^"]*)"/.exec(html)?.[1];
  const base = entry === undefined ? undefined : /^(.*\/)assets\/[^/]+\.js$/.exec(entry)?.[1];
  return base ?? null;
}

/** Throws unless `html` (from `where`) is a build for `base`, saying how to get one. */
export function expectBuiltFor(html: string, base: string, where: string, rebuild: string): void {
  const found = builtBase(html);
  if (found !== base) {
    throw new Error(
      `${where} is ${found === null ? 'not a build of the app' : `a build for the base ${found}`}, not for ${base}: ${rebuild}`,
    );
  }
}
