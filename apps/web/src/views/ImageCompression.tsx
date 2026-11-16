import { ProgressBar } from '@slidock/ui';
import { useCallback, useId, useState } from 'react';

import type { ImagesSummary } from '../compile/session';
import { formatBytes } from '../deck/download';
import { IMAGE_QUALITIES, type ImageQuality } from '../media/compress';

/** The download dialog's picture choice, which the viewer's browser remembers. */
export type ImageChoice = CompressionChoice<ImageQuality>;

export const DEFAULT_IMAGE_CHOICE: ImageChoice = { compress: false, quality: 'high' };

/** Where the choice is kept (`localStorage`). */
export const IMAGE_CHOICE_KEY = 'slidock.download.images';

const QUALITY_LABELS: Readonly<Record<ImageQuality, { label: string; hint: string }>> = {
  high: { label: 'High', hint: 'Sharpest' },
  medium: { label: 'Medium', hint: 'Balanced' },
  small: { label: 'Small', hint: 'Smallest zip' },
};

function storage(): Storage | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

/** A compression choice of the download dialog: on or off, at one of a few qualities. */
export interface CompressionChoice<Quality extends string> {
  compress: boolean;
  quality: Quality;
}

/** The choice kept under `key`; the default (off, the first quality) when there is none or it is not one. */
export function readChoice<Quality extends string>(
  key: string,
  qualities: readonly Quality[],
  from: Storage | null = storage(),
): CompressionChoice<Quality> {
  const fallback = { compress: false, quality: qualities[0]! };
  try {
    const parsed: unknown = JSON.parse(from?.getItem(key) ?? 'null');
    if (typeof parsed !== 'object' || parsed === null) {
      return fallback;
    }
    const { compress, quality } = parsed as Partial<Record<'compress' | 'quality', unknown>>;
    return {
      compress: compress === true,
      quality: qualities.includes(quality as Quality) ? (quality as Quality) : fallback.quality,
    };
  } catch {
    return fallback;
  }
}

/** Keeps `choice` under `key`; a browser that keeps nothing (a private window, blocked storage) forgets it. */
export function writeChoice<Quality extends string>(
  key: string,
  choice: CompressionChoice<Quality>,
  to: Storage | null = storage(),
): void {
  try {
    to?.setItem(key, JSON.stringify(choice));
  } catch {
    // Not kept: the dialog still uses it until the page closes.
  }
}

/** The choice kept under `key`, read once, and a setter that keeps it. */
export function useChoice<Quality extends string>(
  key: string,
  qualities: readonly Quality[],
): [CompressionChoice<Quality>, (choice: CompressionChoice<Quality>) => void] {
  const [choice, setChoice] = useState(() => readChoice(key, qualities));
  const update = useCallback(
    (next: CompressionChoice<Quality>) => {
      setChoice(next);
      writeChoice(key, next);
    },
    [key],
  );
  return [choice, update];
}

/** The remembered choice; the default when there is none, storage is unavailable, or what it holds is not one. */
export function readImageChoice(from: Storage | null = storage()): ImageChoice {
  return readChoice(IMAGE_CHOICE_KEY, IMAGE_QUALITIES, from);
}

/** Remembers `choice`; a browser that keeps nothing (a private window, blocked storage) forgets it. */
export function writeImageChoice(choice: ImageChoice, to: Storage | null = storage()): void {
  writeChoice(IMAGE_CHOICE_KEY, choice, to);
}

/** The choice, read once, and a setter that remembers it. */
export function useImageChoice(): [ImageChoice, (choice: ImageChoice) => void] {
  return useChoice(IMAGE_CHOICE_KEY, IMAGE_QUALITIES);
}

/** The share of `before` that `after` saves, as a whole percentage (`−80%`). */
export function saved(before: number, after: number): string {
  return before > 0 ? `−${Math.round((1 - after / before) * 100)}%` : '';
}

/**
 * The zip's size without the re-encoded pictures: the files written in their place swapped back, each base64 (4 bytes
 * per 3) in a single file.
 */
export function zipWithout(
  images: Pick<ImagesSummary, 'before' | 'after'>,
  zipBytes: number,
  singleFile: boolean,
): number {
  const swapped = images.before - images.after;
  return zipBytes + (singleFile ? Math.ceil((swapped * 4) / 3) : swapped);
}

export interface ImageCompressionProps {
  choice: ImageChoice;
  onChange(choice: ImageChoice): void;
  disabled: boolean;
  /** While the pictures are re-encoded: how many are done of how many. */
  progress?: { done: number; total: number } | undefined;
}

/** The "Compress images" switch and its quality choice, with the re-encode's progress. */
export function ImageCompression({ choice, onChange, disabled, progress }: ImageCompressionProps) {
  const title = useId();
  const hint = useId();
  return (
    <section aria-label="Pictures" className="space-y-2">
      <label className="flex items-start gap-3 text-sm">
        <input
          type="checkbox"
          role="switch"
          checked={choice.compress}
          disabled={disabled}
          aria-labelledby={title}
          aria-describedby={hint}
          onChange={(event) => onChange({ ...choice, compress: event.currentTarget.checked })}
          className="mt-0.5 size-4 accent-[var(--brand-500)]"
        />
        <span>
          <span id={title} className="font-medium text-fg">
            Compress images
          </span>
          <span id={hint} className="block text-muted">
            Scale each picture down to what the slides need and re-encode it (WebP). The slides look the same, with some
            compression detail lost. Vector and animated pictures stay as they are.
          </span>
        </span>
      </label>
      {choice.compress && (
        <fieldset className="ml-7 flex flex-wrap gap-2" disabled={disabled}>
          <legend className="sr-only">Image quality</legend>
          {IMAGE_QUALITIES.map((quality) => (
            <label
              key={quality}
              className="flex cursor-pointer items-center gap-2 rounded-md border border-border px-2.5 py-1 text-sm has-[:checked]:border-[var(--brand-500)] has-[:checked]:bg-elevated"
            >
              <input
                type="radio"
                name="image-quality"
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
            label="Compressing images"
            {...(progress.total > 0 ? { value: progress.done, max: progress.total } : {})}
          />
          <p className="font-mono text-xs tabular-nums text-muted" aria-live="polite">
            {progress.total > 0
              ? `${progress.done} of ${progress.total} ${progress.total === 1 ? 'picture' : 'pictures'}`
              : 'Planning the pictures'}
          </p>
        </div>
      )}
    </section>
  );
}

/** What compressing the pictures saved, once the zip is built. */
export function ImagesResult({
  images,
  zipBytes,
  singleFile,
  showZip = true,
}: {
  images: ImagesSummary;
  zipBytes: number;
  singleFile: boolean;
  /** Whether to say what the zip would be without them (`false` where the videos' result says it for both). */
  showZip?: boolean;
}) {
  const without = zipWithout(images, zipBytes, singleFile);
  return (
    <section aria-labelledby="zip-images">
      <h3 id="zip-images" className="text-[11px] font-semibold uppercase tracking-wider text-muted">
        Compressed images
      </h3>
      <p className="mt-2 text-sm text-fg-secondary">
        <span className="font-mono tabular-nums">
          {formatBytes(images.before)} → {formatBytes(images.after)}
        </span>{' '}
        {saved(images.before, images.after)}: {images.compressed} of {images.planned}{' '}
        {images.planned === 1 ? 'picture' : 'pictures'} re-encoded.
      </p>
      {showZip && (
        <p className="text-sm text-fg-secondary">
          The zip: <span className="font-mono tabular-nums">{formatBytes(zipBytes)}</span>, about{' '}
          <span className="font-mono tabular-nums">{formatBytes(without)}</span> without compressing them.
        </p>
      )}
      {images.kept.length > 0 && (
        <details className="mt-1 text-xs text-muted">
          <summary className="cursor-pointer">
            {images.kept.length} kept as {images.kept.length === 1 ? 'it is' : 'they are'}
          </summary>
          <ul className="mt-1 space-y-0.5 font-mono">
            {images.kept.slice(0, 8).map(({ path, reason }) => (
              <li key={path} className="truncate" title={reason}>
                {path.replace(/^media\//, '').slice(0, 12)}…: {reason}
              </li>
            ))}
          </ul>
        </details>
      )}
    </section>
  );
}
