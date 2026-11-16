// A Keynote package folder (older Keynote saves a folder, and a folder drop gives its files) as one `File`: a stored
// zip whose entries keep their paths (`Deck.key/Index/Document.iwa`), which the Keynote frontend reads like a `.key`.
// The zip is a `Blob` of the headers and the files themselves, so no file is read or copied to build it. Its CRC-32
// fields are 0: slidock's zip reader reads stored entries by range and checks no checksum.

/** One file of a folder, by its path relative to the folder's parent (`File.webkitRelativePath`). */
export interface FolderFile {
  path: string;
  file: Blob;
}

const U32 = 0xffffffff;
const U16 = 0xffff;
/** UTF-8 names (general purpose flag bit 11). */
const UTF8 = 0x0800;
/** 1980-01-01, the earliest DOS date: entries carry no time. */
const DOS_DATE = (1 << 5) | 1;

class Bytes {
  readonly #view: DataView<ArrayBuffer>;
  #at = 0;

  constructor(length: number) {
    this.#view = new DataView(new ArrayBuffer(length));
  }

  u16(value: number): this {
    this.#view.setUint16(this.#at, value, true);
    this.#at += 2;
    return this;
  }

  u32(value: number): this {
    this.#view.setUint32(this.#at, value, true);
    this.#at += 4;
    return this;
  }

  u64(value: number): this {
    this.#view.setBigUint64(this.#at, BigInt(value), true);
    this.#at += 8;
    return this;
  }

  bytes(value: Uint8Array): this {
    new Uint8Array(this.#view.buffer, this.#at, value.length).set(value);
    this.#at += value.length;
    return this;
  }

  get buffer(): ArrayBuffer {
    if (this.#at !== this.#view.byteLength) {
      throw new Error(`wrote ${this.#at} of ${this.#view.byteLength} bytes`);
    }
    return this.#view.buffer;
  }
}

/** A path relative to the folder's parent: `/`-separated plain names, no `.`, `..` or empty segment. */
function checkPath(path: string): void {
  const segments = path.split('/');
  if (segments.some((segment) => segment === '' || segment === '.' || segment === '..' || segment.includes('\\'))) {
    throw new TypeError(`${JSON.stringify(path)} is not a relative path`);
  }
}

/** Where the zip writes zip64 fields: past 32-bit sizes and offsets, or always (for tests). */
export interface PackOptions {
  zip64?: 'auto' | 'always';
}

/**
 * Packs `files` into a stored zip named `name`, in path order (by UTF-16 code unit). The paths are kept as they
 * are; a Keynote package's files sit one folder down (`Deck.key/Index/Document.iwa`).
 */
export function packFolder(name: string, files: readonly FolderFile[], options: PackOptions = {}): File {
  const always = options.zip64 === 'always';
  const sorted = [...files].sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0));
  const encoder = new TextEncoder();
  const parts: BlobPart[] = [];
  const directory: ArrayBuffer[] = [];
  let offset = 0;
  let previous: string | undefined;
  for (const { path, file } of sorted) {
    checkPath(path);
    if (path === previous) {
      throw new TypeError(`${JSON.stringify(path)} appears twice`);
    }
    previous = path;
    const nameBytes = encoder.encode(path);
    if (nameBytes.length > U16) {
      throw new TypeError(`${JSON.stringify(path)} is too long`);
    }
    const size = file.size;
    const large = always || size >= U32;
    const local = new Bytes(30 + nameBytes.length + (large ? 20 : 0))
      .u32(0x04034b50)
      .u16(large ? 45 : 20)
      .u16(UTF8)
      .u16(0)
      .u16(0)
      .u16(DOS_DATE)
      .u32(0)
      .u32(large ? U32 : size)
      .u32(large ? U32 : size)
      .u16(nameBytes.length)
      .u16(large ? 20 : 0)
      .bytes(nameBytes);
    if (large) {
      local.u16(0x0001).u16(16).u64(size).u64(size);
    }
    const header = local.buffer;
    const far = always || offset >= U32;
    const extra = (large ? 16 : 0) + (far ? 8 : 0);
    const central = new Bytes(46 + nameBytes.length + (extra > 0 ? 4 + extra : 0))
      .u32(0x02014b50)
      .u16(45)
      .u16(large || far ? 45 : 20)
      .u16(UTF8)
      .u16(0)
      .u16(0)
      .u16(DOS_DATE)
      .u32(0)
      .u32(large ? U32 : size)
      .u32(large ? U32 : size)
      .u16(nameBytes.length)
      .u16(extra > 0 ? 4 + extra : 0)
      .u16(0)
      .u16(0)
      .u16(0)
      .u32(0)
      .u32(far ? U32 : offset)
      .bytes(nameBytes);
    if (extra > 0) {
      central.u16(0x0001).u16(extra);
      if (large) {
        central.u64(size).u64(size);
      }
      if (far) {
        central.u64(offset);
      }
    }
    directory.push(central.buffer);
    parts.push(header, file);
    offset += header.byteLength + size;
  }
  const directoryOffset = offset;
  const directorySize = directory.reduce((sum, entry) => sum + entry.byteLength, 0);
  parts.push(...directory);
  const count = sorted.length;
  if (always || count >= U16 || directoryOffset >= U32 || directorySize >= U32) {
    const end64 = directoryOffset + directorySize;
    parts.push(
      new Bytes(56)
        .u32(0x06064b50)
        .u64(44)
        .u16(45)
        .u16(45)
        .u32(0)
        .u32(0)
        .u64(count)
        .u64(count)
        .u64(directorySize)
        .u64(directoryOffset).buffer,
      new Bytes(20).u32(0x07064b50).u32(0).u64(end64).u32(1).buffer,
      new Bytes(22).u32(0x06054b50).u16(0).u16(0).u16(U16).u16(U16).u32(U32).u32(U32).u16(0).buffer,
    );
  } else {
    parts.push(
      new Bytes(22).u32(0x06054b50).u16(0).u16(0).u16(count).u16(count).u32(directorySize).u32(directoryOffset).u16(0)
        .buffer,
    );
  }
  return new File(parts, name, { type: 'application/zip' });
}

/** The folder name of a folder drop: the first segment its files' paths share, if they share one. */
export function folderName(files: readonly FolderFile[]): string | undefined {
  const roots = new Set(files.map(({ path }) => path.split('/')[0]));
  const [root] = roots;
  return roots.size === 1 && root !== undefined && files.every(({ path }) => path.includes('/')) ? root : undefined;
}

/**
 * What `droppedFiles` reads of a dropped `FileSystemEntry` (`DataTransferItem.webkitGetAsEntry()`): a file entry has
 * `file`, a folder entry `createReader`.
 */
export interface DroppedEntry {
  readonly isDirectory: boolean;
  /** The path from the drop's root, with a leading `/` (`/Deck.key/Index/Document.iwa`). */
  readonly fullPath: string;
}

interface DroppedFileEntry extends DroppedEntry {
  file(success: (file: File) => void, failure?: (error: DOMException) => void): void;
}

interface DroppedFolderEntry extends DroppedEntry {
  createReader(): {
    readEntries(success: (entries: DroppedEntry[]) => void, failure?: (error: DOMException) => void): void;
  };
}

function isFolder(entry: DroppedEntry): entry is DroppedFolderEntry {
  return entry.isDirectory && 'createReader' in entry;
}

function isFile(entry: DroppedEntry): entry is DroppedFileEntry {
  return !entry.isDirectory && 'file' in entry;
}

/**
 * The files under a dropped entry, keyed by their path relative to the drop's root (`Deck.key/Index/Document.iwa`),
 * for `packFolder`. A drop's `File`s have an empty `webkitRelativePath`; the paths come from the entries.
 */
export async function droppedFiles(entry: DroppedEntry): Promise<FolderFile[]> {
  const path = entry.fullPath.startsWith('/') ? entry.fullPath.slice(1) : entry.fullPath;
  if (isFile(entry)) {
    const file = await new Promise<File>((resolve, reject) => entry.file(resolve, reject));
    return [{ path, file }];
  }
  if (!isFolder(entry)) {
    throw new TypeError(`${JSON.stringify(entry.fullPath)} is neither a file nor a folder`);
  }
  const reader = entry.createReader();
  const children: DroppedEntry[] = [];
  // `readEntries` answers in batches and an empty one at the end.
  for (;;) {
    const batch = await new Promise<DroppedEntry[]>((resolve, reject) => reader.readEntries(resolve, reject));
    if (batch.length === 0) {
      break;
    }
    children.push(...batch);
  }
  const files = await Promise.all(children.map((child) => droppedFiles(child)));
  return files.flat();
}
