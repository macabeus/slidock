import { screen, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { openSample, renderApp, sampleScript } from './harness';

/** The buttons, links and form controls with no accessible name. */
function unnamedControls(): HTMLElement[] {
  return ['button', 'link', 'checkbox', 'tab', 'textbox'].flatMap((role) => screen.queryAllByRole(role, { name: '' }));
}

/** `aria-controls` and `aria-labelledby` references that name no element. */
function danglingReferences(): string[] {
  return [...document.querySelectorAll('[aria-controls], [aria-labelledby]')].flatMap((node) =>
    ['aria-controls', 'aria-labelledby'].flatMap((attribute) =>
      (node.getAttribute(attribute) ?? '')
        .split(/\s+/)
        .filter((id) => id !== '' && !document.getElementById(id))
        .map((id) => `${attribute}=${id}`),
    ),
  );
}

/** Live regions of the app's own chrome (the player's are inside the stage). */
function chromeLiveRegions(): Element[] {
  const stage = screen.getByTestId('stage');
  return [...document.querySelectorAll('[aria-live], [role="status"], [role="alert"], [role="log"]')].filter(
    (node) => !stage.contains(node),
  );
}

async function tabOrder(app: ReturnType<typeof renderApp>, count: number): Promise<string[]> {
  const names: string[] = [];
  for (let i = 0; i < count; i++) {
    await app.user.tab();
    const active = document.activeElement as HTMLElement;
    names.push(active.getAttribute('aria-label') ?? active.textContent?.trim() ?? active.tagName);
  }
  return names;
}

describe('accessibility', () => {
  it('landing: named landmark and controls, focus goes theme → choose', async () => {
    const app = renderApp(sampleScript());
    expect(screen.getByRole('main', { name: 'Open a presentation' })).toBeTruthy();
    expect(unnamedControls()).toEqual([]);
    const order = await tabOrder(app, 2);
    expect(order[0]).toMatch(/^System theme/);
    expect(order[1]).toBe('Choose a presentation');
  });

  it('viewer: landmarks, current thumbnail, and a toolbar-first focus order', async () => {
    const app = renderApp(sampleScript());
    await openSample(app);
    expect(screen.getByRole('main', { name: 'Deck viewer' })).toBeTruthy();
    expect(screen.getByRole('navigation', { name: 'Slides' })).toBeTruthy();
    expect(screen.getByRole('region', { name: 'Notes and diagnostics' })).toBeTruthy();
    expect(unnamedControls()).toEqual([]);

    const order = await tabOrder(app, 12);
    expect(order).toEqual([
      'Open another',
      expect.stringMatching(/^System theme/),
      'Previous slide',
      'Next slide',
      'Fonts',
      'Keyboard shortcuts',
      'Presenter view',
      'Download zip',
      'Present',
      'Slide 1: Quarterly review',
      'Notes',
      'Expand notes and diagnostics',
    ]);
  });

  it('viewer: every ARIA reference resolves, collapsed or expanded', async () => {
    const app = renderApp(sampleScript());
    await openSample(app);
    expect(danglingReferences()).toEqual([]);
    await app.user.click(screen.getByRole('button', { name: 'Expand notes and diagnostics' }));
    expect(screen.getByRole('tabpanel')).toBeTruthy();
    expect(danglingReferences()).toEqual([]);
    await app.user.click(screen.getByRole('tab', { name: /^Diagnostics/ }));
    expect(danglingReferences()).toEqual([]);
  });

  it('viewer: moves are announced by the player alone, not by a second live region in the chrome', async () => {
    const app = renderApp(sampleScript());
    await openSample(app);
    expect(chromeLiveRegions()).toEqual([]);
    await app.user.click(screen.getByRole('button', { name: 'Next slide' }));
    expect(within(screen.getByRole('toolbar', { name: 'Deck' })).getByText('Slide 2 of 3')).toBeTruthy();
    expect(chromeLiveRegions()).toEqual([]);
  });

  it('dialogs are named, modal, take focus and keep it inside', async () => {
    const app = renderApp(sampleScript());
    await openSample(app);
    const opener = screen.getByRole('button', { name: 'Download zip' });
    await app.user.click(opener);
    const dialog = screen.getByRole('dialog', { name: 'Download zip' });
    expect(dialog.getAttribute('aria-modal')).toBe('true');
    expect(document.activeElement).toBe(within(dialog).getByRole('button', { name: 'Build and save' }));
    for (let i = 0; i < 6; i++) {
      await app.user.tab();
      expect(dialog.contains(document.activeElement)).toBe(true);
    }
    await app.user.keyboard('{Escape}');
    expect(document.activeElement).toBe(opener);
  });
});
