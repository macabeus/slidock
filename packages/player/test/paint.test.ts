// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest';

import { PAINT_CSS, holdPaint, hookProperty, linkPaint, paintRuns, unlinkPaints } from '../src/paint';

describe('paint: the deck hooks a change plays through', () => {
  it('finds the property that reads a hook', () => {
    expect(hookProperty('position:absolute;background:var(--sd-fill,#7c3aed)', '--sd-fill')).toBe('background');
    expect(hookProperty('fill: var(--sd-fill, var(--sd-color-accent1,#4472c4)); stroke:#000', '--sd-fill')).toBe(
      'fill',
    );
    expect(hookProperty('border:9pt solid var(--sd-line,#111827)', '--sd-line')).toBe('border');
    expect(hookProperty('Stroke:VAR(--sd-line,#000)', '--sd-line')).toBe('stroke');
    // Another variable with the hook's name as a prefix is not the hook.
    expect(hookProperty('fill:var(--sd-fill2,#000)', '--sd-fill')).toBeNull();
    expect(hookProperty('background:#fff', '--sd-fill')).toBeNull();
    expect(hookProperty(`${'a:b;'.repeat(2000)}fill:var(--sd-fill,#000)`, '--sd-fill')).toBeNull();
  });

  it('plays a font switch on a target that reads its hook, and links and unlinks them', () => {
    document.body.innerHTML =
      '<div id="t"><p style="font-weight:var(--sd-bold,400)">x</p><svg><path style="fill:var(--sd-fill,#000)"></path></svg></div>';
    const target = document.getElementById('t') as Element;
    expect(paintRuns(target, 'bold')).toEqual([{ run: target, from: 'normal' }]);
    // No element under the target reads the hook: the switch has nothing to play on.
    expect(paintRuns(target, 'italic')).toEqual([]);
    expect(paintRuns(target, 'underline')).toEqual([]);
    linkPaint(target, 'fill', '#000000');
    linkPaint(target, 'bold', 'normal');
    linkPaint(target, 'fill', '#000000');
    holdPaint(target, 'italic', 'italic');
    holdPaint(target, 'italic', 'italic');
    expect(target.getAttribute('data-sd-paint')).toBe('fill bold');
    expect(target.getAttribute('data-sd-hold')).toBe('italic');
    expect((target as HTMLElement).style.getPropertyValue('--sd-held-italic')).toBe('italic');
    unlinkPaints(document.body, []);
    expect(target.hasAttribute('data-sd-paint')).toBe(false);
    expect(target.hasAttribute('data-sd-hold')).toBe(false);
    expect((target as HTMLElement).style.cssText).toBe('');
    // A switch starts from the font the hook's reader shows now.
    document.body.innerHTML =
      '<div id="t"><p style="--r:var(--sd-bold) var(--sd-italic) var(--sd-underline);font-weight:700;' +
      'font-style:italic;text-decoration-line:underline">x</p></div>';
    const shown = document.getElementById('t') as Element;
    expect(['bold', 'italic', 'underline'].map((p) => paintRuns(shown, p as 'bold')[0]?.from)).toEqual([
      'bold',
      'italic',
      'underline',
    ]);
  });

  it('links each hook to its registered property, and text through the recolouring rule', () => {
    expect(PAINT_CSS).toContain('[data-sd-paint~=fill],[data-sd-hold~=fill]{--sd-fill:var(--sd-anim-fill)!important}');
    expect(PAINT_CSS).toContain(
      '[data-sd-paint~=underline],[data-sd-hold~=underline]{--sd-underline:var(--sd-anim-underline)!important}',
    );
    expect(PAINT_CSS).toContain(
      '[data-sd-paint~=text],[data-sd-hold~=text],[data-sd-paint~=text] *,[data-sd-hold~=text] *{color:var(--sd-anim-color)!important}',
    );
    // A held paint is a plain declaration of the unregistered `--sd-held-*`, read as a link reads its animation.
    expect(PAINT_CSS).toContain('[data-sd-hold~=bold]{--sd-anim-bold:var(--sd-held-bold)}');
    // A linked target repaints whole when its fonts switch (e2e/settled.spec.ts); a held one is no stacking context.
    expect(PAINT_CSS).toContain('[data-sd-paint]{isolation:isolate}');
    expect(PAINT_CSS).not.toContain('[data-sd-hold]{');
  });
});
