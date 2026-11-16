import { Button, ProgressBar } from '@slidock/ui';
import { useId } from 'react';

import type { ImagesSummary, VideosProgress, VideosSummary } from '../compile/session';
import { formatBytes } from '../deck/download';
import { VIDEO_QUALITIES, type VideoQuality } from '../media/video';
import { type CompressionChoice, readChoice, saved, useChoice, writeChoice, zipWithout } from './ImageCompression';

/** The download dialog's video choice, which the viewer's browser remembers. */
export type VideoChoice = CompressionChoice<VideoQuality>;

export const DEFAULT_VIDEO_CHOICE: VideoChoice = { compress: false, quality: 'high' };

/** Where the choice is kept (`localStorage`). */
export const VIDEO_CHOICE_KEY = 'slidock.download.videos';

const QUALITY_LABELS: Readonly<Record<VideoQuality, { label: string; hint: string }>> = {
  high: { label: 'High', hint: 'Sharpest' },
  medium: { label: 'Medium', hint: 'Balanced' },
  small: { label: 'Small', hint: 'Smallest zip' },
};

export function readVideoChoice(from?: Storage | null): VideoChoice {
  return readChoice(VIDEO_CHOICE_KEY, VIDEO_QUALITIES, from);
}

export function writeVideoChoice(choice: VideoChoice, to?: Storage | null): void {
  writeChoice(VIDEO_CHOICE_KEY, choice, to);
}

export function useVideoChoice(): [VideoChoice, (choice: VideoChoice) => void] {
  return useChoice(VIDEO_CHOICE_KEY, VIDEO_QUALITIES);
}

/** `ms` as the dialog says it: seconds under a minute, else minutes. */
function duration(ms: number): string {
  const seconds = Math.max(1, Math.round(ms / 1000));
  return seconds < 60 ? `${seconds} s` : `${Math.round(seconds / 60)} min`;
}

/**
 * What is left of the re-encode at the rate it went so far, in milliseconds, as of its last progress; `null` until it
 * has run a few seconds and done some frames.
 */
export function remaining(progress: VideosProgress): number | null {
  const elapsed = progress.updated - progress.started;
  if (elapsed < 3000 || progress.frames === 0 || progress.totalFrames === 0) {
    return null;
  }
  return (elapsed / progress.frames) * Math.max(0, progress.totalFrames - progress.frames);
}

/** The progress line: videos done, the share of frames and the time left. */
export function progressText(progress: VideosProgress): string {
  if (progress.total === 0) {
    return 'Planning the videos';
  }
  const share = progress.totalFrames > 0 ? ` · ${Math.floor((progress.frames / progress.totalFrames) * 100)}%` : '';
  const left = remaining(progress);
  return `${progress.done} of ${progress.total} ${progress.total === 1 ? 'video' : 'videos'}${share}${
    left === null ? '' : ` · about ${duration(left)} left`
  }`;
}

export interface VideoCompressionProps {
  choice: VideoChoice;
  onChange(choice: VideoChoice): void;
  disabled: boolean;
  /** Whether this browser can re-encode video (WebCodecs). */
  supported: boolean;
  /** While the videos are re-encoded. */
  progress?: VideosProgress | undefined;
  onCancel(): void;
}

/** The "Compress videos" switch and its quality choice, with the re-encode's progress and a cancel. */
export function VideoCompression({ choice, onChange, disabled, supported, progress, onCancel }: VideoCompressionProps) {
  const title = useId();
  const hint = useId();
  return (
    <section aria-label="Videos" className="space-y-2">
      <label className="flex items-start gap-3 text-sm">
        <input
          type="checkbox"
          role="switch"
          checked={choice.compress && supported}
          disabled={disabled || !supported}
          aria-labelledby={title}
          aria-describedby={hint}
          onChange={(event) => onChange({ ...choice, compress: event.currentTarget.checked })}
          className="mt-0.5 size-4 accent-[var(--brand-500)]"
        />
        <span>
          <span id={title} className="font-medium text-fg">
            Compress videos
          </span>
          <span id={hint} className="block text-muted">
            {supported
              ? 'Re-encode each video as H.264 at the size the slides draw it, with its sound and its whole length; a video every slide plays muted loses its sound. It takes a while for long videos. Videos with transparency or HDR stay as they are.'
              : 'This browser cannot re-encode video (it has no WebCodecs video encoder).'}
          </span>
        </span>
      </label>
      {choice.compress && supported && (
        <fieldset className="ml-7 flex flex-wrap gap-2" disabled={disabled}>
          <legend className="sr-only">Video quality</legend>
          {VIDEO_QUALITIES.map((quality) => (
            <label
              key={quality}
              className="flex cursor-pointer items-center gap-2 rounded-md border border-border px-2.5 py-1 text-sm has-[:checked]:border-[var(--brand-500)] has-[:checked]:bg-elevated"
            >
              <input
                type="radio"
                name="video-quality"
                value={quality}
                checked={choice.quality === quality}
                onChange={() => onChange({ ...choice, quality })}
                className="size-3.5 accent-[var(--brand-500)]"
              />
              <span className="text-fg">{QUALITY_LABELS[quality].label}</span>
              <span className="text-xs text-muted">{QUALITY_LABELS[quality].hint}</span>
            </label>
          ))}
        </fieldset>
      )}
      {progress && (
        <div className="space-y-1">
          <ProgressBar
            label="Compressing videos"
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

/** What compressing the videos saved, once the zip is built; with the zip's size without either compression. */
export function VideosResult({
  videos,
  images,
  zipBytes,
  singleFile,
}: {
  videos: VideosSummary;
  images: ImagesSummary | undefined;
  zipBytes: number;
  singleFile: boolean;
}) {
  const both = {
    before: videos.before + (images?.before ?? 0),
    after: videos.after + (images?.after ?? 0),
  };
  const without = zipWithout(both, zipBytes, singleFile);
  return (
    <section aria-labelledby="zip-videos">
      <h3 id="zip-videos" className="text-[11px] font-semibold uppercase tracking-wider text-muted">
        Compressed videos
      </h3>
      <p className="mt-2 text-sm text-fg-secondary">
        <span className="font-mono tabular-nums">
          {formatBytes(videos.before)} → {formatBytes(videos.after)}
        </span>{' '}
        {saved(videos.before, videos.after)}: {videos.compressed - videos.copied} of {videos.planned}{' '}
        {videos.planned === 1 ? 'video' : 'videos'} re-encoded
        {videos.soundEncoded > 0 && `, ${videos.soundEncoded} with their sound re-encoded to AAC`}
        {videos.copied > 0 && `, ${videos.copied} copied as ${videos.copied === 1 ? 'it is' : 'they are'}`}.
      </p>
      {videos.soundRemoved > 0 && (
        <p className="text-sm text-fg-secondary">
          Sound removed from {videos.soundRemoved} {videos.soundRemoved === 1 ? 'video' : 'videos'} muted in every use:{' '}
          <span className="font-mono tabular-nums">{formatBytes(videos.soundRemovedBytes)}</span> saved.
        </p>
      )}
      <p className="text-sm text-fg-secondary">
        The zip: <span className="font-mono tabular-nums">{formatBytes(zipBytes)}</span>, about{' '}
        <span className="font-mono tabular-nums">{formatBytes(without)}</span> without compressing them.
      </p>
      {videos.kept.length > 0 && (
        <details className="mt-1 text-xs text-muted">
          <summary className="cursor-pointer">
            {videos.kept.length} kept as {videos.kept.length === 1 ? 'it is' : 'they are'}
          </summary>
          <ul className="mt-1 space-y-0.5 font-mono">
            {videos.kept.map(({ path, reason }) => (
              <li key={path} className="break-words" title={path}>
                {path.replace(/^media\//, '').slice(0, 12)}…: {reason}
              </li>
            ))}
          </ul>
        </details>
      )}
    </section>
  );
}
