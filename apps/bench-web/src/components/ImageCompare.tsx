// The row drawer's image comparison: side by side with synchronized pan and zoom,
// a slider between the oracle and the render, an onion skin, a client-side difference, and the heatmap over the
// render. A missing image shows the command that produces it, or why a report does not hold it.
import { cx } from '@slidock/ui';
import { type PointerEvent, type ReactNode, useEffect, useRef, useState } from 'react';

import type { ImageMode } from '../state/url';
import { CopyCommand, SegmentedControl } from './Controls';

export interface Pane {
  id: string;
  label: string;
  /** The image URL, or null when the bench has none. */
  src: string | null;
  /** Where the image came from. */
  caption?: string | undefined;
  /** The command that produces a missing image. */
  missing?: string | undefined;
  /** Why a missing image is not there, where no command produces it (a published report). */
  note?: string | undefined;
}

export const MODE_OPTIONS: { value: ImageMode; label: string; title: string }[] = [
  { value: 'side', label: 'Side by side', title: 'Oracle, render and heatmap next to each other' },
  { value: 'slider', label: 'Slider', title: 'A divider between the oracle and the render' },
  { value: 'onion', label: 'Onion skin', title: 'The render over the oracle, with an opacity slider' },
  { value: 'difference', label: 'Difference', title: 'The per-pixel difference of the oracle and the render' },
  { value: 'heatmap', label: 'Heatmap', title: 'The SSIM heatmap over the render' },
];

interface View {
  scale: number;
  x: number;
  y: number;
}

const HOME: View = { scale: 1, x: 0, y: 0 };

function Missing({ pane }: { pane: Pane }) {
  return (
    <div className="flex aspect-video flex-col items-center justify-center gap-2 rounded-md border border-dashed border-line p-3 text-center">
      <p className="text-xs text-muted">No {pane.label.toLowerCase()} image</p>
      {pane.note && <p className="text-xs text-muted">{pane.note}</p>}
      {pane.missing && <CopyCommand command={pane.missing} label={`command that produces the ${pane.label}`} />}
    </div>
  );
}

function Frame({ label, caption, children }: { label: string; caption?: string | undefined; children: ReactNode }) {
  return (
    <figure className="min-w-0">
      {children}
      <figcaption className="mt-1.5 flex items-baseline justify-between gap-2 text-xs">
        <span className="font-semibold text-fg-secondary">{label}</span>
        {caption && <span className="truncate text-muted">{caption}</span>}
      </figcaption>
    </figure>
  );
}

/** Side by side: one pan and zoom shared by every pane (wheel to zoom, drag to pan, double-click to reset). */
function SideBySide({ panes }: { panes: readonly Pane[] }) {
  const [view, setView] = useState<View>(HOME);
  const [readout, setReadout] = useState<string | null>(null);
  const drag = useRef<{ x: number; y: number; view: View } | null>(null);
  const grid = useRef<HTMLDivElement>(null);

  // A native listener: React's wheel listeners are passive, and zooming must not scroll the drawer.
  useEffect(() => {
    const node = grid.current;
    if (!node) {
      return;
    }
    const onWheel = (event: globalThis.WheelEvent) => {
      const pane = event.target instanceof Element ? event.target.closest('[data-pane]') : null;
      if (!pane) {
        return;
      }
      event.preventDefault();
      const box = pane.getBoundingClientRect();
      const px = event.clientX - box.left;
      const py = event.clientY - box.top;
      setView((current) => {
        const scale = Math.min(16, Math.max(1, current.scale * (event.deltaY < 0 ? 1.25 : 0.8)));
        if (scale === 1) {
          return HOME;
        }
        const ratio = scale / current.scale;
        return { scale, x: px - (px - current.x) * ratio, y: py - (py - current.y) * ratio };
      });
    };
    node.addEventListener('wheel', onWheel, { passive: false });
    return () => node.removeEventListener('wheel', onWheel);
  }, []);

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    drag.current = { x: event.clientX, y: event.clientY, view };
    event.currentTarget.setPointerCapture?.(event.pointerId);
  };
  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const box = event.currentTarget.getBoundingClientRect();
    const image = event.currentTarget.querySelector('img');
    if (image && image.naturalWidth > 0 && box.width > 0) {
      const k = image.naturalWidth / box.width;
      const x = Math.floor(((event.clientX - box.left - view.x) / view.scale) * k);
      const y = Math.floor(((event.clientY - box.top - view.y) / view.scale) * k);
      setReadout(x >= 0 && y >= 0 && x < image.naturalWidth && y < image.naturalHeight ? `${x}, ${y} px` : null);
    }
    const start = drag.current;
    if (start) {
      setView({ ...start.view, x: start.view.x + event.clientX - start.x, y: start.view.y + event.clientY - start.y });
    }
  };
  const onPointerUp = () => {
    drag.current = null;
  };

  return (
    <div>
      <div ref={grid} className={cx('grid grid-cols-1 gap-3', panes.length >= 4 ? 'lg:grid-cols-4' : 'lg:grid-cols-3')}>
        {panes.map((pane) => (
          <Frame key={pane.id} label={pane.label} caption={pane.caption}>
            {pane.src === null ? (
              <Missing pane={pane} />
            ) : (
              <div
                data-pane={pane.id}
                className="relative aspect-video cursor-grab touch-none overflow-hidden rounded-md bg-sunken active:cursor-grabbing"
                onPointerDown={onPointerDown}
                onPointerMove={onPointerMove}
                onPointerUp={onPointerUp}
                onPointerLeave={() => setReadout(null)}
                onDoubleClick={() => setView(HOME)}
              >
                <img
                  src={pane.src}
                  alt={pane.label}
                  draggable={false}
                  className="size-full origin-top-left object-contain select-none"
                  style={{ transform: `translate(${view.x}px, ${view.y}px) scale(${view.scale})` }}
                />
              </div>
            )}
          </Frame>
        ))}
      </div>
      <p className="mt-2 flex justify-between font-mono text-[11px] text-muted">
        <span>
          {view.scale === 1 ? 'Wheel to zoom, drag to pan' : `${Math.round(view.scale * 100)}% · double-click resets`}
        </span>
        <span>{readout ?? ''}</span>
      </p>
    </div>
  );
}

function Range({ label, value, onChange }: { label: string; value: number; onChange(value: number): void }) {
  return (
    <label className="flex items-center gap-3 text-xs text-muted">
      <span className="shrink-0">{label}</span>
      <input
        type="range"
        min={0}
        max={100}
        value={value}
        onChange={(event) => onChange(Number(event.currentTarget.value))}
        className="w-full accent-[var(--brand-400)]"
      />
      <span className="w-10 shrink-0 text-right font-mono tabular-nums">{value}%</span>
    </label>
  );
}

/** Two images stacked: `top` drawn over `bottom` with the given opacity or clipped at the divider. */
function Stack({ bottom, top, opacity = 1, divider }: { bottom: Pane; top: Pane; opacity?: number; divider?: number }) {
  if (bottom.src === null || top.src === null) {
    return (
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
        {[bottom, top].map((pane) =>
          pane.src === null ? (
            <Frame key={pane.id} label={pane.label}>
              <Missing pane={pane} />
            </Frame>
          ) : null,
        )}
      </div>
    );
  }
  return (
    <div className="relative mx-auto aspect-video max-w-4xl overflow-hidden rounded-md bg-sunken">
      <img src={bottom.src} alt={bottom.label} className="absolute inset-0 size-full object-contain" />
      <img
        src={top.src}
        alt={top.label}
        className="absolute inset-0 size-full object-contain"
        style={{
          opacity,
          ...(divider === undefined ? {} : { clipPath: `inset(0 0 0 ${divider}%)` }),
        }}
      />
      {divider !== undefined && (
        <div aria-hidden className="absolute inset-y-0 w-0.5 bg-brand-400 shadow" style={{ left: `${divider}%` }} />
      )}
      <span className="absolute left-2 top-2 rounded bg-root/70 px-1.5 text-[11px] text-fg-secondary">
        {bottom.label}
      </span>
      <span className="absolute right-2 top-2 rounded bg-root/70 px-1.5 text-[11px] text-fg-secondary">
        {top.label}
      </span>
    </div>
  );
}

function loadImage(src: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const image = new Image();
    image.onload = () => resolve(image);
    image.onerror = () => reject(new Error(`could not load ${src}`));
    image.src = src;
  });
}

/** `|oracle − render|` per channel, drawn by the canvas's `difference` compositing; black where they agree. */
function Difference({ oracle, render }: { oracle: Pane; render: Pane }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    const node = canvas.current;
    if (!node || oracle.src === null || render.src === null) {
      return;
    }
    let live = true;
    Promise.all([loadImage(oracle.src), loadImage(render.src)]).then(
      ([a, b]) => {
        const context = node.getContext('2d');
        if (!live) {
          return;
        }
        if (!context) {
          setFailed(true);
          return;
        }
        node.width = a.naturalWidth;
        node.height = a.naturalHeight;
        context.globalCompositeOperation = 'source-over';
        context.drawImage(a, 0, 0);
        context.globalCompositeOperation = 'difference';
        context.drawImage(b, 0, 0, a.naturalWidth, a.naturalHeight);
        setFailed(false);
      },
      () => live && setFailed(true),
    );
    return () => {
      live = false;
    };
  }, [oracle.src, render.src]);
  if (oracle.src === null || render.src === null) {
    return <Stack bottom={oracle} top={render} />;
  }
  return (
    <figure className="mx-auto max-w-4xl">
      <canvas
        ref={canvas}
        role="img"
        aria-label="Difference of the oracle and the render"
        className="aspect-video w-full rounded-md bg-sunken"
      />
      <figcaption className="mt-1.5 text-xs text-muted">
        {failed
          ? 'The difference could not be drawn in this browser.'
          : 'Black where the render equals the oracle; brighter where they differ.'}
      </figcaption>
    </figure>
  );
}

export interface ImageCompareProps {
  oracle: Pane;
  render: Pane;
  heatmap: Pane;
  /** More panes for side by side (the golden render, a base run's render). */
  extra?: readonly Pane[];
  mode: ImageMode;
  onMode(mode: ImageMode): void;
}

export function ImageCompare({ oracle, render, heatmap, extra = [], mode, onMode }: ImageCompareProps) {
  const [divider, setDivider] = useState(50);
  const [opacity, setOpacity] = useState(50);
  const [alpha, setAlpha] = useState(70);
  return (
    <section aria-label="Images" className="space-y-3">
      <SegmentedControl label="Compare mode" options={MODE_OPTIONS} value={mode} onChange={onMode} />
      {mode === 'side' && <SideBySide panes={[oracle, render, heatmap, ...extra]} />}
      {mode === 'slider' && (
        <div className="space-y-2">
          <Stack bottom={oracle} top={render} divider={divider} />
          <Range label="Divider" value={divider} onChange={setDivider} />
        </div>
      )}
      {mode === 'onion' && (
        <div className="space-y-2">
          <Stack bottom={oracle} top={render} opacity={opacity / 100} />
          <Range label="Render opacity" value={opacity} onChange={setOpacity} />
        </div>
      )}
      {mode === 'difference' && <Difference oracle={oracle} render={render} />}
      {mode === 'heatmap' && (
        <div className="space-y-2">
          <Stack bottom={render} top={heatmap} opacity={alpha / 100} />
          <Range label="Heatmap opacity" value={alpha} onChange={setAlpha} />
        </div>
      )}
    </section>
  );
}
