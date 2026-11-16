// The bundled substitutes the app serves at `fonts/core/` under its base URL: the files of `slidock-substitutes`
// (`crates/slidock-substitutes/fonts/`, which the CLI builds in), served by the dev server and copied into the build,
// so the two hosts ship the same bytes.
import { readFileSync, readdirSync } from 'node:fs';
import { extname, join } from 'node:path';
import type { Plugin } from 'vite';

/** The folder the files come from. */
export const SUBSTITUTES_DIR = join(import.meta.dirname, '../../crates/slidock-substitutes/fonts');

/** Where the app serves them, under its base URL. */
export const SUBSTITUTES_PATH = 'fonts/core/';

const TYPES: Record<string, string> = {
  '.json': 'application/json',
  '.txt': 'text/plain; charset=utf-8',
  '.woff2': 'font/woff2',
};

/** The folder's file names. */
export function substituteFiles(): string[] {
  return readdirSync(SUBSTITUTES_DIR).sort();
}

/** The file a path under the base URL (`fonts/core/<file>`) names, when it is one of the folder's. */
export function substitutePath(path: string): string | undefined {
  const name = path.startsWith(SUBSTITUTES_PATH) ? path.slice(SUBSTITUTES_PATH.length) : undefined;
  return name !== undefined && substituteFiles().includes(name) ? join(SUBSTITUTES_DIR, name) : undefined;
}

/** Serves the substitutes in the dev server, and writes them into the build at `fonts/core/`. */
export function substitutesPlugin(): Plugin {
  return {
    name: 'slidock-substitutes',
    configureServer(server) {
      server.middlewares.use((request, response, next) => {
        const { base } = server.config;
        const url = (request.url ?? '').split('?')[0] ?? '';
        const file = url.startsWith(base) ? substitutePath(decodeURIComponent(url.slice(base.length))) : undefined;
        if (file === undefined) {
          next();
          return;
        }
        response.setHeader('Content-Type', TYPES[extname(file)] ?? 'application/octet-stream');
        response.end(readFileSync(file));
      });
    },
    generateBundle() {
      for (const name of substituteFiles()) {
        this.emitFile({
          type: 'asset',
          fileName: `${SUBSTITUTES_PATH}${name}`,
          source: readFileSync(join(SUBSTITUTES_DIR, name)),
        });
      }
    },
  };
}
