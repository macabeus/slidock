import type { BundleSettings, Deck, FontReport, NotesPlacement, PageDefaults, PlannedVideo } from '@slidock/types';
import { Banner, Button, Dialog, Icon, ProgressBar } from '@slidock/ui';
import { useEffect, useMemo, useRef, useState } from 'react';

import type { BundleState } from '../compile/session';
import { formatBytes, saveBlob, zipName } from '../deck/download';
import { siteUrlProblem, sizeNotice } from '../deck/page';
import type { FontsController } from '../fonts/controller';
import { familyRows } from '../fonts/families';
import { useFontsState } from '../fonts/useFonts';
import { canMakeAlphaCopies } from '../media/alpha';
import type { ImageQuality } from '../media/compress';
import { type VideoQuality, hasVideoEncoder } from '../media/video';
import { includeInDownload } from './FontsPanel';
import { ImageCompression, ImagesResult, useImageChoice } from './ImageCompression';
import { VideoCompression, VideosResult, useVideoChoice } from './VideoCompression';
import { AlternatesResult, VideoCopies, useCopiesChoice } from './VideoCopies';

export interface DownloadDialogProps {
  open: boolean;
  onClose(): void;
  deck: Deck;
  /** The fonts panel's faces of the deck (the deck message's `faces`): which files come from this computer or the user. */
  faces: readonly FontReport[];
  /** The page's default title and description (the deck message's `page`), which the CLI writes too. */
  page?: PageDefaults | undefined;
  /** The deck's fonts flow: the families it may leave out of the download. */
  fonts: FontsController | null;
  bundle: BundleState;
  /** `quality` is the pictures' and `videoQuality` the videos' when the settings ask to compress them. */
  onBundle(settings: BundleSettings, quality?: ImageQuality, videoQuality?: VideoQuality): void;
  onReset(): void;
  /** Stops a running download. */
  onCancel?(): void;
  /** The deck's videos that get a copy other browsers play (asked once the dialog opens); none when absent. */
  planAlternates?: () => Promise<PlannedVideo[]>;
}

const SHOWN_FILES = 8;

/**
 * Which notes the zip carries, as the CLI's flags say it (`--no-notes`, `--notes-on-page`): they ship only where the
 * page shows them, in the presenter view (when it ships one) and, when asked, in the text version.
 */
export function notesPlacement(choice: {
  presenter: boolean;
  notes: boolean;
  notesOnPage: boolean;
  textView: boolean;
}): NotesPlacement {
  const onPage = choice.notesOnPage && choice.textView;
  if (!choice.notes || (!choice.presenter && !onPage)) {
    return 'none';
  }
  return onPage ? 'page' : 'presenter';
}

const NEEDS_TEXT_VIEW = 'Needs the text version below the slides.';

/** The speaker notes on the page: in the text version, so only with it. */
function NotesOnPage({
  label,
  hint,
  checked,
  textView,
  disabled,
  onChange,
  className = '',
}: {
  label: string;
  hint?: string;
  checked: boolean;
  textView: boolean;
  disabled: boolean;
  onChange(checked: boolean): void;
  className?: string;
}) {
  return (
    <label className={`flex items-start gap-3 text-sm ${className}`}>
      <input
        type="checkbox"
        checked={checked && textView}
        disabled={disabled || !textView}
        onChange={(event) => onChange(event.currentTarget.checked)}
        className="mt-0.5 size-4 accent-[var(--brand-500)] disabled:opacity-60"
      />
      <span>
        <span className="font-medium text-fg">{label}</span>
        {(textView ? hint : NEEDS_TEXT_VIEW) && (
          <span className="block text-muted">{textView ? hint : NEEDS_TEXT_VIEW}</span>
        )}
      </span>
    </label>
  );
}

const FIELD =
  'mt-1 block w-full rounded-md border border-border bg-elevated px-2.5 py-1.5 text-sm text-fg placeholder:text-fg-secondary disabled:opacity-60';

/** Builds the zip in the worker, saves it, and says what it holds and which fonts were left out. */
export function DownloadDialog({
  open,
  onClose,
  deck,
  faces,
  page,
  fonts,
  bundle,
  onBundle,
  onReset,
  onCancel = () => {},
  planAlternates,
}: DownloadDialogProps) {
  const [singleFile, setSingleFile] = useState(false);
  // Prefilled with the defaults the bundle writes when it is given none: only an edit is sent.
  const initialTitle = page?.title ?? deck.title;
  const initialDescription = page?.description ?? '';
  const [title, setTitle] = useState(initialTitle);
  const [description, setDescription] = useState(initialDescription);
  const [siteUrl, setSiteUrl] = useState('');
  const [textView, setTextView] = useState(true);
  const [presenter, setPresenter] = useState(true);
  const [notes, setNotes] = useState(true);
  const [notesOnPage, setNotesOnPage] = useState(false);
  const placement = notesPlacement({ presenter, notes, notesOnPage, textView });
  const urlProblem = siteUrl === '' ? null : siteUrlProblem(siteUrl);
  const [images, setImages] = useImageChoice();
  const [videos, setVideos] = useVideoChoice();
  const [videoEncoder] = useState(() => hasVideoEncoder());
  const [copies, setCopies] = useCopiesChoice();
  const [copier] = useState(() => canMakeAlphaCopies());
  const [planned, setPlanned] = useState<readonly PlannedVideo[]>([]);
  const hasVideos = deck.media.some((medium) => medium.mime.startsWith('video/'));
  const fontsState = useFontsState(fonts);
  const leftOut = fontsState?.leaveOut;
  const included = fontsState?.include;
  const personal = useMemo(
    () => familyRows(deck, faces, [], new Set(leftOut ?? []), new Set(included ?? [])).filter((row) => row.personal),
    [deck, faces, leftOut, included],
  );
  useEffect(() => {
    let live = true;
    if (open && planAlternates) {
      void planAlternates().then((videos) => live && setPlanned(videos));
    }
    return () => {
      live = false;
    };
  }, [open, planAlternates, deck]);
  const saved = useRef<Blob | null>(null);
  const name = zipName(deck.title);

  useEffect(() => {
    if (bundle.state === 'done' && saved.current !== bundle.blob) {
      saved.current = bundle.blob;
      saveBlob(bundle.blob, name);
    }
  }, [bundle, name]);

  const notice = bundle.state === 'done' ? sizeNotice(bundle.report, textView) : null;
  const knownMedia = deck.media.reduce((sum, medium) => sum + (medium.bytes ?? 0), 0);
  const running = bundle.state === 'running';

  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="Download zip"
      subtitle="A standalone copy that plays offline, from any folder or static host"
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            Close
          </Button>
          {bundle.state === 'done' ? (
            <Button variant="primary" onClick={() => saveBlob(bundle.blob, name)} data-autofocus>
              <Icon name="download" className="size-4" />
              Save again
            </Button>
          ) : (
            <Button
              variant="primary"
              disabled={running || urlProblem !== null}
              data-autofocus
              onClick={() =>
                onBundle(
                  {
                    singleFile,
                    singleFileThreshold: null,
                    leaveOut: [...(leftOut ?? [])],
                    include: [...(included ?? [])],
                    ...(images.compress ? { compressImages: true } : {}),
                    ...(videos.compress && videoEncoder && hasVideos ? { compressVideos: true } : {}),
                    ...(title === initialTitle ? {} : { title }),
                    ...(description === initialDescription ? {} : { description }),
                    ...(siteUrl === '' ? {} : { siteUrl }),
                    ...(presenter ? {} : { presenter: false }),
                    ...(placement === 'presenter' ? {} : { notes: placement }),
                    ...(textView ? {} : { textView: false }),
                    ...(copies && planned.length > 0 ? { alternateVideos: true } : {}),
                  },
                  images.quality,
                  videos.quality,
                )
              }
            >
              <Icon name="download" className="size-4" />
              {running ? 'Building…' : 'Build and save'}
            </Button>
          )}
        </>
      }
    >
      <section aria-labelledby="zip-contents">
        <h3 id="zip-contents" className="text-[11px] font-semibold uppercase tracking-wider text-muted">
          What's inside
        </h3>
        <ul className="mt-2 space-y-1 text-sm text-fg-secondary">
          <li>
            <code className="font-mono text-fg">index.html</code>: every slide, the deck data, the fonts and the player
            {presenter && ', with its presenter view'}
          </li>
          <li>
            <code className="font-mono text-fg">media/</code>:{' '}
            <span className="font-mono tabular-nums">{deck.media.length}</span>{' '}
            {deck.media.length === 1 ? 'file' : 'files'}
            {knownMedia > 0 && <span className="font-mono tabular-nums"> ({formatBytes(knownMedia)} or more)</span>}
          </li>
          <li>
            <code className="font-mono text-fg">LICENSES.txt</code>: the licences of the fonts it ships
          </li>
        </ul>
      </section>

      <label className="flex items-start gap-3 text-sm">
        <input
          type="checkbox"
          checked={singleFile}
          disabled={running}
          onChange={(event) => {
            setSingleFile(event.currentTarget.checked);
            onReset();
          }}
          className="mt-0.5 size-4 accent-[var(--brand-500)]"
        />
        <span>
          <span className="font-medium text-fg">Single HTML file</span>
          <span className="block text-muted">Put the media inside index.html too (decks under 50 MB).</span>
        </span>
      </label>

      <section aria-labelledby="zip-page" className="space-y-3">
        <h3 id="zip-page" className="text-[11px] font-semibold uppercase tracking-wider text-muted">
          The page
        </h3>
        <label className="block text-sm">
          <span className="font-medium text-fg">Title</span>
          <input
            type="text"
            value={title}
            placeholder={initialTitle}
            disabled={running}
            onChange={(event) => {
              setTitle(event.currentTarget.value);
              onReset();
            }}
            className={FIELD}
          />
        </label>
        <label className="block text-sm">
          <span className="font-medium text-fg">Description</span>
          <span className="block text-muted">What search results and link previews show under the title.</span>
          <textarea
            value={description}
            rows={2}
            disabled={running}
            onChange={(event) => {
              setDescription(event.currentTarget.value);
              onReset();
            }}
            className={FIELD}
          />
        </label>
        <label className="block text-sm">
          <span className="font-medium text-fg">Public URL (optional)</span>
          <span className="block text-muted">Where the deck will be published: the page names it as its address.</span>
          <input
            type="url"
            value={siteUrl}
            placeholder="https://example.com/talk/"
            disabled={running}
            aria-invalid={urlProblem !== null}
            aria-describedby={urlProblem === null ? undefined : 'zip-url-problem'}
            onChange={(event) => {
              setSiteUrl(event.currentTarget.value);
              onReset();
            }}
            className={FIELD}
          />
          {urlProblem !== null && (
            <span id="zip-url-problem" role="alert" className="mt-1 block text-xs text-danger">
              {urlProblem}
            </span>
          )}
        </label>
        <label className="flex items-start gap-3 text-sm">
          <input
            type="checkbox"
            checked={textView}
            disabled={running}
            onChange={(event) => {
              setTextView(event.currentTarget.checked);
              onReset();
            }}
            className="mt-0.5 size-4 accent-[var(--brand-500)]"
          />
          <span>
            <span className="font-medium text-fg">Text version below the slides</span>
            <span className="block text-muted">
              The slides' text, which readers can scroll to and search engines index.
            </span>
          </span>
        </label>
        <label className="flex items-start gap-3 text-sm">
          <input
            type="checkbox"
            checked={presenter}
            disabled={running}
            onChange={(event) => {
              setPresenter(event.currentTarget.checked);
              onReset();
            }}
            className="mt-0.5 size-4 accent-[var(--brand-500)]"
          />
          <span>
            <span className="font-medium text-fg">Presenter view</span>
            <span className="block text-muted">
              P opens a second window with the next slide, the speaker notes, timers and controls, as here.
            </span>
          </span>
        </label>
        {presenter ? (
          <div className="space-y-2">
            <label className="flex items-start gap-3 text-sm">
              <input
                type="checkbox"
                checked={notes}
                disabled={running}
                onChange={(event) => {
                  setNotes(event.currentTarget.checked);
                  onReset();
                }}
                className="mt-0.5 size-4 accent-[var(--brand-500)]"
              />
              <span>
                <span className="font-medium text-fg">Speaker notes</span>
                <span className="block text-muted">
                  {notes ? 'In the presenter view.' : 'Left out: the zip carries none.'}
                </span>
              </span>
            </label>
            {notes && (
              <NotesOnPage
                label="Also on the page, below each slide's text"
                checked={notesOnPage}
                textView={textView}
                disabled={running}
                onChange={(checked) => {
                  setNotesOnPage(checked);
                  onReset();
                }}
                className="ml-7"
              />
            )}
          </div>
        ) : (
          <NotesOnPage
            label="Speaker notes on the page, below each slide's text"
            hint="Without the presenter view only the page shows them: unchecked, the zip carries none."
            checked={notes && notesOnPage}
            textView={textView}
            disabled={running}
            onChange={(checked) => {
              if (checked) {
                setNotes(true);
              }
              setNotesOnPage(checked);
              onReset();
            }}
          />
        )}
      </section>

      <ImageCompression
        choice={images}
        onChange={(choice) => {
          setImages(choice);
          onReset();
        }}
        disabled={running}
        progress={bundle.state === 'running' ? bundle.images : undefined}
      />

      {hasVideos && (
        <VideoCompression
          choice={videos}
          onChange={(choice) => {
            setVideos(choice);
            onReset();
          }}
          disabled={running}
          supported={videoEncoder}
          progress={bundle.state === 'running' ? bundle.videos : undefined}
          onCancel={onCancel}
        />
      )}

      {planned.length > 0 && (
        <VideoCopies
          planned={planned}
          on={copies}
          onChange={(on) => {
            setCopies(on);
            onReset();
          }}
          disabled={running}
          supported={copier}
          progress={bundle.state === 'running' ? bundle.alternates : undefined}
          onCancel={onCancel}
        />
      )}

      <p className="text-sm text-muted">
        Embedded, Google and substitute fonts ship in the zip. A font whose licence forbids it is left out and drawn
        with a substitute.
      </p>

      {personal.length > 0 && fonts && (
        <section
          aria-labelledby="zip-personal"
          className="space-y-2 rounded-md border border-warning/30 bg-warning/5 p-3"
        >
          <h3 id="zip-personal" className="text-sm font-semibold text-warning">
            Fonts from this computer or your files
          </h3>
          <p className="text-xs text-fg-secondary">
            The zip ships them so it looks the same everywhere. Share it only where their licences allow, or leave a
            family out: the zip then draws it with the viewer's installed font or a substitute.
          </p>
          <ul className="space-y-1">
            {personal.map((row) => (
              <li key={row.family}>
                <label className="flex items-center gap-2 text-sm">
                  <input
                    type="checkbox"
                    checked={!row.leftOut}
                    disabled={running}
                    onChange={(event) => includeInDownload(fonts, row, event.currentTarget.checked)}
                    className="size-4 accent-[var(--brand-500)]"
                  />
                  <span className="text-fg">Include {row.family}</span>
                  {row.restricted && <span className="text-xs text-warning">its licence restricts embedding it</span>}
                </label>
              </li>
            ))}
          </ul>
        </section>
      )}

      {running && !bundle.images && !bundle.videos && !bundle.alternates && (
        <div className="space-y-2">
          <ProgressBar label="Building the zip" />
          <p className="font-mono text-xs tabular-nums text-muted" aria-live="polite">
            {formatBytes(bundle.bytes)} written
          </p>
        </div>
      )}

      {bundle.state === 'failed' && (
        <Banner tone="danger" title="The zip couldn't be built">
          <p className="break-words font-mono text-xs">
            {bundle.error.kind}: {bundle.error.message}
          </p>
          {bundle.error.diagnostics.map((diagnostic, index) => (
            <p key={index} className="text-xs">
              <code className="font-mono font-semibold">{diagnostic.code}</code> {diagnostic.message}
            </p>
          ))}
        </Banner>
      )}

      {bundle.state === 'done' && (
        <>
          <Banner tone="success" title={`Saved ${name}`}>
            <span className="font-mono tabular-nums">{formatBytes(bundle.report.bytes)}</span>,{' '}
            {bundle.report.files.length} {bundle.report.files.length === 1 ? 'file' : 'files'}
            {bundle.report.singleFile && ', media inside index.html'}.
          </Banner>
          {notice !== null && (
            <Banner tone="warning" title="Larger than search engines read">
              <p className="text-xs">{notice}</p>
            </Banner>
          )}
          {bundle.images && (
            <ImagesResult
              images={bundle.images}
              zipBytes={bundle.report.bytes}
              singleFile={bundle.report.singleFile}
              showZip={!bundle.videos}
            />
          )}
          {bundle.videos && (
            <VideosResult
              videos={bundle.videos}
              images={bundle.images}
              zipBytes={bundle.report.bytes}
              singleFile={bundle.report.singleFile}
            />
          )}
          {planned.length > 0 && <AlternatesResult copies={bundle.alternates ?? null} planned={planned} />}
          <section aria-labelledby="zip-files">
            <h3 id="zip-files" className="text-[11px] font-semibold uppercase tracking-wider text-muted">
              Files
            </h3>
            <ul className="mt-2 space-y-0.5 font-mono text-xs">
              {bundle.report.files.slice(0, SHOWN_FILES).map((file) => (
                <li key={file.path} className="flex justify-between gap-4">
                  <span className="truncate text-fg-secondary">{file.path}</span>
                  <span className="shrink-0 tabular-nums text-muted">{formatBytes(file.bytes)}</span>
                </li>
              ))}
              {bundle.report.files.length > SHOWN_FILES && (
                <li className="text-muted">and {bundle.report.files.length - SHOWN_FILES} more</li>
              )}
            </ul>
          </section>
          <section aria-labelledby="zip-fonts">
            <h3 id="zip-fonts" className="text-[11px] font-semibold uppercase tracking-wider text-muted">
              Fonts left out
            </h3>
            {bundle.report.fontsLeftOut.length === 0 ? (
              <p className="mt-2 text-sm text-fg-secondary">None: every font the slides draw is in the zip.</p>
            ) : (
              <>
                <ul className="mt-2 flex flex-wrap gap-1.5">
                  {bundle.report.fontsLeftOut.map((family) => (
                    <li key={family} className="rounded bg-warning/10 px-2 py-0.5 text-xs text-warning">
                      {family}
                    </li>
                  ))}
                </ul>
                <p className="mt-2 text-xs text-muted">
                  Left out for their licence or by you. The zip draws them with the viewer's installed font or a
                  substitute.
                </p>
              </>
            )}
          </section>
        </>
      )}
    </Dialog>
  );
}
