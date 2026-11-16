// Resolves the extensionless relative imports of `src/` (written for bundlers) to their `.ts` files, so Node runs them
// with its type stripping: `node --import ./ts-resolve.ts …`. Workers inherit the flag.
import { registerHooks } from 'node:module';

registerHooks({
  resolve(specifier, context, nextResolve) {
    const relative = specifier.startsWith('./') || specifier.startsWith('../');
    if (relative && !/\.[cm]?[jt]s$/.test(specifier)) {
      return nextResolve(`${specifier}.ts`, context);
    }
    return nextResolve(specifier, context);
  },
});
