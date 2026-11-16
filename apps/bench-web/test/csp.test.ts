import { createHash } from 'node:crypto';
// @vitest-environment node
import { describe, expect, it } from 'vitest';

import { reportCsp, withReportCsp } from '../scripts/report-csp';

const page = (scripts: string) =>
  `<!doctype html><html><head>\n    <meta charset="utf-8" />\n    <title>t</title>${scripts}</head><body></body></html>`;

describe("the report's CSP", () => {
  it("allows the page's one inline module script by its sha256, after the charset", () => {
    const script = 'console.log("<\\/script>")';
    const html = withReportCsp(page(`<script type="module" crossorigin>${script}</script>`));
    const hash = createHash('sha256').update(script).digest('base64');
    expect(html).toContain(`<meta charset="utf-8" />\n    <meta http-equiv="Content-Security-Policy" content="`);
    expect(html).toContain(`script-src 'sha256-${hash}'`);
    expect(reportCsp(script)).toMatch(/^default-src 'none'; /);
    expect(reportCsp(script)).toContain("img-src 'self' data:");
  });

  it('refuses a page without exactly one inline module script', () => {
    expect(() => withReportCsp(page(''))).toThrow('found 0');
    const one = '<script type="module">a()</script>';
    expect(() => withReportCsp(page(one + one))).toThrow('found 2');
    expect(() => withReportCsp(page('<script type="module"></script>'))).toThrow('found 1');
  });
});
