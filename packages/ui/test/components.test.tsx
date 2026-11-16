import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { useState } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { afterEach, describe, expect, it, vi } from 'vitest';

import {
  Button,
  Dialog,
  Drawer,
  ICON_NAMES,
  Icon,
  Pill,
  ProgressBar,
  type TabItem,
  Tabs,
  ThemeProvider,
  ThemeToggle,
  focusRing,
  openOverlayCount,
} from '../src';

afterEach(() => {
  cleanup();
  document.documentElement.removeAttribute('data-theme');
  localStorage.clear();
});

describe('ProgressBar', () => {
  it('is determinate with value and indeterminate without', () => {
    const determinate = renderToStaticMarkup(<ProgressBar label="Bundling" value={3} max={4} />);
    expect(determinate).toContain('role="progressbar"');
    expect(determinate).toContain('aria-valuenow="3"');
    expect(determinate).toContain('aria-valuemax="4"');
    expect(determinate).toContain('width:75%');
    const indeterminate = renderToStaticMarkup(<ProgressBar label="Reading" />);
    expect(indeterminate).toContain('aria-label="Reading"');
    expect(indeterminate).not.toContain('aria-valuenow');
    expect(indeterminate).toContain('motion-safe:animate-indeterminate');
  });
});

describe('Pill', () => {
  it('washes its tint', () => {
    const html = renderToStaticMarkup(<Pill tint="var(--font-google)">Google</Pill>);
    expect(html).toContain('color:var(--font-google)');
    expect(html).toContain('color-mix(in srgb, var(--font-google) 13%, transparent)');
  });
});

describe('Icon', () => {
  it('is decorative without a label and an image with one', () => {
    for (const name of ICON_NAMES) {
      expect(renderToStaticMarkup(<Icon name={name} />)).toContain('aria-hidden="true"');
    }
    expect(renderToStaticMarkup(<Icon name="x" label="Close" />)).toContain('role="img"');
  });
});

describe('Tabs', () => {
  type Id = 'a' | 'b' | 'c';
  const items: TabItem<Id>[] = [
    { id: 'a', label: 'A' },
    { id: 'b', label: 'B', count: 2 },
    { id: 'c', label: 'C' },
  ];

  function Harness() {
    const [value, setValue] = useState<Id>('a');
    return <Tabs<Id> label="Sections" idPrefix="t" value={value} onChange={setValue} items={items} />;
  }

  it('moves selection and focus with arrow keys, Home and End', () => {
    render(<Harness />);
    const a = screen.getByRole('tab', { name: 'A' });
    expect(a.getAttribute('aria-selected')).toBe('true');
    expect(a.tabIndex).toBe(0);
    fireEvent.keyDown(a, { key: 'ArrowLeft' });
    const c = screen.getByRole('tab', { name: 'C' });
    expect(c.getAttribute('aria-selected')).toBe('true');
    expect(document.activeElement).toBe(c);
    fireEvent.keyDown(c, { key: 'ArrowRight' });
    expect(screen.getByRole('tab', { name: 'A' }).getAttribute('aria-selected')).toBe('true');
    fireEvent.keyDown(a, { key: 'End' });
    expect(document.activeElement).toBe(c);
    expect(screen.getByRole('tab', { name: /^B/ }).getAttribute('aria-controls')).toBe('t-panel-b');
  });

  it('names only the panels in the document', () => {
    const controls = () => screen.getAllByRole('tab').map((tab) => tab.getAttribute('aria-controls'));
    const { rerender } = render(
      <Tabs<Id> label="Sections" idPrefix="t" value="b" onChange={() => {}} items={items} panels="selected" />,
    );
    expect(controls()).toEqual([null, 't-panel-b', null]);
    rerender(<Tabs<Id> label="Sections" idPrefix="t" value="b" onChange={() => {}} items={items} panels="none" />);
    expect(controls()).toEqual([null, null, null]);
  });
});

describe('overlays', () => {
  it('Escape closes only the innermost overlay, and focus returns to the opener', () => {
    const outerClose = vi.fn();
    function Harness() {
      const [outer, setOuter] = useState(false);
      const [inner, setInner] = useState(false);
      return (
        <>
          <button onClick={() => setOuter(true)}>open</button>
          <Drawer
            open={outer}
            onClose={() => {
              outerClose();
              setOuter(false);
            }}
            title="Outer"
          >
            <button onClick={() => setInner(true)}>inner</button>
          </Drawer>
          <Dialog open={inner} onClose={() => setInner(false)} title="Inner">
            <button data-autofocus>ok</button>
          </Dialog>
        </>
      );
    }
    render(<Harness />);
    const opener = screen.getByRole('button', { name: 'open' });
    opener.focus();
    fireEvent.click(opener);
    expect(screen.getByRole('dialog', { name: 'Outer' })).toBeTruthy();
    expect(document.body.style.overflow).toBe('hidden');
    fireEvent.click(screen.getByRole('button', { name: 'inner' }));
    expect(document.activeElement).toBe(screen.getByRole('button', { name: 'ok' }));
    expect(openOverlayCount()).toBe(2);

    act(() => {
      fireEvent.keyDown(document, { key: 'Escape' });
    });
    expect(screen.queryByRole('dialog', { name: 'Inner' })).toBeNull();
    expect(screen.getByRole('dialog', { name: 'Outer' })).toBeTruthy();
    expect(outerClose).not.toHaveBeenCalled();

    act(() => {
      fireEvent.keyDown(document, { key: 'Escape' });
    });
    expect(outerClose).toHaveBeenCalledTimes(1);
    expect(openOverlayCount()).toBe(0);
    expect(document.body.style.overflow).toBe('');
    expect(document.activeElement).toBe(opener);
  });

  it('Tab cycles inside the dialog', () => {
    render(
      <Dialog open onClose={() => {}} title="Cycle">
        <button>one</button>
        <button>two</button>
      </Dialog>,
    );
    const close = screen.getByRole('button', { name: 'Close' });
    const two = screen.getByRole('button', { name: 'two' });
    two.focus();
    fireEvent.keyDown(document, { key: 'Tab' });
    expect(document.activeElement).toBe(close);
    fireEvent.keyDown(document, { key: 'Tab', shiftKey: true });
    expect(document.activeElement).toBe(two);
  });

  it('a focus the Tab key moves asks for visible focus, after a click as after a key', () => {
    render(
      <Dialog open onClose={() => {}} title="Cycle">
        <button>one</button>
      </Dialog>,
    );
    const focus = vi.spyOn(HTMLElement.prototype, 'focus');
    try {
      (document.activeElement as HTMLElement | null)?.blur();
      fireEvent.keyDown(document, { key: 'Tab' });
      fireEvent.keyDown(document, { key: 'Tab', shiftKey: true });
      expect(focus.mock.calls).toEqual([[{ focusVisible: true }], [{ focusVisible: true }]]);
    } finally {
      focus.mockRestore();
    }
  });
});

describe('focus indicator', () => {
  // A box shadow is not drawn on a native-appearance <select> in WebKit, nor kept in forced-colors mode.
  it('is an outline on every control that draws its own, never a box-shadow ring', () => {
    const markup = renderToStaticMarkup(
      <ThemeProvider>
        <Button>go</Button>
        <ThemeToggle />
        <Drawer open={false} onClose={() => {}} title="T">
          body
        </Drawer>
      </ThemeProvider>,
    );
    expect(focusRing).toContain('focus-visible:outline-2');
    expect(`${focusRing} ${markup}`).not.toMatch(/focus(?:-visible)?:(?:ring|outline-none)/);
  });

  it('is drawn on the close button of an open dialog', () => {
    render(
      <Dialog open onClose={() => {}} title="Ring">
        <button>one</button>
      </Dialog>,
    );
    expect(screen.getByRole('button', { name: 'Close' }).className).toContain('focus-visible:outline-2');
  });
});

describe('ThemeToggle', () => {
  it('cycles system → light → dark, setting data-theme and remembering the choice', () => {
    render(
      <ThemeProvider>
        <ThemeToggle />
      </ThemeProvider>,
    );
    const button = () => screen.getByRole('button');
    expect(button().getAttribute('aria-label')).toMatch(/^System theme/);
    expect(document.documentElement.hasAttribute('data-theme')).toBe(false);
    fireEvent.click(button());
    expect(document.documentElement.getAttribute('data-theme')).toBe('light');
    expect(localStorage.getItem('slidock.theme')).toBe('light');
    fireEvent.click(button());
    expect(document.documentElement.getAttribute('data-theme')).toBe('dark');
    fireEvent.click(button());
    expect(document.documentElement.hasAttribute('data-theme')).toBe(false);
    expect(localStorage.getItem('slidock.theme')).toBeNull();
  });
});
