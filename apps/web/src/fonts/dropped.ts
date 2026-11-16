// Font files the user adds: dropped anywhere on the page or picked in the fonts panel.
// The worker reads each one (TrueType, OpenType, a collection, WOFF or WOFF2) and refuses what is no font.

/** The extensions the picker offers and a drop is recognized by. */
export const FONT_EXTENSIONS = ['.ttf', '.otf', '.ttc', '.otc', '.woff', '.woff2'] as const;
export const FONT_ACCEPT = FONT_EXTENSIONS.join(',');
/** A file larger than this is not read (the worker's own limit is lower). */
export const MAX_FONT_FILE_BYTES = 64 << 20;
/** At most this many of the user's files are taken per deck, and this many bytes in all. */
export const MAX_USER_FILES = 64;
export const MAX_USER_BYTES = 256 << 20;

export function isFontFile(file: { name: string }): boolean {
  const name = file.name.toLowerCase();
  return FONT_EXTENSIONS.some((extension) => name.endsWith(extension));
}

/** The files of a drop when every one is a font file (a drop that holds anything else opens a deck). */
export function droppedFonts(data: DataTransfer): File[] | null {
  const files = Array.from(data.files);
  if (files.length === 0 || !files.every(isFontFile)) {
    return null;
  }
  return files;
}

/** One added file and what became of it. */
export type DroppedFile =
  | { name: string; state: 'reading' }
  | { name: string; state: 'added' }
  | { name: string; state: 'rejected'; reason: string };
