import { screen, waitFor } from '@testing-library/react';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

import { listedPackages, packageOf, unlisted } from '../notices';
import { renderApp, sampleScript } from './harness';

// Vitest runs in the app's folder.
const NOTICES = readFileSync(join(process.cwd(), '../../THIRD-PARTY-NOTICES.txt'), 'utf8');

describe('third-party notices', () => {
  it('list the JavaScript packages of their table, and only those', () => {
    const listed = listedPackages(NOTICES);
    expect([...listed].sort()).toEqual(['react', 'react-dom', 'scheduler', 'tailwindcss', 'vite']);
    expect(listed.has('cssparser')).toBe(false);
  });

  it("name each bundled module's package, and the ones the notices miss", () => {
    expect(packageOf('/repo/node_modules/.pnpm/react@19/node_modules/react/index.js')).toBe('react');
    expect(packageOf('/repo/node_modules/.pnpm/x/node_modules/@scope/pkg/dist/a.js')).toBe('@scope/pkg');
    expect(packageOf('\0vite/preload-helper.js')).toBe('vite');
    expect(packageOf('/repo/apps/web/src/App.tsx')).toBeNull();
    const ids = [
      '/r/node_modules/react/index.js',
      '/r/node_modules/left-pad/index.js',
      '/r/node_modules/@slidock/ui/src/index.ts',
      '\0vite/modulepreload-polyfill.js',
    ];
    expect(unlisted(ids, listedPackages(NOTICES))).toEqual(['left-pad']);
  });

  it('the landing links them and says whose trademarks Keynote and PowerPoint are', async () => {
    renderApp(sampleScript());
    await waitFor(() => expect(screen.getByTestId('wasm-status').textContent).toBe('slidock-wasm 9.9.9'));
    expect(screen.getByRole('link', { name: 'Licences' }).getAttribute('href')).toBe('/THIRD-PARTY-NOTICES.txt');
    expect(
      screen.getByText(
        'Keynote is a trademark of Apple Inc. and PowerPoint of Microsoft Corporation; slidock is not affiliated with either.',
      ),
    ).toBeTruthy();
  });
});
