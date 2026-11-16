// The determinism gate's wasm leg: the compiler built to WebAssembly (`pnpm wasm:build`),
// run under Node on one presentation, writing what the native CLI writes for the same input and `--fonts <dir>
// --fonts-off` (no font source, nothing shipped: the parity path).
//
//   node bench/wasm-cli.mjs <pkg dir> <input> <name> <documents dir> <deck.zip> [<fonts dir>]
//
// It reads the input by ranges, as the compile worker does, adds the font folder's files one by one in the order the
// CLI reads them (every .ttf, .otf and .ttc below it, by full path in UTF-16 order), then writes the document set
// (`--emit documents`: text and media) and the bundled deck zip, and prints one JSON line: the stage that failed and
// its error, if any.
import { closeSync, fstatSync, mkdirSync, openSync, readFileSync, readSync, readdirSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { pathToFileURL } from 'node:url';

const [pkg, input, name, documents, zip, fontsDir] = process.argv.slice(2);
if (zip === undefined) {
  console.error('usage: node bench/wasm-cli.mjs <pkg dir> <input> <name> <documents dir> <deck.zip> [<fonts dir>]');
  process.exit(2);
}

function fontFiles(dir) {
  const files = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) {
      files.push(...fontFiles(path));
    } else if (entry.isFile() && /\.(ttf|otf|ttc)$/i.test(entry.name)) {
      files.push(path);
    }
  }
  return files;
}

function write(path, bytes) {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, bytes);
}

const wasm = await import(pathToFileURL(join(pkg, 'slidock_wasm.js')).href);
wasm.initSync({ module: readFileSync(join(pkg, 'slidock_wasm_bg.wasm')) });

const fd = openSync(input, 'r');
const source = {
  size: fstatSync(fd).size,
  read(offset, length) {
    const bytes = new Uint8Array(length);
    let done = 0;
    while (done < length) {
      const read = readSync(fd, bytes, done, length - done, offset + done);
      if (read === 0) {
        throw new RangeError(`the file ended at ${offset + done}`);
      }
      done += read;
    }
    return bytes;
  },
};

const result = { stage: null, error: null };
let stage = 'open';
try {
  const compiler = new wasm.Compiler();
  compiler.open(name, source, false);
  if (fontsDir !== undefined) {
    stage = 'fonts';
    for (const file of fontFiles(fontsDir).sort()) {
      compiler.addFont(readFileSync(file), 'local');
    }
  }
  stage = 'finish';
  compiler.finish();
  stage = 'print';
  compiler.print();
  stage = 'documents';
  const set = compiler.documents();
  for (const [path, text] of Object.entries(set.text)) {
    write(join(documents, path), text);
  }
  for (const path of set.media) {
    write(join(documents, path), compiler.mediaBytes(path));
  }
  stage = 'bundle';
  const chunks = [];
  compiler.bundle(
    JSON.stringify({ singleFile: false, singleFileThreshold: null, leaveOut: [], include: [] }),
    (chunk) => {
      chunks.push(chunk);
    },
  );
  write(zip, Buffer.concat(chunks));
} catch (error) {
  result.stage = stage;
  result.error = error instanceof Error ? error.message : JSON.stringify(error);
} finally {
  closeSync(fd);
}
process.stdout.write(`${JSON.stringify(result)}\n`);
