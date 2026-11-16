import type { MediaHandle } from '@slidock/types';

/** `blob:` URLs for a deck's media, by `media/…` path. `revoke` releases them all; it is safe to call twice. */
export class MediaUrls {
  #urls = new Map<string, string>();
  #blobs = new Map<string, Blob>();

  constructor(media: readonly MediaHandle[]) {
    for (const { path, blob } of media) {
      this.#urls.set(path, URL.createObjectURL(blob));
      this.#blobs.set(path, blob);
    }
  }

  /**
   * Whether `media` are exactly these: the same paths. A path names the same bytes on every finish of one input
   * (`packages/wasm/src/serve.ts`), and a `Blob` never keeps its identity across `postMessage`, so paths are compared.
   */
  holds(media: readonly MediaHandle[]): boolean {
    return media.length === this.#blobs.size && media.every(({ path }) => this.#blobs.has(path));
  }

  /** The URL for `path`, or `path` itself when the deck has no such medium. */
  url = (path: string): string => this.#urls.get(path) ?? path;

  get size(): number {
    return this.#urls.size;
  }

  revoke() {
    for (const url of this.#urls.values()) {
      URL.revokeObjectURL(url);
    }
    this.#urls.clear();
    this.#blobs.clear();
  }
}
