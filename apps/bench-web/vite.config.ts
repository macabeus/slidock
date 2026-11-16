import tailwindcss from '@tailwindcss/vite';
import react from '@vitejs/plugin-react';
import { readFileSync } from 'node:fs';
import { type Plugin, defineConfig } from 'vite';
import { viteSingleFile } from 'vite-plugin-singlefile';

import { loadFixture } from './mock/fixtures.ts';
import { benchMock } from './mock/plugin.ts';
import { samplePayload } from './mock/report.ts';
import { REPORT_DATA_SLOT, checkEmptyReportPage, injectReportData } from './src/data/inline.ts';

/**
 * The single-file report's data slot, in the page's head. `SLIDOCK_BENCH_REPORT_DATA` fills it at build time: a path
 * to a report JSON, or `sample` for the synthetic bench. Unset, the slot stays empty for `slidock-bench report`, and
 * the written page must hold the marker only there.
 */
export function reportSlot(data: string | undefined): Plugin {
  const empty = data === undefined || data === '';
  return {
    name: 'slidock-bench-report-slot',
    apply: 'build',
    writeBundle(_options, bundle) {
      const page = bundle['index.html'];
      if (!page || page.type !== 'asset') {
        throw new Error('the report build wrote no index.html');
      }
      if (empty) {
        checkEmptyReportPage(typeof page.source === 'string' ? page.source : new TextDecoder().decode(page.source));
      }
    },
    transformIndexHtml: {
      order: 'post',
      handler(html) {
        if (!html.includes('</head>')) {
          throw new Error('index.html has no </head> to put the report data slot before');
        }
        const page = html.replace('</head>', () => `  ${REPORT_DATA_SLOT}\n  </head>`);
        if (empty) {
          return page;
        }
        const payload: unknown =
          data === 'sample' ? samplePayload(loadFixture()) : JSON.parse(readFileSync(data, 'utf8'));
        return injectReportData(page, payload);
      },
    },
  };
}

export default defineConfig(({ mode }) => {
  const report = mode === 'report';
  return {
    base: './',
    plugins: [
      react(),
      tailwindcss(),
      ...(report ? [reportSlot(process.env.SLIDOCK_BENCH_REPORT_DATA), viteSingleFile()] : [benchMock()]),
    ],
    build: {
      target: 'es2022',
      outDir: report ? 'dist/report' : 'dist',
      emptyOutDir: true,
      // The report is one file by design.
      ...(report ? { chunkSizeWarningLimit: 2048 } : {}),
    },
  };
});
