import { createHash } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { stat } from 'node:fs/promises';

/** The SHA-256 and size of a file, read as a stream (the corpus zips reach a gigabyte). */
export async function digest(path: string): Promise<{ sha256: string; bytes: number }> {
  const hash = createHash('sha256');
  for await (const chunk of createReadStream(path)) {
    hash.update(chunk as Buffer);
  }
  return { sha256: hash.digest('hex'), bytes: (await stat(path)).size };
}
