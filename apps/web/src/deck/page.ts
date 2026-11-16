// The deck page's own metadata as the download dialog asks for it: the public URL check and
// the notice for a page past what a crawler reads. The bundle checks the URL again and refuses what this lets by.
import type { BundleReport } from '@slidock/types';

/** What Googlebot reads of a file: its first 2 MiB, uncompressed. */
export const CRAWL_BYTES = 2 * 1024 * 1024;

/**
 * Why `url` cannot be the deck's public URL, or `null` when it can: an absolute `http(s)` URL with a host, no user
 * name or password, no fragment, and no whitespace, control character or backslash. Refused, never repaired.
 */
export function siteUrlProblem(url: string): string | null {
  // eslint-disable-next-line no-control-regex
  if (/[\s\u0000-\u001f\u007f-\u009f\\]/u.test(url)) {
    return 'The public URL must not hold spaces or control characters.';
  }
  const scheme = /^([a-z][a-z0-9+.-]*):\/\//i.exec(url)?.[1]?.toLowerCase();
  if (scheme !== 'http' && scheme !== 'https') {
    return 'The public URL must be an absolute http:// or https:// URL.';
  }
  if (url.includes('#')) {
    return 'The public URL must not hold a fragment (#…).';
  }
  const authority = url.slice(url.indexOf('://') + 3).split(/[/?]/)[0] ?? '';
  if (authority.includes('@')) {
    return 'The public URL must not hold a user name or password.';
  }
  const host = authority.startsWith('[') ? (authority.slice(1).split(']')[0] ?? '') : (authority.split(':')[0] ?? '');
  return host === '' ? 'The public URL has no host.' : null;
}

/** The notice for an `index.html` past the 2 MiB a crawler reads; `null` within it. */
export function sizeNotice(report: BundleReport, textView: boolean): string | null {
  const bytes = report.files.find((file) => file.path === 'index.html')?.bytes ?? 0;
  if (bytes <= CRAWL_BYTES) {
    return null;
  }
  const held = textView
    ? "that holds the page's text but not the player, so Google's preview shows no slide"
    : 'the page has no text version, so Google indexes none of its slides';
  return `index.html is ${(bytes / 1_048_576).toFixed(1)} MiB. Google reads the first 2 MiB: ${held}. Leave out single-file media or whole fonts to fit.`;
}
