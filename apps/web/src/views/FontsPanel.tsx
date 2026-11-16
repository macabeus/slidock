import type { Deck, FaceRequest, FontNeed, FontReport, FontSource } from '@slidock/types';
import { Banner, Button, Drawer, Icon, Pill, cx, focusRing } from '@slidock/ui';
import { type ReactNode, useId, useMemo, useRef, useState } from 'react';

import type { FontsController, FontsState } from '../fonts/controller';
import { type DroppedFile, FONT_ACCEPT, isFontFile } from '../fonts/dropped';
import {
  EMOJI_SOURCE,
  type FamilyFace,
  type FamilyRow,
  STAND_INS,
  familyRows,
  missingFamilies,
} from '../fonts/families';
import { useFontsState } from '../fonts/useFonts';

export const SOURCES: Record<FontSource, { label: string; tint: string }> = {
  embedded: { label: 'Embedded in the file', tint: 'var(--font-embedded)' },
  local: { label: 'This computer', tint: 'var(--font-local)' },
  google: { label: 'Google Fonts', tint: 'var(--font-google)' },
  user: { label: 'Your file', tint: 'var(--font-dropped)' },
  bundled: { label: 'Substitute', tint: 'var(--font-substituted)' },
  system: { label: 'Not available', tint: 'var(--font-missing)' },
  emoji: { label: "The viewer's emoji font", tint: 'var(--font-local)' },
};

export function faceLabel(face: Pick<FaceRequest, 'weight' | 'style'>): string {
  return `${face.weight}${face.style === 'italic' ? ' italic' : ''}`;
}

function plural(count: number, one: string, many = `${one}s`): string {
  return `${count} ${count === 1 ? one : many}`;
}

function SectionTitle({ id, children }: { id: string; children: ReactNode }) {
  return (
    <h3 id={id} className="text-[11px] font-semibold uppercase tracking-wider text-muted">
      {children}
    </h3>
  );
}

export interface FontsPanelProps {
  open: boolean;
  onClose(): void;
  deck: Deck;
  /** The fonts panel's faces of the deck shown (the deck message's `faces`). */
  faces: readonly FontReport[];
  /** The faces nothing provides, as of the deck shown. */
  needs: readonly FontNeed[];
  /** The deck's fonts flow; without one the panel only lists the faces. */
  fonts: FontsController | null;
}

/**
 * Each family the slides draw, where each face comes from and what the download does with it; then the sources: the
 * fonts installed on this computer (Local Font Access), Google Fonts, and font files the user adds.
 */
export function FontsPanel({ open, onClose, deck, faces, needs, fonts }: FontsPanelProps) {
  const state = useFontsState(fonts);
  const currentNeeds = state?.needs ?? needs;
  const leftOut = state?.leaveOut;
  const included = state?.include;
  const leaveOut = useMemo(() => new Set(leftOut ?? []), [leftOut]);
  const include = useMemo(() => new Set(included ?? []), [included]);
  const rows = useMemo(
    () => familyRows(deck, faces, currentNeeds, leaveOut, include),
    [deck, faces, currentNeeds, leaveOut, include],
  );
  const missing = missingFamilies(rows);

  return (
    <Drawer open={open} onClose={onClose} title="Fonts" subtitle="Where each face the slides draw comes from">
      {missing.length > 0 ? (
        <Banner
          tone="warning"
          title={`${missing.length} ${missing.length === 1 ? "font isn't" : "fonts aren't"} in the file: ${missing.join(', ')}`}
        >
          The slides draw {missing.length === 1 ? 'it' : 'them'} with a substitute until a source below has{' '}
          {missing.length === 1 ? 'it' : 'them'}.
        </Banner>
      ) : (
        rows.length > 0 && (
          <Banner tone="success" title="Every font is available">
            The slides draw with the fonts listed below.
          </Banner>
        )
      )}
      {state?.refreshing && (
        <p role="status" className="flex items-center gap-2 text-sm text-muted">
          <span aria-hidden className="size-2 rounded-full bg-brand-400 motion-safe:animate-pulse" />
          Updating the slides with the new fonts…
        </p>
      )}
      {state?.refreshError && (
        <Banner tone="danger" title="The slides couldn't be updated">
          <p className="break-words font-mono text-xs">{state.refreshError}</p>
        </Banner>
      )}

      <section aria-labelledby="fonts-families">
        <SectionTitle id="fonts-families">Families</SectionTitle>
        {rows.length === 0 ? (
          <p className="mt-2 text-sm text-muted">The slides draw no text.</p>
        ) : (
          <ul className="mt-2 divide-y divide-line-subtle">
            {rows.map((row) => (
              <FamilyItem key={row.family} row={row} fonts={fonts} />
            ))}
          </ul>
        )}
      </section>

      {fonts && state && (
        <>
          <LocalFontsSection fonts={fonts} state={state} />
          <GoogleSection fonts={fonts} state={state} />
          <FilesSection fonts={fonts} state={state} />
          <SubstitutesSection state={state} />
        </>
      )}
    </Drawer>
  );
}

function FamilyItem({ row, fonts }: { row: FamilyRow; fonts: FontsController | null }) {
  const toggleId = useId();
  return (
    <li className="space-y-1.5 py-3" aria-label={row.family}>
      <p className="text-base text-fg">{row.family}</p>
      <ul className="flex flex-wrap gap-1.5" aria-label={`Faces of ${row.family}`}>
        {row.faces.map((face) => (
          <li key={`${face.weight}-${face.style}`}>
            <FacePill face={face} />
          </li>
        ))}
        {row.missing.map((face) => (
          <li key={`missing-${face.weight}-${face.style}`}>
            <Pill tint="var(--font-missing)" dot size="xs">
              <span className="font-mono tabular-nums">{faceLabel(face)}</span>
              <span>· Missing</span>
            </Pill>
          </li>
        ))}
      </ul>
      {row.faces.some((face) => face.demoted) && (
        <p className="text-xs text-warning">
          The file embeds a face whose own name says another weight or family. It draws only because no source has the
          face asked for.
        </p>
      )}
      {row.faces.some((face) => face.synthesized) && (
        <p className="text-xs text-muted">
          No file has a synthesized face: the browser emboldens or slants the face it names.
        </p>
      )}
      <RefusedNote row={row} />
      {row.faces.some((face) => face.source === EMOJI_SOURCE) && (
        <p className="text-xs text-muted">
          An emoji font: no file ships, and each viewer's own emoji font draws it (Apple Color Emoji on a Mac, Segoe UI
          Emoji on Windows, Noto Color Emoji elsewhere).
        </p>
      )}
      {row.repaired && (
        <p className="text-xs text-muted">
          Its file has no {row.repaired} table, which browsers require: slidock wrote one from the file's other tables,
          so browsers load it.
        </p>
      )}
      {row.restricted && <RestrictedNote row={row} />}
      {row.personal && fonts && (
        <label htmlFor={toggleId} className="flex items-start gap-2 text-xs text-fg-secondary">
          <input
            id={toggleId}
            type="checkbox"
            checked={!row.leftOut}
            onChange={(event) => includeInDownload(fonts, row, event.currentTarget.checked)}
            className="mt-0.5 size-3.5 accent-[var(--brand-500)]"
          />
          <span>
            Include in the download
            <span className="block text-muted">
              {row.restricted
                ? row.unstated
                  ? 'It states no licence for embedding it: include it only if you may share it.'
                  : 'Its licence restricts embedding it: include it only if you may share it.'
                : 'It comes from this computer or your files: share it only where its licence allows.'}
            </span>
          </span>
        </label>
      )}
    </li>
  );
}

/** The toggle of a family row: a restricted family is included on request, any other left out on request. */
export function includeInDownload(fonts: FontsController, row: FamilyRow, included: boolean) {
  if (row.restricted) {
    fonts.setInclude(row.family, included);
  } else {
    fonts.setLeaveOut(row.family, !included);
  }
}

const FOUND_IN: Partial<Record<FontSource, string>> = {
  embedded: 'in the presentation',
  local: 'on this computer',
  user: 'in your font file',
};

/**
 * Why a substitute draws faces a source has a file for: the file was found, but browsers would refuse it or its
 * licence keeps it out.
 */
function RefusedNote({ row }: { row: FamilyRow }) {
  const refused = row.faces.find((face) => face.refused !== null)?.refused;
  if (!refused) {
    return null;
  }
  const found = FOUND_IN[refused.source] ?? 'here';
  return (
    <p className="text-xs text-warning">
      Found {found}, but {refused.reason}: a substitute draws it.
    </p>
  );
}

/** Why a family whose licence restricts embedding it draws here, and what the download does with it. */
function RestrictedNote({ row }: { row: FamilyRow }) {
  const drawn = row.faces.find((face) => face.restricted && !STAND_INS.has(face.source));
  const found = (drawn && FOUND_IN[drawn.source]) ?? 'here';
  const why = row.unstated
    ? 'it states no embedding permissions (its file has no OS/2 table), so it is treated as restricted'
    : 'its licence restricts embedding it';
  return (
    <p className="text-xs text-warning">
      {row.leftOut
        ? `Found ${found}, but ${why}: the slides here draw with it, and the download leaves it out, where the viewer's installed font or a substitute draws it.`
        : `${why[0]?.toUpperCase()}${why.slice(1)}, and you included it in the download: share the zip only where its licence allows.`}
    </p>
  );
}

/** A face and what draws it: its source, or for a synthesized face the face the browser emboldens or slants. */
function FacePill({ face }: { face: FamilyFace }) {
  const source = SOURCES[face.source];
  if (face.synthesized) {
    const from = face.from ?? face;
    return (
      <Pill dot size="xs" title={`Drawn from the ${faceLabel(from)} face of a file (${source.label})`}>
        <span className="font-mono tabular-nums">{faceLabel(face)}</span>
        <span>
          · Synthesized from <span className="font-mono tabular-nums">{faceLabel(from)}</span>
        </span>
      </Pill>
    );
  }
  return (
    <Pill tint={source.tint} dot size="xs">
      <span className="font-mono tabular-nums">{faceLabel(face)}</span>
      <span>· {source.label}</span>
      {face.demoted && <span>· mislabeled</span>}
      {face.restricted && <span>{STAND_INS.has(face.source) ? '· not in the download' : '· restricted licence'}</span>}
      {face.refused && !face.restricted && <span>· file refused</span>}
    </Pill>
  );
}

/** The fonts installed on this computer, through Local Font Access: one visible state per permission and outcome. */
function LocalFontsSection({ fonts, state }: { fonts: FontsController; state: FontsState }) {
  const { permission, local, localMatching } = state;
  const button = (label: string) => (
    <Button variant="wash" size="sm" onClick={() => void fonts.useLocalFonts()}>
      <Icon name="desktop" className="size-4" />
      {label}
    </Button>
  );
  let status: string;
  let body: ReactNode;
  if (permission === 'unsupported') {
    status = 'unsupported';
    body = (
      <p className="text-sm text-muted">
        This browser can't share the fonts installed on this computer. Drop the font files here instead.
      </p>
    );
  } else if (localMatching === false) {
    status = 'no-matching';
    body = (
      <p className="text-sm text-muted">
        This version of the converter can't pick the fonts installed on this computer. Drop the font files here instead.
      </p>
    );
  } else if (local.state === 'reading') {
    status = 'reading';
    body = (
      <p role="status" className="text-sm text-muted">
        Reading the installed fonts the slides need…
      </p>
    );
  } else if (local.state === 'failed' && local.failure.kind === 'activation') {
    status = 'activation';
    body = (
      <>
        <p className="text-sm text-muted">The browser wants a click before it shares the installed fonts.</p>
        {button('Use fonts installed on this computer')}
      </>
    );
  } else if (local.state === 'failed' && local.failure.kind === 'failed') {
    status = 'error';
    body = (
      <>
        <p className="text-sm text-danger">
          The installed fonts couldn't be read: <span className="font-mono text-xs">{local.failure.message}</span>
        </p>
        {button('Try again')}
      </>
    );
  } else if (local.state === 'done') {
    status = 'done';
    const { listed, added, known, skipped, skippedFiles } = local.result;
    body = (
      <>
        <p role="status" className="text-sm text-fg-secondary">
          {added > 0
            ? `Added ${plural(added, 'font file')} from this computer.`
            : known > 0
              ? `The installed fonts the slides need are already added (${plural(known, 'file')}).`
              : `None of the ${plural(listed, 'installed font')} has a face the slides still need.`}
          {skipped > 0 && (
            <span className="text-muted">
              {' '}
              {plural(skipped, 'file')} not used{skippedFiles.length > 0 ? ':' : '.'}
            </span>
          )}
        </p>
        {skippedFiles.length > 0 && (
          <ul className="space-y-0.5 text-xs text-warning" aria-label="Installed files not used">
            {skippedFiles.map((file, index) => (
              <li key={`${file.name}-${index}`}>
                <span className="font-medium">{file.name}</span>: {file.reason}
              </li>
            ))}
          </ul>
        )}
        {button('Look again')}
      </>
    );
  } else if (permission === 'denied' || local.state === 'failed') {
    status = 'denied';
    body = (
      <p className="text-sm text-muted">
        Access to the installed fonts is blocked for this site. Allow “Fonts” in the site's settings, or drop the font
        files here.
      </p>
    );
  } else if (permission === null) {
    status = 'checking';
    body = <p className="text-sm text-muted">Checking whether this browser can share installed fonts…</p>;
  } else {
    status = permission;
    body = button('Use fonts installed on this computer');
  }
  return (
    <section aria-labelledby="fonts-local" className="space-y-2" data-lfa-state={status}>
      <SectionTitle id="fonts-local">This computer</SectionTitle>
      {body}
      {permission !== 'unsupported' && localMatching !== false && (
        <p className="text-xs text-muted">Only the fonts the slides need are read, and their files stay in this tab.</p>
      )}
    </section>
  );
}

function GoogleSection({ fonts, state }: { fonts: FontsController; state: FontsState }) {
  const { googleEnabled, google } = state;
  let status: ReactNode = null;
  if (google.state === 'fetching') {
    status = (
      <p role="status" className="text-sm text-muted">
        Fetching {google.families.join(', ')}…
      </p>
    );
  } else if (google.state === 'removing') {
    status = (
      <p role="status" className="text-sm text-muted">
        Removing the Google Fonts faces…
      </p>
    );
  } else if (google.state === 'done' && googleEnabled) {
    const { families, files, problems } = google.result;
    status = (
      <>
        <p className="text-sm text-fg-secondary">
          {families.length === 0
            ? 'No missing font is an open font on Google Fonts.'
            : `${plural(files, 'file')} for ${families.join(', ')}.`}
        </p>
        {problems.length > 0 && (
          <ul className="space-y-0.5 text-xs text-danger">
            {problems.map((problem, index) => (
              <li key={index}>
                {problem.family}: {problem.message}
              </li>
            ))}
          </ul>
        )}
      </>
    );
  }
  return (
    <section aria-labelledby="fonts-google" className="space-y-2">
      <SectionTitle id="fonts-google">Google Fonts</SectionTitle>
      <label className="flex items-start gap-3 text-sm">
        <input
          type="checkbox"
          role="switch"
          checked={googleEnabled}
          disabled={google.state === 'removing'}
          onChange={(event) => void fonts.setGoogle(event.currentTarget.checked)}
          className="mt-0.5 size-4 accent-[var(--brand-500)]"
        />
        <span>
          <span className="font-medium text-fg">Fetch open fonts from Google Fonts</span>
          <span className="block text-muted">
            Sends the names of the missing open-licence fonts to Google, never the slides' text.
          </span>
        </span>
      </label>
      {status}
    </section>
  );
}

const FILE_STATES: Record<DroppedFile['state'], { label: string; tone: string }> = {
  reading: { label: 'Reading…', tone: 'text-muted' },
  added: { label: 'Added', tone: 'text-success' },
  rejected: { label: 'Not used', tone: 'text-danger' },
};

const PICK_BUTTON = cx(
  'rounded-md border border-line px-3 py-1.5 text-xs font-medium text-fg hover:bg-elevated',
  focusRing,
);

function FilesSection({ fonts, state }: { fonts: FontsController; state: FontsState }) {
  const files = useRef<HTMLInputElement>(null);
  const folder = useRef<HTMLInputElement | null>(null);
  const [over, setOver] = useState(false);
  const take = (list: FileList | null, onlyFonts: boolean) => {
    const picked = Array.from(list ?? []).filter((file) => !onlyFonts || isFontFile(file));
    if (picked.length > 0) {
      void fonts.addFiles(picked);
    }
  };
  return (
    <section aria-labelledby="fonts-files" className="space-y-2">
      <SectionTitle id="fonts-files">Your font files</SectionTitle>
      {/* A drop here reaches the window's drop handler, which adds font files to the open deck. */}
      <div
        data-testid="font-drop"
        onDragOver={() => setOver(true)}
        onDragLeave={() => setOver(false)}
        onDrop={() => setOver(false)}
        className={cx(
          'flex flex-col items-center gap-2 rounded-lg border-2 border-dashed px-4 py-5 text-center text-sm transition-colors',
          over ? 'border-brand-400 bg-brand-500/5' : 'border-dim',
        )}
      >
        <p className="text-fg-secondary">
          Drop <span className="font-mono text-xs">.ttf .otf .ttc .woff .woff2</span> files here or anywhere on the page
        </p>
        <div className="flex flex-wrap justify-center gap-2">
          <button type="button" onClick={() => files.current?.click()} className={PICK_BUTTON}>
            Add font files
          </button>
          <button type="button" onClick={() => folder.current?.click()} className={PICK_BUTTON}>
            Add a folder
          </button>
        </div>
        <input
          ref={files}
          type="file"
          multiple
          accept={FONT_ACCEPT}
          aria-label="Font files"
          className="hidden"
          onChange={(event) => {
            take(event.currentTarget.files, false);
            event.currentTarget.value = '';
          }}
        />
        <input
          ref={(node) => {
            folder.current = node;
            node?.setAttribute('webkitdirectory', '');
          }}
          type="file"
          multiple
          aria-label="Font folder"
          className="hidden"
          onChange={(event) => {
            take(event.currentTarget.files, true);
            event.currentTarget.value = '';
          }}
        />
      </div>
      <p className="text-xs text-muted">A file fills the faces no other source has. It never leaves this tab.</p>
      {state.files.length > 0 && (
        <ul className="space-y-1 text-xs" aria-label="Added files">
          {state.files.map((file, index) => (
            <li key={`${file.name}-${index}`} className="flex justify-between gap-3">
              <span className="truncate font-mono text-fg-secondary">{file.name}</span>
              <span className={cx('shrink-0 text-right', FILE_STATES[file.state].tone)}>
                {FILE_STATES[file.state].label}
                {file.state === 'rejected' && <span className="block text-muted">{file.reason}</span>}
              </span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function SubstitutesSection({ state }: { state: FontsState }) {
  const { substitutes } = state;
  if (substitutes.state === 'idle') {
    return null;
  }
  return (
    <section aria-labelledby="fonts-substitutes" className="space-y-1">
      <SectionTitle id="fonts-substitutes">Substitutes</SectionTitle>
      {substitutes.state === 'loading' ? (
        <p role="status" className="text-sm text-muted">
          Loading the substitute fonts…
        </p>
      ) : (
        <>
          <p className="text-sm text-fg-secondary">
            {substitutes.families.length === 0
              ? 'No substitute was needed.'
              : `${substitutes.families.join(', ')} ${substitutes.families.length === 1 ? 'draws' : 'draw'} the faces no source has, with matching metrics.`}
          </p>
          {substitutes.problems.length > 0 && (
            <ul className="space-y-0.5 text-xs text-danger">
              {substitutes.problems.map((problem) => (
                <li key={problem}>{problem}</li>
              ))}
            </ul>
          )}
        </>
      )}
    </section>
  );
}
