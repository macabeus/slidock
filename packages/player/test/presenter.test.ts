// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { PRESENTER_CSS } from '../src/presenter/style';
import { presenterSession } from '../src/presenter/support';
import { THEME_STORAGE_KEY } from '../src/presenter/theme';
import { formatDuration } from '../src/presenter/view';
import { createPresenterWindow } from '../src/presenter/window';

beforeEach(() => {
  document.head.replaceChildren();
  document.body.replaceChildren();
  document.documentElement.removeAttribute('data-theme');
  localStorage.clear();
});

afterEach(() => {
  localStorage.clear();
});

function toggle(): HTMLButtonElement {
  const button = document.querySelector<HTMLButtonElement>('.sd-pw-theme');
  if (button === null) {
    throw new Error('no theme toggle');
  }
  return button;
}

describe('presenter window frame', () => {
  it('draws the header card over the main area, with its rules once per document', () => {
    const frame = createPresenterWindow(document.body);
    createPresenterWindow(document.body).destroy();
    expect(document.querySelectorAll('style[data-sd-presenter]')).toHaveLength(1);
    const root = document.querySelector('.sd-pw');
    expect(Array.from(root?.children ?? [], (child) => child.className)).toEqual(['sd-pw-header', 'sd-pw-main']);
    expect(root?.querySelector('.sd-pw-title')?.textContent).toBe('slidock');
    expect(root?.querySelector('.sd-pw-subtitle')?.textContent).toBe('Presenter view');
    expect(root?.querySelector('.sd-pw-mark svg')?.getAttribute('aria-hidden')).toBe('true');
    expect(frame.main.getAttribute('aria-label')).toBe('Presenter view');
    frame.setTitle('Talk');
    expect(root?.querySelector('.sd-pw-subtitle')?.textContent).toBe('Presenter view · Talk');
    frame.setTitle(null);
    expect(root?.querySelector('.sd-pw-subtitle')?.textContent).toBe('Presenter view');
    frame.destroy();
    expect(document.querySelector('.sd-pw')).toBeNull();
  });

  it('shows one status at a time before the presenter view', () => {
    const frame = createPresenterWindow(document.body);
    const view = frame.main.appendChild(document.createElement('div'));
    frame.setStatus({ kind: 'waiting', text: 'Waiting for the deck window…' });
    expect(frame.main.firstElementChild?.getAttribute('role')).toBe('status');
    expect(frame.main.textContent).toBe('Waiting for the deck window…');
    frame.setStatus({ kind: 'warning', title: 'Nothing to present here', text: 'Open it from a deck window.' });
    const banner = frame.main.querySelector('.sd-pw-banner');
    expect(banner?.getAttribute('role')).toBe('alert');
    expect(banner?.querySelector('.sd-pw-banner-title')?.textContent).toBe('Nothing to present here');
    expect(banner?.querySelector('.sd-pw-banner-text')?.textContent).toBe('Open it from a deck window.');
    expect(frame.main.querySelectorAll('[role]')).toHaveLength(1);
    frame.setStatus(null);
    expect(Array.from(frame.main.children)).toEqual([view]);
  });

  it('cycles the theme system → light → dark → system, remembered and set on <html>', () => {
    createPresenterWindow(document.body);
    const root = document.documentElement;
    const seen = () => ({
      label: toggle().getAttribute('aria-label'),
      title: toggle().getAttribute('title'),
      theme: root.getAttribute('data-theme'),
      stored: localStorage.getItem(THEME_STORAGE_KEY),
    });
    expect(seen()).toEqual({
      label: 'System theme. Switch to light theme',
      title: 'System theme (switch to light theme)',
      theme: null,
      stored: null,
    });
    toggle().click();
    expect(seen()).toEqual({
      label: 'Light theme. Switch to dark theme',
      title: 'Light theme (switch to dark theme)',
      theme: 'light',
      stored: 'light',
    });
    toggle().click();
    expect(seen()).toMatchObject({ label: 'Dark theme. Switch to system theme', theme: 'dark', stored: 'dark' });
    toggle().click();
    expect(seen()).toMatchObject({ label: 'System theme. Switch to light theme', theme: null, stored: null });
  });

  it('starts on the remembered theme', () => {
    localStorage.setItem(THEME_STORAGE_KEY, 'dark');
    createPresenterWindow(document.body);
    expect(document.documentElement.getAttribute('data-theme')).toBe('dark');
    expect(toggle().getAttribute('aria-label')).toBe('Dark theme. Switch to system theme');
  });

  it('ignores a remembered value that is not a theme', () => {
    localStorage.setItem(THEME_STORAGE_KEY, 'sepia');
    createPresenterWindow(document.body);
    expect(document.documentElement.hasAttribute('data-theme')).toBe(false);
  });

  it('has light tokens for a forced light theme and for the system preference', () => {
    expect(PRESENTER_CSS).toContain(':root[data-theme=light] .sd-pw{');
    expect(PRESENTER_CSS).toContain('@media (prefers-color-scheme:light){:root:not([data-theme]) .sd-pw{');
  });
});

describe('presenter session', () => {
  it.each([
    ['#presenter', ''],
    ['#presenter=abc', 'abc'],
    ['#presenter=a%20b', 'a b'],
    ['#presenter=%E0', ''],
    ['', null],
    ['#slide-2', null],
    ['#presenters', null],
  ])('%s → %s', (hash, session) => {
    expect(presenterSession(hash)).toBe(session);
  });
});

describe('timers', () => {
  it.each([
    [0, '0:00'],
    [59_999, '0:59'],
    [61_000, '1:01'],
    [3_600_000, '1:00:00'],
    [-5, '0:00'],
  ])('%d ms → %s', (ms, text) => {
    expect(formatDuration(ms)).toBe(text);
  });
});
