// The font requests to the compile worker (packages/wasm/README.md): what is still missing, a font file added, and
// the installed faces worth reading.
import type { AddedFontSource, FontNeed, LocalFontMeta, LocalPick } from '@slidock/types';

import type { CompileClient } from '../compile/client';

export class FontRequests {
  readonly #client: CompileClient;

  constructor(client: CompileClient) {
    this.#client = client;
  }

  /** The faces no font provides yet. */
  async needs(): Promise<FontNeed[]> {
    let needs: FontNeed[] = [];
    await this.#client.request({ type: 'fontsNeeded' }, { onNeeds: (list) => (needs = list) });
    return needs;
  }

  /**
   * Adds a font file for the next finish, moving `bytes` to the worker; resolves with what is still missing. A file
   * that is no font rejects with the worker's `request` error.
   */
  async addFont(bytes: ArrayBuffer, source: AddedFontSource, unicodeRange: string | null): Promise<FontNeed[]> {
    let needs: FontNeed[] = [];
    await this.#client.request(
      { type: 'addFont', bytes, source, ...(unicodeRange === null ? {} : { unicodeRange }) },
      { transfer: [bytes], onNeeds: (list) => (needs = list) },
    );
    return needs;
  }

  /**
   * The installed faces the worker matched against the open faces; `null` from a worker that answers with no
   * `localPicks` (it cannot match installed fonts).
   */
  async selectLocal(fonts: LocalFontMeta[]): Promise<LocalPick[] | null> {
    let picks: LocalPick[] | null = null;
    await this.#client.request({ type: 'selectLocal', fonts }, { onLocalPicks: (list) => (picks = list) });
    return picks;
  }
}
