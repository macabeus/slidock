// Fails with a clear message when the wasm-pack build this package exports (`pkg/`) is missing: the type checks, tests
// and builds of the packages that import it need it first.
//
//   node packages/wasm/require-pkg.mjs
import { existsSync } from 'node:fs';

const files = ['slidock_wasm.js', 'slidock_wasm.d.ts', 'slidock_wasm_bg.wasm'];
const missing = files.filter((file) => !existsSync(new URL(`./pkg/${file}`, import.meta.url)));
if (missing.length > 0) {
  console.error(
    `packages/wasm/pkg/ is not built (${missing.join(', ')} missing): run \`pnpm wasm:build\` first (it needs the ` +
      'Rust toolchain and wasm-pack; see the README).',
  );
  process.exit(1);
}
