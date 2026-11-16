// The single-file report's Content Security Policy, set as a <meta> right after its charset: the page runs its one
// inline module script (allowed by its sha256, so no other script can run), styles itself inline, shows images from
// its own folder or `data:` URIs, and loads or sends nothing else. `slidock-bench report` fills only the data slot, a
// `type="application/json"` script the browser never runs, so the hash holds for every filled page.
import { createHash } from 'node:crypto';

const CHARSET = '<meta charset="utf-8" />';
const MODULE = /<script type="module"[^>]*>([\s\S]*?)<\/script>/g;

export function reportCsp(script: string): string {
  const hash = createHash('sha256').update(script, 'utf8').digest('base64');
  return [
    "default-src 'none'",
    `script-src 'sha256-${hash}'`,
    "style-src 'unsafe-inline'",
    "img-src 'self' data:",
    "base-uri 'none'",
    "form-action 'none'",
  ].join('; ');
}

/** The page with its CSP. Throws unless it holds one charset meta and exactly one inline module script. */
export function withReportCsp(html: string): string {
  const scripts = [...html.matchAll(MODULE)];
  const [only] = scripts;
  if (scripts.length !== 1 || only?.[1] === undefined || only[1].trim() === '') {
    throw new Error(`the report page must hold exactly one inline module script (found ${scripts.length})`);
  }
  if (html.split(CHARSET).length !== 2) {
    throw new Error(`the report page must hold ${CHARSET} once`);
  }
  return html.replace(
    CHARSET,
    () => `${CHARSET}\n    <meta http-equiv="Content-Security-Policy" content="${reportCsp(only[1] ?? '')}" />`,
  );
}
