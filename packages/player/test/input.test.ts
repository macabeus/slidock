// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest';

import {
  type KeyContext,
  LINK_ACTIONS,
  activates,
  clickZone,
  isActivatable,
  isEditable,
  keyCommand,
  linkTarget,
} from '../src/input';

function key(k: string, mods: Partial<{ ctrlKey: boolean; metaKey: boolean; altKey: boolean }> = {}) {
  return { key: k, ctrlKey: false, metaKey: false, altKey: false, defaultPrevented: false, ...mods };
}

const presenting: KeyContext = { presenting: true, digits: '', helpOpen: false, presenter: true };
const viewing: KeyContext = { presenting: false, digits: '', helpOpen: false, presenter: true };

describe('navigation keys', () => {
  it.each([
    ['ArrowRight', 'next'],
    ['ArrowDown', 'next'],
    ['PageDown', 'next'],
    [' ', 'next'],
    ['Enter', 'next'],
    ['ArrowLeft', 'previous'],
    ['ArrowUp', 'previous'],
    ['PageUp', 'previous'],
    ['Backspace', 'previous'],
    ['Home', 'first'],
    ['End', 'last'],
    ['f', 'fullscreen'],
    ['?', 'help'],
    ['P', 'presenter'],
    ['F5', 'present'],
    ['Escape', 'stop'],
  ])('%s → %s while presenting', (k, type) => {
    expect(keyCommand(key(k), presenting).command?.type).toBe(type);
  });

  it("P is not the player's when it offers no presenter view", () => {
    for (const context of [presenting, viewing]) {
      expect(keyCommand(key('p'), { ...context, presenter: false })).toEqual({
        command: null,
        digits: '',
        handled: false,
      });
      expect(keyCommand(key('p'), context).command?.type).toBe('presenter');
    }
  });

  it('blanks with B and . (black) and W and , (white)', () => {
    for (const k of ['b', 'B', '.']) {
      expect(keyCommand(key(k), presenting).command).toEqual({ type: 'blank', kind: 'black' });
    }
    for (const k of ['w', 'W', ',']) {
      expect(keyCommand(key(k), presenting).command).toEqual({ type: 'blank', kind: 'white' });
    }
  });

  it('Space, Enter and Escape act only while presenting', () => {
    for (const k of [' ', 'Enter', 'Escape']) {
      expect(keyCommand(key(k), viewing)).toEqual({ command: null, digits: '', handled: false });
    }
    expect(keyCommand(key('ArrowRight'), viewing).command?.type).toBe('next');
  });

  it('a number then Enter goes to that slide', () => {
    let context = presenting;
    for (const k of ['1', '2']) {
      const result = keyCommand(key(k), context);
      expect(result).toEqual({ command: null, digits: context.digits + k, handled: true });
      context = { ...context, digits: result.digits };
    }
    expect(keyCommand(key('Enter'), context)).toEqual({
      command: { type: 'goToNumber', n: 12 },
      digits: '',
      handled: true,
    });
  });

  it('Escape clears typed digits before stopping, and closes help first', () => {
    expect(keyCommand(key('Escape'), { ...presenting, digits: '4' })).toEqual({
      command: null,
      digits: '',
      handled: true,
    });
    expect(keyCommand(key('Escape'), { ...presenting, helpOpen: true }).command?.type).toBe('help');
  });

  it('while the help is open only its own keys work, closing it', () => {
    const open = { ...presenting, helpOpen: true };
    for (const k of ['Escape', '?']) {
      expect(keyCommand(key(k), open).command?.type).toBe('help');
    }
    for (const k of ['ArrowRight', ' ', 'PageUp', 'End', 'b', 'p', '3', 'Enter']) {
      expect(keyCommand(key(k), open), k).toEqual({ command: null, digits: '', handled: false });
    }
  });

  it('another key drops typed digits', () => {
    expect(keyCommand(key('ArrowRight'), { ...presenting, digits: '3' }).digits).toBe('');
  });

  it('ignores keys with a modifier or a prevented default', () => {
    for (const mods of [{ ctrlKey: true }, { metaKey: true }, { altKey: true }]) {
      expect(keyCommand(key('ArrowRight', mods), presenting).handled).toBe(false);
    }
    expect(keyCommand({ ...key('ArrowRight'), defaultPrevented: true }, presenting).handled).toBe(false);
    expect(keyCommand(key('x'), presenting).handled).toBe(false);
  });

  it('knows editable targets', () => {
    const input = document.createElement('input');
    const div = document.createElement('div');
    const editable = document.createElement('div');
    editable.setAttribute('contenteditable', 'true');
    expect(isEditable(input)).toBe(true);
    expect(isEditable(editable)).toBe(true);
    expect(isEditable(div)).toBe(false);
    expect(isEditable(null)).toBe(false);
  });
});

describe('pointer and links', () => {
  it('the left third goes back', () => {
    expect(clickZone(0, 900)).toBe('previous');
    expect(clickZone(299, 900)).toBe('previous');
    expect(clickZone(300, 900)).toBe('next');
    expect(clickZone(899, 900)).toBe('next');
  });

  it('reads slide links and external links', () => {
    expect(linkTarget('#slide=pp-3')).toEqual({ type: 'slide', id: 'pp-3' });
    expect(linkTarget('#4')).toEqual({ type: 'number', n: 4 });
    expect(linkTarget('#slide-12')).toEqual({ type: 'number', n: 12 });
    expect(linkTarget('#slide-x')).toBeNull();
    expect(linkTarget('#sd-text')).toBeNull();
    expect(linkTarget('https://example.com/x')).toEqual({ type: 'external', url: 'https://example.com/x' });
    expect(linkTarget('mailto:a@b.c')).toEqual({ type: 'external', url: 'mailto:a@b.c' });
    expect(linkTarget('javascript:alert(1)')).toBeNull();
    expect(linkTarget('#other')).toBeNull();
  });
});

describe('link actions', () => {
  it('maps the four actions a slide link names, and nothing inherited', () => {
    expect([...LINK_ACTIONS]).toEqual([
      ['action:next', 'next'],
      ['action:prev', 'previous'],
      ['action:first', 'first'],
      ['action:last', 'last'],
    ]);
    expect(LINK_ACTIONS.get('action:constructor')).toBeUndefined();
    expect(LINK_ACTIONS.get('constructor')).toBeUndefined();
  });
});

describe('keys aimed at a link or a button', () => {
  it('leaves Enter and Space to what they activate', () => {
    document.body.innerHTML =
      '<a id="link" href="#sd-text"><span id="inner">Text</span></a><a id="bare">x</a><button id="button">B</button>' +
      '<details><summary id="summary">S</summary></details><div id="div" tabindex="0">d</div>';
    const byId = (id: string) => document.getElementById(id);
    expect(isActivatable(byId('link'))).toBe(true);
    expect(isActivatable(byId('inner'))).toBe(true);
    expect(isActivatable(byId('button'))).toBe(true);
    expect(isActivatable(byId('summary'))).toBe(true);
    expect(isActivatable(byId('bare'))).toBe(false);
    expect(isActivatable(byId('div'))).toBe(false);
    expect(isActivatable(null)).toBe(false);
    expect(['Enter', ' ', 'Spacebar'].every(activates)).toBe(true);
    expect(['ArrowRight', 'PageDown', 'Escape'].some(activates)).toBe(false);
  });
});
