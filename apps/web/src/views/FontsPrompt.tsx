import type { Deck, FontReport } from '@slidock/types';
import { Banner, Button, Icon } from '@slidock/ui';
import { useMemo, useRef, useState } from 'react';

import type { FontsController, FontsState } from '../fonts/controller';
import { FONT_ACCEPT } from '../fonts/dropped';
import { familyRows, missingFamilies } from '../fonts/families';
import { useFontsState } from '../fonts/useFonts';

export interface FontsPromptProps {
  deck: Deck;
  /** The fonts panel's faces of the deck shown (the deck message's `faces`). */
  faces: readonly FontReport[];
  /** The deck's fonts flow; without one there is nothing to ask. */
  fonts: FontsController | null;
  onOpenPanel(): void;
}

/** The sources that need no click have settled and nothing is being added: what is missing now stays missing. */
function settled(state: FontsState): boolean {
  return (
    !state.settling && !state.refreshing && state.substitutes.state !== 'loading' && state.local.state !== 'reading'
  );
}

/** Whether a click can still read installed fonts: the browser shares them and they have not been read yet. */
function canReadLocal(state: FontsState): boolean {
  return (
    state.permission !== 'unsupported' &&
    state.permission !== 'denied' &&
    state.localMatching !== false &&
    state.local.state !== 'done' &&
    !(state.local.state === 'failed' && state.local.failure.kind === 'denied')
  );
}

/** Why the families are missing; `refused` are those a source has a file for that cannot be used. */
function why(state: FontsState, one: boolean, refused: readonly string[]): string {
  if (refused.length > 0) {
    return one
      ? 'A file was found for it, but it cannot be used here: Details says why.'
      : `A file was found for ${refused.join(', ')}, but it cannot be used here: Details says why.`;
  }
  if (state.permission === 'unsupported' || state.localMatching === false) {
    return "This browser can't share the fonts installed on this computer: add the font files instead.";
  }
  if (state.permission === 'denied' || (state.local.state === 'failed' && state.local.failure.kind === 'denied')) {
    return 'Access to the installed fonts is blocked for this site: add the font files, or allow “Fonts” in the site settings.';
  }
  if (state.local.state === 'done') {
    return `The fonts this browser lists for this computer don't include ${one ? 'it' : 'them'}: add the font files.`;
  }
  return 'Use the fonts installed on this computer, or add the font files.';
}

/**
 * Asks, once a deck opens, for the fonts no source had: when the sources that need no click have settled and a family
 * still draws with a substitute, a banner over the stage names the families and offers this computer's fonts (the
 * click Local Font Access needs) and the user's files. It stays while what it started runs, goes once nothing is
 * missing, and the user can keep the substitutes for this deck.
 */
export function FontsPrompt({ deck, faces, fonts, onOpenPanel }: FontsPromptProps) {
  const state = useFontsState(fonts);
  const [dismissed, setDismissed] = useState(false);
  /** Once it has asked, it stays through the work it started rather than hiding until that settles. */
  const [asked, setAsked] = useState(false);
  const picker = useRef<HTMLInputElement>(null);
  const leftOut = state?.leaveOut;
  const leaveOut = useMemo(() => new Set(leftOut ?? []), [leftOut]);
  const needs = state?.needs;
  const rows = useMemo(() => (needs ? familyRows(deck, faces, needs, leaveOut) : []), [deck, faces, needs, leaveOut]);
  const missing = missingFamilies(rows);
  const refused = rows
    .filter((row) => missing.includes(row.family) && row.faces.some((face) => face.refused !== null))
    .map((row) => row.family);

  if (!fonts || !state || dismissed) {
    return null;
  }
  const ready = settled(state);
  if (!asked && ready && missing.length > 0) {
    setAsked(true);
  }
  if (!asked || (ready && missing.length === 0)) {
    return null;
  }

  const one = missing.length === 1;
  const working = !ready;
  return (
    <Banner
      tone="warning"
      title={`${missing.length} ${one ? "font isn't" : "fonts aren't"} in the file: ${missing.join(', ')}`}
      actions={
        <>
          {canReadLocal(state) && (
            <Button variant="primary" size="sm" disabled={working} onClick={() => void fonts.useLocalFonts()}>
              <Icon name="desktop" className="size-4" />
              Use fonts installed on this computer
            </Button>
          )}
          <Button variant="wash" size="sm" disabled={working} onClick={() => picker.current?.click()}>
            <Icon name="upload" className="size-4" />
            Add font files
          </Button>
          <Button variant="ghost" size="sm" onClick={onOpenPanel} aria-haspopup="dialog">
            Details
          </Button>
          <Button variant="ghost" size="sm" onClick={() => setDismissed(true)}>
            Keep the substitutes
          </Button>
          <input
            ref={picker}
            type="file"
            multiple
            accept={FONT_ACCEPT}
            aria-label="Missing font files"
            className="hidden"
            onChange={(event) => {
              const files = Array.from(event.currentTarget.files ?? []);
              event.currentTarget.value = '';
              if (files.length > 0) {
                void fonts.addFiles(files);
              }
            }}
          />
        </>
      }
    >
      {working ? (
        <p role="status">Adding the fonts…</p>
      ) : (
        <p>
          The slides draw {one ? 'it' : 'them'} with a substitute. {why(state, one, refused)}
        </p>
      )}
    </Banner>
  );
}
