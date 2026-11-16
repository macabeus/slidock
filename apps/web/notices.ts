// THIRD-PARTY-NOTICES.txt (`cargo xtask notices`) in the web app: the build copies it next to index.html, and the
// dev server serves it at the same path. The build fails when its bundle holds code from a package the notices do
// not list, so the list cannot fall behind the dependencies.
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import type { Plugin } from 'vite';

export const NOTICES_FILE = 'THIRD-PARTY-NOTICES.txt';

const NOTICES_PATH = fileURLToPath(new URL(`../../${NOTICES_FILE}`, import.meta.url));

/** The code Vite itself adds to a build (its module preload helper and polyfill), by module id. */
const VITE_MODULES = /^\0vite\//;

/** The JavaScript packages the notices list: the names of their "JavaScript packages" table. */
export function listedPackages(notices: string): Set<string> {
  const section = /\nJavaScript packages\n-+\n([\s\S]*?)\nLicence texts\n/.exec(notices)?.[1] ?? '';
  const names = [...section.matchAll(/^ {2}(\S+) \S+ /gm)].map((match) => match[1] ?? '');
  return new Set(names);
}

/** The npm package a bundled module comes from; `null` for the app's own and the workspace's modules. */
export function packageOf(id: string): string | null {
  if (VITE_MODULES.test(id)) {
    return 'vite';
  }
  const path = id.replaceAll('\\', '/');
  const at = path.lastIndexOf('/node_modules/');
  if (at < 0) {
    return null;
  }
  const [scope = '', name = ''] = path.slice(at + '/node_modules/'.length).split('/');
  return scope.startsWith('@') ? `${scope}/${name}` : scope;
}

/** The packages of `moduleIds` the notices do not list, sorted. Workspace packages (`@slidock/*`) are slidock's. */
export function unlisted(moduleIds: Iterable<string>, listed: Set<string>): string[] {
  const missing = new Set<string>();
  for (const id of moduleIds) {
    const name = packageOf(id);
    if (name !== null && !name.startsWith('@slidock/') && !listed.has(name)) {
      missing.add(name);
    }
  }
  return [...missing].sort();
}

/**
 * Serves and emits the notices, after checking the bundle against them; with `emit` false (the worker builds) it only
 * checks.
 */
export function noticesPlugin({ emit = true }: { emit?: boolean } = {}): Plugin {
  return {
    name: 'slidock-notices',
    configureServer(server) {
      server.middlewares.use(`/${NOTICES_FILE}`, (_request, response) => {
        response.setHeader('Content-Type', 'text/plain; charset=utf-8');
        response.end(readFileSync(NOTICES_PATH));
      });
    },
    generateBundle(_options, bundle) {
      const notices = readFileSync(NOTICES_PATH, 'utf8');
      const moduleIds = Object.values(bundle).flatMap((output) => (output.type === 'chunk' ? output.moduleIds : []));
      const missing = unlisted(moduleIds, listedPackages(notices));
      if (missing.length > 0) {
        this.error(
          `the bundle holds code from ${missing.join(', ')}, which ${NOTICES_FILE} does not list: add them to ` +
            'JS_PACKAGES in xtask/src/notices.rs and run cargo xtask notices',
        );
      }
      if (emit) {
        this.emitFile({ type: 'asset', fileName: NOTICES_FILE, source: notices });
      }
    },
  };
}
