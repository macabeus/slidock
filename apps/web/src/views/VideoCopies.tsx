import type { PlannedVideo } from '@slidock/types';
import { Banner, Button, ProgressBar } from '@slidock/ui';
import { useCallback, useId, useState } from 'react';

import type { AlternatesSummary } from '../compile/alternates';
import type { VideosProgress } from '../compile/session';
import { formatBytes } from '../deck/download';
import { progressText } from './VideoCompression';

/** Where the choice is kept (`localStorage`). */
export const COPIES_CHOICE_KEY = 'slidock.download.copies';

/** Where only some browsers play them: Safari, and Chrome on a Mac, decode HEVC's alpha layer. */
export const WHERE_THEY_PLAY = 'Safari and Chrome on a Mac';

function storage(): Storage | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

/** Whether to add the copies: on unless the viewer turned it off. */
export function readCopiesChoice(from: Storage | null = storage()): boolean {
  try {
    return from?.getItem(COPIES_CHOICE_KEY) !== 'off';
  } catch {
    return true;
  }
}

export function writeCopiesChoice(on: boolean, to: Storage | null = storage()): void {
  try {
    to?.setItem(COPIES_CHOICE_KEY, on ? 'on' : 'off');
  } catch {
    // Not kept: the dialog still uses it until the page closes.
  }
}

export function useCopiesChoice(): [boolean, (on: boolean) => void] {
  const [on, setOn] = useState(() => readCopiesChoice());
  const update = useCallback((next: boolean) => {
    setOn(next);
    writeCopiesChoice(next);
  }, []);
  return [on, update];
}

function videos(count: number): string {
  return `${count} ${count === 1 ? 'video' : 'videos'}`;
}

export interface VideoCopiesProps {
  /** The deck's videos that get a copy. */
  planned: readonly PlannedVideo[];
  on: boolean;
  onChange(on: boolean): void;
  disabled: boolean;
  /** Whether this browser can make copies at all. */
  supported: boolean;
  /** While the copies are made. */
  progress?: VideosProgress | undefined;
  onCancel(): void;
}

/** The "transparent videos" switch: a WebM copy every browser plays, with its progress and a cancel. */
export function VideoCopies({ planned, on, onChange, disabled, supported, progress, onCancel }: VideoCopiesProps) {
  const title = useId();
  const hint = useId();
  const count = planned.length;
  return (
    <section aria-label="Videos with transparency" className="space-y-2">
      <label className="flex items-start gap-3 text-sm">
        <input
          type="checkbox"
          role="switch"
          checked={on && supported}
          disabled={disabled || !supported}
          aria-labelledby={title}
          aria-describedby={hint}
          onChange={(event) => onChange(event.currentTarget.checked)}
          className="mt-0.5 size-4 accent-[var(--brand-500)]"
        />
        <span>
          <span id={title} className="font-medium text-fg">
            Play transparent videos in every browser
          </span>
          <span id={hint} className="block text-muted">
            {videos(count)} with transparency {count === 1 ? 'plays' : 'play'} only in {WHERE_THEY_PLAY}.{' '}
            {supported
              ? `Add a WebM copy of ${count === 1 ? 'it' : 'each'} that Chrome, Edge and Firefox play everywhere. This browser makes it as it plays the video, which takes a while for long videos.`
              : 'This browser cannot make the copies: download the deck from Safari or Chrome on a Mac to add them.'}
          </span>
        </span>
      </label>
      {progress && (
        <div className="space-y-1">
          <ProgressBar
            label="Copying transparent videos"
            {...(progress.totalFrames > 0 ? { value: progress.frames, max: progress.totalFrames } : {})}
          />
          <div className="flex items-center justify-between gap-2">
            <p className="font-mono text-xs tabular-nums text-muted" aria-live="polite">
              {progressText(progress)}
            </p>
            <Button variant="ghost" onClick={onCancel}>
              Cancel
            </Button>
          </div>
        </div>
      )}
    </section>
  );
}

/**
 * What became of the copies, once the zip is built: those added, and a warning for each video left playing only
 * where its original plays, with why and how to fix it.
 */
export function AlternatesResult({
  copies,
  planned,
}: {
  /** `null` when the download made none (the switch was off). */
  copies: AlternatesSummary | null;
  planned: readonly PlannedVideo[];
}) {
  const without = copies
    ? copies.kept
    : planned.map((video) => ({ path: video.path, reason: 'no copy was asked for' }));
  return (
    <section aria-labelledby="zip-copies" className="space-y-2">
      <h3 id="zip-copies" className="text-[11px] font-semibold uppercase tracking-wider text-muted">
        Transparent videos
      </h3>
      {copies && copies.made > 0 && (
        <p className="text-sm text-fg-secondary">
          {copies.made} of {videos(copies.planned)} with a WebM copy for every browser (
          <span className="font-mono tabular-nums">{formatBytes(copies.bytes)}</span> added).
        </p>
      )}
      {without.length > 0 && (
        <Banner
          tone="warning"
          title={`${videos(without.length)} ${without.length === 1 ? 'plays' : 'play'} only in ${WHERE_THEY_PLAY}`}
        >
          <p className="text-xs">
            Chrome elsewhere, Edge and Firefox show {without.length === 1 ? 'its' : 'their'} poster instead. To add a
            copy every browser plays, download the deck again from Safari or Chrome on a Mac with “Play transparent
            videos in every browser” on.
          </p>
          <ul className="mt-1 space-y-0.5 font-mono text-xs">
            {without.map(({ path, reason }) => (
              <li key={path} className="break-words" title={path}>
                {path.replace(/^media\//, '').slice(0, 12)}…: {reason}
              </li>
            ))}
          </ul>
        </Banner>
      )}
    </section>
  );
}
