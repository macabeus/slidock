// Compiles presentations with the built wasm module under Node, for slidock-cli's native-vs-wasm parity test:
//
//   node --import ./packages/wasm/test/node/ts-resolve.ts packages/wasm/test/node/compile.ts <out> \
//     [--font=<file>...] [--core=<dir>] [--media=<need>=<file>...] <name>=<input>...
//
// For each input, into `<out>/<name>/`:
// - `documents/`: the document set (`--emit documents`) from the compiler called directly over a range reader, with
//   no font source;
// - `deck.zip`: the zip the compile worker (`src/serve.ts`, in a worker_threads Worker) streams for `open` (with the
//   input's embedded fonts), an `addFont` as `user` per `--font` file in order, an `addMedia` per `--media` file (a
//   decoded HEIC photo, as the web app adds it: the need's id, the file), `finish`, then, as the web app and
//   the CLI do, an `addFont` as `bundled` per substitute face the deck names that the `--core` folder's `core.json`
//   lists (`crates/slidock-substitutes/fonts/`, the app's `fonts/core/`: same family, weight and style, SHA-256 checked, each file once) and
//   `finish` again, until a deck names none it lacks, and `bundle`, when it gets that far;
// - `result.json`: each request's messages, the zip's chunk sizes, the errors, whether every medium the worker serves
//   as a slice of the input holds the bytes the compiler writes for it, and the wasm memory each module instance
//   reached (it never shrinks).
import type { FromWorker } from '@slidock/types';
import { createHash } from 'node:crypto';
import { closeSync, fstatSync, mkdirSync, openSync, readFileSync, readSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { Worker } from 'node:worker_threads';

import { Compiler as WasmCompiler, initSync } from '../../pkg/slidock_wasm.js';
import { Compiler, StageError } from '../../src/compiler';
import type { RangeSource } from '../../src/compiler';
import type { HostReply, HostRequest } from './worker-host';

function fileSource(fd: number): RangeSource {
  return {
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
}

function write(path: string, bytes: Uint8Array | string): void {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, bytes);
}

function describe(error: unknown): unknown {
  return error instanceof StageError ? error.error : String(error);
}

/** The document set, from the compiler called directly. */
function documents(input: string, name: string, out: string): object {
  const fd = openSync(input, 'r');
  try {
    const compiler = new Compiler(new WasmCompiler());
    compiler.open(name, fileSource(fd), false);
    compiler.finish();
    let print: unknown = null;
    try {
      compiler.print();
    } catch (error) {
      print = describe(error);
    }
    const set = compiler.documents();
    for (const [path, text] of Object.entries(set.text)) {
      write(join(out, path), text);
    }
    for (const path of set.media) {
      write(join(out, path), compiler.mediaBytes(path));
    }
    return { print };
  } catch (error) {
    return { error: describe(error) };
  } finally {
    closeSync(fd);
  }
}

/** A face of the substitutes manifest (`crates/slidock-substitutes/fonts/core.json`). */
interface CoreFace {
  family: string;
  weight: number;
  style: string;
  file: string;
  sha256: string;
}

/** The manifest's faces with their folder, read as the app reads them. */
interface Core {
  dir: string;
  faces: CoreFace[];
}

function readCore(dir: string): Core {
  const manifest: unknown = JSON.parse(readFileSync(join(dir, 'core.json'), 'utf8'));
  const faces = typeof manifest === 'object' && manifest !== null ? (manifest as { faces?: unknown }).faces : null;
  if (!Array.isArray(faces)) {
    throw new Error(`${dir}/core.json lists no faces`);
  }
  return { dir, faces: faces as CoreFace[] };
}

/** The manifest's face for a substitute face the worker names, as the app's `substituteFile` matches it. */
function substituteFile(core: Core, face: { family: string; weight: number; style: string }): CoreFace | undefined {
  const family = face.family.toLowerCase();
  return core.faces.find(
    (entry) => entry.family.toLowerCase() === family && entry.weight === face.weight && entry.style === face.style,
  );
}

/** A manifest face's file, checked against its SHA-256. */
function readChecked(core: Core, face: CoreFace): ArrayBuffer {
  const bytes = new Uint8Array(readFileSync(join(core.dir, face.file)));
  if (createHash('sha256').update(bytes).digest('hex') !== face.sha256) {
    throw new Error(`${face.file} is not the file the manifest records`);
  }
  return bytes.buffer;
}

interface Session {
  replies: FromWorker[];
  peakMemory: number;
}

/** `open`, `finish` and `bundle` through the compile worker. */
async function throughWorker(
  input: string,
  name: string,
  out: string,
  fonts: string[],
  core: Core | null,
  decoded: [string, string][],
): Promise<unknown> {
  const worker = new Worker(new URL('./worker-host.ts', import.meta.url));
  const session: Session = { replies: [], peakMemory: 0 };
  let pending: { id: number; resolve: () => void } | undefined;
  worker.on('message', (reply: HostReply) => {
    if (reply.type === 'memory') {
      session.peakMemory = Math.max(session.peakMemory, reply.bytes);
      return;
    }
    session.replies.push(reply);
    if (reply.type !== 'ready' && (reply.type === 'done' || reply.type === 'error') && reply.id === pending?.id) {
      pending.resolve();
    }
  });
  const request = (message: HostRequest) =>
    new Promise<void>((resolve) => {
      pending = { id: message.id, resolve };
      worker.postMessage(message);
    });
  const failed = (id: number) => session.replies.some((reply) => reply.type === 'error' && reply.id === id);
  try {
    await request({ type: 'open', id: 1, path: input, name });
    for (const [index, font] of fonts.entries()) {
      const bytes = new Uint8Array(readFileSync(font)).buffer;
      await request({ type: 'addFont', id: 10 + index, bytes, source: 'user' });
    }
    for (const [index, [need, file]] of decoded.entries()) {
      const bytes = new Uint8Array(readFileSync(file)).buffer;
      await request({ type: 'addMedia', id: 40 + index, need, bytes });
    }
    if (!failed(1)) {
      await request({ type: 'finish', id: 2 });
      // Each round adds the files of the substitute faces the last deck names, then finishes again.
      const tried = new Set<string>();
      let finishId = 2;
      let addId = 20;
      for (let round = 0; core !== null; round += 1) {
        const id = finishId;
        const finished = session.replies.find(
          (reply): reply is Extract<FromWorker, { type: 'deck' }> => reply.type === 'deck' && reply.id === id,
        );
        let added = 0;
        for (const wanted of finished?.substitutes ?? []) {
          const face = substituteFile(core, wanted);
          if (face === undefined || tried.has(face.file)) {
            continue;
          }
          tried.add(face.file);
          await request({ type: 'addFont', id: addId, bytes: readChecked(core, face), source: 'bundled' });
          addId += 1;
          added += 1;
        }
        if (added === 0) {
          break;
        }
        finishId = 100 + round;
        await request({ type: 'finish', id: finishId });
      }
    }
    // The bundle writes the last printed deck: it runs whether or not `finish` printed one.
    await request({
      type: 'bundle',
      id: 3,
      settings: { singleFile: false, singleFileThreshold: null, leaveOut: [], include: [] },
    });
  } finally {
    await worker.terminate();
  }

  const chunks = session.replies.flatMap((reply) => (reply.type === 'zipChunk' ? [new Uint8Array(reply.bytes)] : []));
  const ended = session.replies.find((reply) => reply.type === 'done' && reply.id === 3);
  if (ended !== undefined) {
    write(join(out, 'deck.zip'), Buffer.concat(chunks));
  }

  // Each medium served as a slice of the input holds the bytes the documents' media file holds.
  const media: Record<string, boolean> = {};
  const deck = session.replies.findLast(
    (reply): reply is Extract<FromWorker, { type: 'deck' }> => reply.type === 'deck',
  );
  if (deck !== undefined) {
    const fd = openSync(input, 'r');
    try {
      const source = fileSource(fd);
      for (const handle of deck.media) {
        const blob: unknown = handle.blob;
        if (typeof blob === 'object' && blob !== null && 'range' in blob && Array.isArray(blob.range)) {
          const [start, end] = blob.range as [number, number];
          const expected = readFileSync(join(out, 'documents', handle.path));
          media[handle.path] = Buffer.from(source.read(start, end - start)).equals(expected);
        }
      }
    } finally {
      closeSync(fd);
    }
  }
  return {
    messages: session.replies.map((reply) =>
      reply.type === 'ready' ? 'ready' : `${reply.id}:${reply.type}${'phase' in reply ? `:${reply.phase}` : ''}`,
    ),
    errors: session.replies.flatMap((reply) => (reply.type === 'error' ? [{ id: reply.id, error: reply.error }] : [])),
    chunks: chunks.map((chunk) => chunk.length),
    slides: deck?.slides.length ?? null,
    rangeMedia: media,
    peakMemory: session.peakMemory,
  };
}

const [out, ...rest] = process.argv.slice(2);
const fonts = rest.filter((argument) => argument.startsWith('--font=')).map((argument) => argument.slice(7));
const coreDir = rest.findLast((argument) => argument.startsWith('--core='))?.slice(7);
const core = coreDir === undefined ? null : readCore(coreDir);
const media = rest
  .filter((argument) => argument.startsWith('--media='))
  .map((argument): [string, string] => {
    const value = argument.slice(8);
    const at = value.indexOf('=');
    return [value.slice(0, at), value.slice(at + 1)];
  });
const inputs = rest.filter((argument) => !/^--(font|core|media)=/.test(argument));
if (out === undefined || inputs.length === 0) {
  console.error(
    'usage: node compile.ts <out> [--font=<file>...] [--core=<dir>] [--media=<need>=<file>...] <name>=<input>...',
  );
  process.exit(2);
}
const wasm = initSync({ module: readFileSync(new URL('../../pkg/slidock_wasm_bg.wasm', import.meta.url)) });
for (const argument of inputs) {
  const at = argument.indexOf('=');
  const [name, input] = [argument.slice(0, at), argument.slice(at + 1)];
  const target = join(out, name);
  rmSync(target, { recursive: true, force: true });
  const fileName = input.split('/').at(-1) ?? input;
  const direct = { ...documents(input, fileName, join(target, 'documents')), memory: wasm.memory.buffer.byteLength };
  const worker = await throughWorker(input, fileName, target, fonts, core, media);
  write(join(target, 'result.json'), `${JSON.stringify({ documents: direct, worker }, null, 2)}\n`);
}
