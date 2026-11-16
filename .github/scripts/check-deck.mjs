// Checks a deck zip the release CLI wrote: its index.html embeds the player and the presenter view the repository
// holds (crates/slidock-bundle/player/), byte for byte.
//
//   node .github/scripts/check-deck.mjs <deck.zip>
import { readFileSync } from 'node:fs';
import { inflateRawSync } from 'node:zlib';

const [zipPath] = process.argv.slice(2);
if (zipPath === undefined) {
  throw new Error('usage: node .github/scripts/check-deck.mjs <deck.zip>');
}

/** The bytes of one entry of a zip, read through its central directory. */
function entry(zip, name) {
  const end = zip.lastIndexOf(Buffer.from([0x50, 0x4b, 0x05, 0x06]));
  if (end < 0) {
    throw new Error(`${zipPath}: no end of central directory`);
  }
  const count = zip.readUInt16LE(end + 10);
  let at = zip.readUInt32LE(end + 16);
  for (let index = 0; index < count; index += 1) {
    const method = zip.readUInt16LE(at + 10);
    const size = zip.readUInt32LE(at + 20);
    const nameLength = zip.readUInt16LE(at + 28);
    const extra = zip.readUInt16LE(at + 30);
    const comment = zip.readUInt16LE(at + 32);
    const local = zip.readUInt32LE(at + 42);
    if (zip.toString('utf8', at + 46, at + 46 + nameLength) === name) {
      const start = local + 30 + zip.readUInt16LE(local + 26) + zip.readUInt16LE(local + 28);
      const data = zip.subarray(start, start + size);
      return method === 8 ? inflateRawSync(data) : data;
    }
    at += 46 + nameLength + extra + comment;
  }
  throw new Error(`${zipPath}: no ${name}`);
}

const page = entry(readFileSync(zipPath), 'index.html').toString('utf8');
for (const script of ['player.iife.js', 'presenter.iife.js']) {
  const text = readFileSync(new URL(`../../crates/slidock-bundle/player/${script}`, import.meta.url), 'utf8');
  if (!page.includes(text)) {
    throw new Error(`${zipPath}: index.html does not embed crates/slidock-bundle/player/${script}`);
  }
}
console.log(`${zipPath}: index.html embeds the player and the presenter view`);
