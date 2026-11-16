import { describe, expect, it } from 'vitest';

import { siteUrlProblem, sizeNotice } from '../src/deck/page';

describe('the public URL', () => {
  it('takes an absolute http(s) URL with a host and refuses anything else, as the bundle does', () => {
    for (const good of [
      'https://example.com/talk/',
      'http://example.com',
      'HTTPS://example.com:8443/a?b=c',
      'https://[::1]:8080/x',
    ]) {
      expect(siteUrlProblem(good), good).toBeNull();
    }
    for (const bad of [
      'ftp://example.com/',
      '/talk/index.html',
      'talk/index.html',
      'example.com',
      'javascript://example.com/%0aalert(1)',
      'https://user:pass@example.com/',
      'https://example.com/#slide-2',
      'https:///path',
      'https://example.com/a b',
      ' https://example.com/',
      'https://example.com\\\\@evil.test/',
      '',
    ]) {
      expect(siteUrlProblem(bad), bad).not.toBeNull();
    }
  });
});

describe('the size notice', () => {
  const report = (bytes: number) => ({
    files: [{ path: 'index.html', bytes }],
    bytes,
    singleFile: false,
    fontsLeftOut: [],
  });

  it('says nothing within the 2 MiB a crawler reads, and the size past it', () => {
    expect(sizeNotice(report(2 * 1024 * 1024), true)).toBeNull();
    expect(sizeNotice(report(5_353_730), true)).toMatch(/^index.html is 5.1 MiB. Google reads the first 2 MiB/);
  });
});
