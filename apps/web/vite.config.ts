import tailwindcss from '@tailwindcss/vite';
import react from '@vitejs/plugin-react';
import { fileURLToPath } from 'node:url';
import { type Plugin, defineConfig } from 'vite';

import { CSP } from './csp.ts';
import { noticesPlugin } from './notices.ts';
import { substitutesPlugin } from './substitutes.ts';

const repoRoot = fileURLToPath(new URL('../..', import.meta.url));

const CHARSET = '<meta charset="utf-8" />';

/**
 * Puts the CSP into the built index.html, right after the charset and before any script. The dev server runs without
 * it: React refresh injects inline scripts.
 */
export function cspPlugin(): Plugin {
  return {
    name: 'slidock-csp',
    apply: 'build',
    transformIndexHtml(html) {
      if (!html.includes(CHARSET)) {
        throw new Error(`index.html has no ${CHARSET} to put the CSP after`);
      }
      return html.replace(CHARSET, `${CHARSET}\n    <meta http-equiv="Content-Security-Policy" content="${CSP}" />`);
    },
  };
}

/**
 * Serves the published bench (`public/bench/`, see `bench/README.md`) at `<base>/bench/` on the dev server, as GitHub
 * Pages serves a folder's `index.html`; the dev server would answer the app's own page there.
 */
export function benchIndexPlugin(): Plugin {
  return {
    name: 'slidock-bench-index',
    apply: 'serve',
    configureServer(server) {
      const folder = `${server.config.base}bench/`;
      server.middlewares.use((request, _response, next) => {
        if (request.url?.split('?')[0] === folder) {
          request.url = `${folder}index.html`;
        }
        next();
      });
    },
  };
}

export default defineConfig({
  base: process.env.VITE_BASE_URL ?? '/',
  plugins: [react(), tailwindcss(), cspPlugin(), substitutesPlugin(), noticesPlugin(), benchIndexPlugin()],
  // wasm-pack's glue loads its .wasm with `new URL(…, import.meta.url)`, which pre-bundling would break.
  optimizeDeps: { exclude: ['@slidock/wasm'] },
  worker: { format: 'es', plugins: () => [noticesPlugin({ emit: false })] },
  build: { target: 'es2022' },
  server: { fs: { allow: [repoRoot] } },
});
