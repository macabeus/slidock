import { describe, expect, it } from 'vitest';

import { droppedFiles, folderName, packFolder } from '../src/folder';
import type { DroppedEntry } from '../src/folder';

/** A dropped file entry, as `webkitGetAsEntry` gives it: its `File` has no relative path. */
function fileEntry(fullPath: string, content: string): DroppedEntry {
  const file = new File([content], fullPath.split('/').pop() ?? '');
  return { isDirectory: false, fullPath, file: (success: (file: File) => void) => success(file) } as DroppedEntry;
}

/** A dropped folder entry whose reader answers in batches of `batch`, then an empty one. */
function folderEntry(fullPath: string, children: DroppedEntry[], batch = 2): DroppedEntry {
  return {
    isDirectory: true,
    fullPath,
    createReader() {
      let at = 0;
      return {
        readEntries(success: (entries: DroppedEntry[]) => void) {
          const entries = children.slice(at, at + batch);
          at += entries.length;
          success(entries);
        },
      };
    },
  } as DroppedEntry;
}

describe('droppedFiles', () => {
  it('lists the files of a dropped folder by their path from the drop, for packFolder', async () => {
    const drop = folderEntry('/Deck.key', [
      fileEntry('/Deck.key/Index.zip', 'z'),
      folderEntry('/Deck.key/Index', [
        fileEntry('/Deck.key/Index/Document.iwa', 'd'),
        fileEntry('/Deck.key/Index/Slide-1.iwa', 's1'),
        fileEntry('/Deck.key/Index/Slide-2.iwa', 's2'),
      ]),
      fileEntry('/Deck.key/Data/a.png', 'a'),
    ]);
    const files = await droppedFiles(drop);
    expect(files.map(({ path }) => path).sort()).toEqual([
      'Deck.key/Data/a.png',
      'Deck.key/Index.zip',
      'Deck.key/Index/Document.iwa',
      'Deck.key/Index/Slide-1.iwa',
      'Deck.key/Index/Slide-2.iwa',
    ]);
    expect(await files.find(({ path }) => path.endsWith('Slide-2.iwa'))?.file.text()).toBe('s2');
    expect(folderName(files)).toBe('Deck.key');
    expect(packFolder('Deck.key', files).size).toBeGreaterThan(0);
  });

  it('turns a failed read into a rejection', async () => {
    const broken = {
      isDirectory: true,
      fullPath: '/Deck.key',
      createReader: () => ({
        readEntries: (_: unknown, failure?: (error: DOMException) => void) => failure?.(new DOMException('gone')),
      }),
    } as DroppedEntry;
    await expect(droppedFiles(broken)).rejects.toThrow('gone');
  });
});
