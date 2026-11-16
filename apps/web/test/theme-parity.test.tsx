import { createPresenterWindow } from '@slidock/player/presenter';
import { ThemeProvider, ThemeToggle } from '@slidock/ui';
import { act, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';

afterEach(() => {
  localStorage.clear();
  document.documentElement.removeAttribute('data-theme');
  document.body.replaceChildren();
});

/** Each state a theme toggle goes through in four clicks: its label, `data-theme` and what it remembers. */
function cycle(toggle: () => HTMLElement) {
  const states = [];
  for (let i = 0; i < 4; i++) {
    states.push({
      label: toggle().getAttribute('aria-label'),
      title: toggle().getAttribute('title'),
      theme: document.documentElement.getAttribute('data-theme'),
      stored: Object.fromEntries(Object.entries(localStorage)),
    });
    act(() => toggle().click());
  }
  return states;
}

describe("the presenter window's theme toggle", () => {
  it("follows the app's: the same labels, the same attribute and the same remembered choice", () => {
    const app = render(
      <ThemeProvider>
        <ThemeToggle />
      </ThemeProvider>,
    );
    const appStates = cycle(() => screen.getByRole('button'));
    app.unmount();
    localStorage.clear();
    document.documentElement.removeAttribute('data-theme');

    createPresenterWindow(document.body);
    const frameStates = cycle(() => document.querySelector<HTMLElement>('.sd-pw-theme') as HTMLElement);
    expect(frameStates).toEqual(appStates);
    expect(appStates.map((state) => state.theme)).toEqual([null, 'light', 'dark', null]);
  });
});
