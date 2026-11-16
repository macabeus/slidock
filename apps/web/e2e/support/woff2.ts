// Reads the names of a shipped WOFF2 face (the `name` table, which WOFF2 never transforms), so the font matrix knows
// which family and PostScript name a browser should report for each `@font-face` rule.
import { brotliDecompressSync } from 'node:zlib';

/** WOFF2's known table tags, by index (the WOFF2 specification, §5.1). */
const KNOWN_TAGS = [
  'cmap', 'head', 'hhea', 'hmtx', 'maxp', 'name', 'OS/2', 'post', 'cvt ', 'fpgm', 'glyf', 'loca', 'prep', 'CFF ',
  'VORG', 'EBDT', 'EBLC', 'gasp', 'hdmx', 'kern', 'LTSH', 'PCLT', 'VDMX', 'vhea', 'vmtx', 'BASE', 'GDEF', 'GPOS',
  'GSUB', 'EBSC', 'JSTF', 'MATH', 'CBDT', 'CBLC', 'COLR', 'CPAL', 'SVG ', 'sbix', 'acnt', 'avar', 'bdat', 'bloc',
  'bsln', 'cvar', 'fdsc', 'feat', 'fmtx', 'fvar', 'gvar', 'hsty', 'just', 'lcar', 'mort', 'morx', 'opbd', 'prop',
  'trak', 'Zapf', 'Silf', 'Glat', 'Gloc', 'Feat', 'Sill',
]; // prettier-ignore

/** The names a browser reports for a face. */
export interface FaceNames {
  /** The typographic family (name ID 16), else the family (name ID 1). */
  family: string;
  /** Name ID 6; empty when the file has none. */
  postscript: string;
}

class Reader {
  at = 0;
  constructor(readonly bytes: Buffer) {}

  u8(): number {
    const value = this.bytes.readUInt8(this.at);
    this.at += 1;
    return value;
  }

  base128(): number {
    let value = 0;
    for (let i = 0; i < 5; i++) {
      const byte = this.u8();
      if (i === 0 && byte === 0x80) {
        throw new Error('UIntBase128 with a leading zero');
      }
      value = value * 128 + (byte & 0x7f);
      if ((byte & 0x80) === 0) {
        return value;
      }
    }
    throw new Error('UIntBase128 longer than five bytes');
  }
}

/** The decompressed bytes of table `wanted` of a single-face WOFF2 file, or `undefined` when it has none. */
export function woff2Table(file: Buffer, wanted: string): Buffer | undefined {
  if (file.toString('latin1', 0, 4) !== 'wOF2') {
    throw new Error('not a WOFF2 file');
  }
  if (file.toString('latin1', 4, 8) === 'ttcf') {
    throw new Error('a WOFF2 collection');
  }
  const tables = file.readUInt16BE(12);
  const compressedSize = file.readUInt32BE(20);
  const reader = new Reader(file);
  reader.at = 48;
  let offset = 0;
  let found: { offset: number; length: number } | undefined;
  for (let i = 0; i < tables; i++) {
    const flags = reader.u8();
    let tag: string;
    if ((flags & 0x3f) === 0x3f) {
      tag = file.toString('latin1', reader.at, reader.at + 4);
      reader.at += 4;
    } else {
      tag = KNOWN_TAGS[flags & 0x3f] ?? '';
    }
    const version = flags >> 6;
    const original = reader.base128();
    const transformed = tag === 'glyf' || tag === 'loca' ? version !== 3 : version !== 0;
    const length = transformed ? reader.base128() : original;
    if (tag === wanted) {
      found = { offset, length };
    }
    offset += length;
  }
  if (found === undefined) {
    return undefined;
  }
  const stream = brotliDecompressSync(file.subarray(reader.at, reader.at + compressedSize));
  return stream.subarray(found.offset, found.offset + found.length);
}

function utf16be(bytes: Buffer): string {
  let text = '';
  for (let i = 0; i + 1 < bytes.length; i += 2) {
    text += String.fromCharCode(bytes.readUInt16BE(i));
  }
  return text;
}

/** The family and PostScript name of a single-face WOFF2 file (Windows Unicode names first, then Macintosh). */
export function woff2Names(file: Buffer): FaceNames {
  const table = woff2Table(file, 'name');
  if (table === undefined) {
    throw new Error('the face has no name table');
  }
  const count = table.readUInt16BE(2);
  const strings = table.readUInt16BE(4);
  const found = new Map<number, { rank: number; text: string }>();
  for (let i = 0; i < count; i++) {
    const record = 6 + i * 12;
    const platform = table.readUInt16BE(record);
    const encoding = table.readUInt16BE(record + 2);
    const language = table.readUInt16BE(record + 4);
    const id = table.readUInt16BE(record + 6);
    const length = table.readUInt16BE(record + 8);
    const start = strings + table.readUInt16BE(record + 10);
    const bytes = table.subarray(start, start + length);
    let rank: number;
    let text: string;
    if (platform === 3 && (encoding === 1 || encoding === 10)) {
      rank = language === 0x409 ? 0 : 1;
      text = utf16be(bytes);
    } else if (platform === 0) {
      rank = 2;
      text = utf16be(bytes);
    } else if (platform === 1 && encoding === 0) {
      rank = 3;
      text = bytes.toString('latin1');
    } else {
      continue;
    }
    const best = found.get(id);
    if (best === undefined || rank < best.rank) {
      found.set(id, { rank, text });
    }
  }
  return {
    family: found.get(16)?.text ?? found.get(1)?.text ?? '',
    postscript: found.get(6)?.text ?? '',
  };
}
