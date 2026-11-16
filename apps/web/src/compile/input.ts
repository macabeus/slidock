import { type FolderFile, packFolder } from '@slidock/wasm/folder';

/** The presentation formats the compiler reads, by file extension. */
export const ACCEPTED_EXTENSIONS = ['.pptx', '.key', '.odp'] as const;
export const ACCEPT = ACCEPTED_EXTENSIONS.join(',');

/** Why an input is not opened. */
export class UnsupportedInput extends Error {}

function extension(name: string): string {
  const dot = name.lastIndexOf('.');
  return dot < 0 ? '' : name.slice(dot).toLowerCase();
}

/** Throws `UnsupportedInput` with a reason a person can act on, unless the compiler reads this kind of file. */
export function checkFileName(name: string) {
  const ext = extension(name);
  if ((ACCEPTED_EXTENSIONS as readonly string[]).includes(ext)) {
    return;
  }
  if (ext === '.pdf') {
    throw new UnsupportedInput("PDF isn't supported yet. Open the original .pptx, .key or .odp file.");
  }
  if (ext === '.ppt' || ext === '.pps' || ext === '.pot') {
    throw new UnsupportedInput(
      `${ext} is the old binary PowerPoint format. Save the presentation as .pptx and open that.`,
    );
  }
  if (ext === '.odg' || ext === '.odt' || ext === '.ods') {
    throw new UnsupportedInput(`${ext} is not a presentation. slidock opens .pptx, .key and .odp files.`);
  }
  throw new UnsupportedInput(
    `“${name}” is not a presentation slidock can open. Use a .pptx, .key or .odp file, or a Keynote package folder.`,
  );
}

/**
 * What a drop holds, captured while the drop event runs (a `DataTransfer` is emptied after it): a file, or a folder
 * as a `FileSystemDirectoryEntry`.
 */
export type DroppedItem = { kind: 'file'; file: File } | { kind: 'folder'; entry: FileSystemDirectoryEntry };

/** Reads a drop's items synchronously. Call it inside the drop handler. */
export function captureDrop(data: DataTransfer): DroppedItem[] {
  const items: DroppedItem[] = [];
  for (const item of Array.from(data.items ?? [])) {
    if (item.kind !== 'file') {
      continue;
    }
    const entry = typeof item.webkitGetAsEntry === 'function' ? item.webkitGetAsEntry() : null;
    if (entry?.isDirectory) {
      items.push({ kind: 'folder', entry: entry as FileSystemDirectoryEntry });
      continue;
    }
    const file = item.getAsFile();
    if (file) {
      items.push({ kind: 'file', file });
    }
  }
  if (items.length === 0) {
    for (const file of Array.from(data.files ?? [])) {
      items.push({ kind: 'file', file });
    }
  }
  return items;
}

function readBatch(reader: FileSystemDirectoryReader): Promise<FileSystemEntry[]> {
  return new Promise((resolve, reject) => reader.readEntries(resolve, reject));
}

function fileOf(entry: FileSystemFileEntry): Promise<File> {
  return new Promise((resolve, reject) => entry.file(resolve, reject));
}

/** Every file under `folder`, with its path from the folder's parent (`Deck.key/Index/Document.iwa`). */
export async function folderFiles(folder: FileSystemDirectoryEntry): Promise<FolderFile[]> {
  const files: FolderFile[] = [];
  const walk = async (directory: FileSystemDirectoryEntry, prefix: string) => {
    const reader = directory.createReader();
    // readEntries returns the listing in batches and an empty batch at the end.
    for (let batch = await readBatch(reader); batch.length > 0; batch = await readBatch(reader)) {
      for (const entry of batch) {
        const path = `${prefix}/${entry.name}`;
        if (entry.isDirectory) {
          await walk(entry as FileSystemDirectoryEntry, path);
        } else if (entry.isFile) {
          files.push({ path, file: await fileOf(entry as FileSystemFileEntry) });
        }
      }
    }
  };
  await walk(folder, folder.name);
  return files;
}

/** Turns a drop into the one `File` to open, packing a Keynote package folder as a stored zip. */
export async function fileFromDrop(items: readonly DroppedItem[]): Promise<File> {
  const [item, ...rest] = items;
  if (!item) {
    throw new UnsupportedInput('The drop held no file.');
  }
  if (rest.length > 0) {
    throw new UnsupportedInput('Drop one presentation at a time.');
  }
  if (item.kind === 'file') {
    checkFileName(item.file.name);
    return item.file;
  }
  const name = item.entry.name;
  if (extension(name) !== '.key') {
    throw new UnsupportedInput(
      `“${name}” is a folder. The only folders slidock opens are Keynote packages (folders named “….key”).`,
    );
  }
  const files = await folderFiles(item.entry);
  if (files.length === 0) {
    throw new UnsupportedInput(`The Keynote package “${name}” is empty.`);
  }
  try {
    return packFolder(name, files);
  } catch (error) {
    if (error instanceof TypeError) {
      throw new UnsupportedInput(`The Keynote package “${name}” cannot be opened: ${error.message}.`);
    }
    throw error;
  }
}
