import { defineConfig } from 'vitest/config';

// Unit tests only; the Playwright effect suite under e2e/ runs with `pnpm e2e`.
export default defineConfig({ test: { include: ['test/**/*.test.ts'] } });
