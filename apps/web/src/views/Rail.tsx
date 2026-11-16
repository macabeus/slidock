import type { Deck } from '@slidock/types';
import { Icon, cx, focusRing } from '@slidock/ui';
import { type KeyboardEvent, useEffect, useRef, useState } from 'react';

import type { Player } from '../player/types';

function Thumbnail({ player, slideId, deck }: { player: Player | null; slideId: string; deck: Deck }) {
  const ref = useRef<HTMLDivElement>(null);
  const [visible, setVisible] = useState(() => typeof IntersectionObserver !== 'function');

  useEffect(() => {
    const node = ref.current;
    if (visible || !node) {
      return;
    }
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          setVisible(true);
          observer.disconnect();
        }
      },
      { rootMargin: '200px' },
    );
    observer.observe(node);
    return () => observer.disconnect();
  }, [visible]);

  useEffect(() => {
    const node = ref.current;
    if (!visible || !player || !node) {
      return;
    }
    return player.renderStatic(slideId, node, 'contain');
  }, [visible, player, slideId]);

  return (
    <div
      ref={ref}
      data-thumbnail={slideId}
      className="relative w-full overflow-hidden bg-sunken"
      style={{ aspectRatio: `${deck.size.w} / ${deck.size.h}` }}
    />
  );
}

export interface RailProps {
  deck: Deck;
  player: Player | null;
  current: number;
  onSelect(index: number): void;
}

/** The thumbnail rail: vertical beside the stage on wide screens, a horizontal strip under it on narrow ones. */
export function Rail({ deck, player, current, onSelect }: RailProps) {
  const buttons = useRef<(HTMLButtonElement | null)[]>([]);

  useEffect(() => {
    buttons.current[current]?.scrollIntoView?.({ block: 'nearest', inline: 'nearest' });
  }, [current]);

  const onKeyDown = (event: KeyboardEvent<HTMLOListElement>) => {
    const delta = event.key === 'ArrowDown' ? 1 : event.key === 'ArrowUp' ? -1 : 0;
    if (delta === 0) {
      return;
    }
    const focused = buttons.current.findIndex((button) => button === document.activeElement);
    const target = Math.min(deck.slides.length - 1, Math.max(0, (focused < 0 ? current : focused) + delta));
    event.preventDefault();
    buttons.current[target]?.focus();
    onSelect(target);
  };

  return (
    <nav aria-label="Slides" className="min-h-0 min-w-0 lg:overflow-y-auto">
      <ol
        className="flex gap-3 overflow-x-auto p-1 pb-2 lg:flex-col lg:overflow-x-visible lg:pb-1"
        onKeyDown={onKeyDown}
      >
        {deck.slides.map((slide, index) => {
          const active = index === current;
          const label = `${slide.hidden ? 'Hidden slide' : `Slide ${slide.number ?? index + 1}`}${slide.title ? `: ${slide.title}` : ''}`;
          return (
            <li key={slide.id} className="w-36 shrink-0 lg:w-full">
              <button
                ref={(node) => {
                  buttons.current[index] = node;
                }}
                type="button"
                aria-label={label}
                aria-current={active ? 'true' : undefined}
                tabIndex={active ? 0 : -1}
                onClick={() => onSelect(index)}
                className={cx(
                  'group relative block w-full overflow-hidden rounded-md ring-1 transition-shadow',
                  active ? 'ring-2 ring-brand-400' : 'ring-line hover:ring-muted',
                  slide.hidden && 'opacity-50',
                  focusRing,
                )}
              >
                <Thumbnail player={player} slideId={slide.id} deck={deck} />
                <span className="absolute bottom-1 left-1 flex items-center gap-1 rounded bg-root/80 px-1.5 font-mono text-[11px] tabular-nums text-fg-secondary">
                  {slide.hidden ? <Icon name="eye-off" className="size-3" /> : (slide.number ?? index + 1)}
                </span>
              </button>
            </li>
          );
        })}
      </ol>
    </nav>
  );
}
