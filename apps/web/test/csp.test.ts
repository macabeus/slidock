// @vitest-environment node
import { describe, expect, it } from 'vitest';

import { CSP } from '../csp';
import { cspPlugin } from '../vite.config';

function directives(policy: string): Map<string, string[]> {
  return new Map(
    policy.split(';').map((part) => {
      const [name = '', ...values] = part.trim().split(/\s+/);
      return [name, values];
    }),
  );
}

describe('CSP', () => {
  const policy = directives(CSP);

  it('runs only the app and its wasm, with blob: media and workers and Google Fonts files', () => {
    expect(policy.get('default-src')).toEqual(["'self'"]);
    expect(policy.get('script-src')).toEqual(["'self'", "'wasm-unsafe-eval'"]);
    expect(policy.get('worker-src')).toContain('blob:');
    expect(policy.get('img-src')).toContain('blob:');
    expect(policy.get('media-src')).toContain('blob:');
    expect(policy.get('font-src')).toEqual(expect.arrayContaining(['blob:', 'data:', 'https://fonts.gstatic.com']));
    expect(policy.get('object-src')).toEqual(["'none'"]);
  });

  it('connects nowhere but itself and Google Fonts', () => {
    expect(policy.get('connect-src')).toEqual(["'self'", 'https://fonts.googleapis.com', 'https://fonts.gstatic.com']);
    expect(CSP).not.toMatch(/unsafe-eval'(?<!wasm-unsafe-eval')/);
    expect(policy.get('script-src')).not.toContain("'unsafe-inline'");
  });

  it('the build puts it right after the charset', () => {
    const plugin = cspPlugin();
    expect(plugin.apply).toBe('build');
    const transform = plugin.transformIndexHtml as (html: string) => string;
    const html = transform('<head>\n    <meta charset="utf-8" />\n    <title>x</title>');
    expect(html).toContain(
      `<meta charset="utf-8" />\n    <meta http-equiv="Content-Security-Policy" content="${CSP}" />`,
    );
    expect(() => transform('<head></head>')).toThrow(/charset/);
  });
});
