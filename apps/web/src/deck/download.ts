/** Saves `blob` as `name` through a temporary `<a download>`. */
export function saveBlob(blob: Blob, name: string) {
  const url = URL.createObjectURL(blob);
  const link = document.createElement('a');
  link.href = url;
  link.download = name;
  link.rel = 'noopener';
  link.style.display = 'none';
  document.body.append(link);
  link.click();
  link.remove();
  // The download has started by the next task; the URL is then no longer needed.
  setTimeout(() => URL.revokeObjectURL(url), 0);
}

/** A file name for the downloaded zip: the deck title without characters file systems refuse. */
export function zipName(title: string): string {
  const printable = Array.from(title, (char) => (char.charCodeAt(0) < 0x20 ? ' ' : char)).join('');
  const safe = printable
    .replace(/[\\/:*?"<>|]+/g, ' ')
    .replace(/\s+/g, ' ')
    .trim()
    .slice(0, 120);
  return `${safe || 'deck'}.slidock.zip`;
}

const UNITS = ['B', 'KB', 'MB', 'GB'] as const;

/** `1.2 MB`-style sizes, in powers of 1000. */
export function formatBytes(bytes: number): string {
  let value = bytes;
  let unit = 0;
  while (value >= 1000 && unit < UNITS.length - 1) {
    value /= 1000;
    unit += 1;
  }
  return `${unit === 0 ? value : value.toFixed(value < 10 ? 1 : 0)} ${UNITS[unit]}`;
}
