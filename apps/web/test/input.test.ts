import { describe, expect, it } from 'vitest';

import { type DroppedItem, UnsupportedInput, captureDrop, checkFileName, fileFromDrop } from '../src/compile/input';

function expectUnsupported(run: () => unknown, pattern: RegExp) {
  try {
    run();
  } catch (error) {
    expect(error).toBeInstanceOf(UnsupportedInput);
    expect((error as Error).message).toMatch(pattern);
    return;
  }
  throw new Error('expected UnsupportedInput');
}

describe('checkFileName', () => {
  it('accepts the three presentation formats, in any case', () => {
    for (const name of ['a.pptx', 'b.KEY', 'c.odp', 'with.dots.in.name.Pptx']) {
      expect(() => checkFileName(name)).not.toThrow();
    }
  });

  it('refuses everything else with a reason', () => {
    expectUnsupported(() => checkFileName('deck.pdf'), /PDF isn't supported/);
    expectUnsupported(() => checkFileName('old.ppt'), /old binary PowerPoint/);
    expectUnsupported(() => checkFileName('sheet.ods'), /not a presentation/);
    expectUnsupported(() => checkFileName('notes.txt'), /not a presentation slidock can open/);
    expectUnsupported(() => checkFileName('noextension'), /not a presentation slidock can open/);
  });
});

/** A `FileSystemDirectoryEntry` over a nested object, listing in batches of two like a real reader. */
function folder(name: string, tree: Record<string, string | Record<string, unknown>>): FileSystemDirectoryEntry {
  const entry = {
    name,
    isDirectory: true,
    isFile: false,
    createReader() {
      const children: FileSystemEntry[] = Object.entries(tree).map(([child, value]) =>
        typeof value === 'string'
          ? ({
              name: child,
              isDirectory: false,
              isFile: true,
              file: (resolve: (file: File) => void) => resolve(new File([value], child)),
            } as unknown as FileSystemEntry)
          : folder(child, value as Record<string, string>),
      );
      let at = 0;
      return {
        readEntries(resolve: (entries: FileSystemEntry[]) => void) {
          const batch = children.slice(at, at + 2);
          at += 2;
          setTimeout(() => resolve(batch), 0);
        },
      };
    },
  };
  return entry as unknown as FileSystemDirectoryEntry;
}

/** The entry names of a zip, from its central directory. */
async function zipNames(file: File): Promise<string[]> {
  const bytes = new Uint8Array(await file.arrayBuffer());
  const view = new DataView(bytes.buffer);
  const end = bytes.length - 22;
  let at = view.getUint32(end + 16, true);
  const names: string[] = [];
  for (let i = 0; i < view.getUint16(end + 10, true); i++) {
    const length = view.getUint16(at + 28, true);
    names.push(new TextDecoder().decode(bytes.subarray(at + 46, at + 46 + length)));
    at += 46 + length;
  }
  return names;
}

describe('fileFromDrop', () => {
  it('passes a presentation file through and refuses others', async () => {
    const file = new File(['x'], 'Deck.pptx');
    await expect(fileFromDrop([{ kind: 'file', file }])).resolves.toBe(file);
    await expect(fileFromDrop([{ kind: 'file', file: new File(['x'], 'x.pdf') }])).rejects.toThrow(/PDF/);
    await expect(fileFromDrop([])).rejects.toThrow(/no file/);
    await expect(
      fileFromDrop([
        { kind: 'file', file },
        { kind: 'file', file },
      ]),
    ).rejects.toThrow(/one presentation at a time/);
  });

  it('packs a Keynote package folder as a zip whose entries keep the folder name, sorted', async () => {
    const entry = folder('Talk.key', {
      'preview.jpg': 'jpeg',
      Index: { 'Slide-1.iwa': 'one', 'Document.iwa': 'doc', 'Slide-2.iwa': 'two' },
      Data: { 'image.png': 'png' },
    });
    const file = await fileFromDrop([{ kind: 'folder', entry }]);
    expect(file.name).toBe('Talk.key');
    expect(await zipNames(file)).toEqual([
      'Talk.key/Data/image.png',
      'Talk.key/Index/Document.iwa',
      'Talk.key/Index/Slide-1.iwa',
      'Talk.key/Index/Slide-2.iwa',
      'Talk.key/preview.jpg',
    ]);
  });

  it('refuses other folders and empty packages', async () => {
    await expect(fileFromDrop([{ kind: 'folder', entry: folder('Photos', { 'a.png': 'x' }) }])).rejects.toThrow(
      /Keynote packages/,
    );
    await expect(fileFromDrop([{ kind: 'folder', entry: folder('Empty.key', {}) }])).rejects.toThrow(/empty/);
  });
});

describe('captureDrop', () => {
  it('reads folders as entries and files as files', () => {
    const file = new File(['x'], 'Deck.odp');
    const entry = folder('Talk.key', {});
    const items = [
      { kind: 'file', webkitGetAsEntry: () => ({ isDirectory: false }), getAsFile: () => file },
      { kind: 'file', webkitGetAsEntry: () => entry, getAsFile: () => null },
      { kind: 'string', webkitGetAsEntry: () => null, getAsFile: () => null },
    ];
    const captured = captureDrop({ items, files: [] } as unknown as DataTransfer);
    expect(captured).toEqual([
      { kind: 'file', file },
      { kind: 'folder', entry },
    ] satisfies DroppedItem[]);
  });

  it('falls back to files when items are unavailable', () => {
    const file = new File(['x'], 'Deck.key');
    expect(captureDrop({ items: [], files: [file] } as unknown as DataTransfer)).toEqual([{ kind: 'file', file }]);
  });
});
