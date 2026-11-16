import { type PresenterWindow, createPresenterWindow } from '@slidock/player/presenter';
import { readHash } from '@slidock/ui';
import { useEffect, useRef, useState } from 'react';

import { MediaUrls } from '../deck/media';
import { DocumentFonts } from '../fonts/documentFonts';
import type { PlayerFactory } from '../player/types';
import { type PresenterDeck, presenterSessionOf, requestDeck } from './bridge';

/**
 * The presenter window: current and next slide, notes and timers, in step with the deck window that opened it. Its
 * frame (the header, the theme toggle, the status) is the one a downloaded deck's presenter window draws
 * (`@slidock/player/presenter`), so the two look and behave the same.
 */
export function PresenterApp({ player }: { player: PlayerFactory }) {
  const [received, setReceived] = useState<PresenterDeck | null>(null);
  const [orphan] = useState(() => !window.opener);
  const [session] = useState(() => presenterSessionOf(readHash()));
  const host = useRef<HTMLDivElement>(null);
  const [frame, setFrame] = useState<PresenterWindow | null>(null);
  const [documentFonts] = useState(() => new DocumentFonts());

  useEffect(() => () => documentFonts.clear(), [documentFonts]);

  useEffect(() => {
    const node = host.current;
    if (!node) {
      return;
    }
    const created = createPresenterWindow(node);
    setFrame(created);
    return () => {
      created.destroy();
      setFrame(null);
    };
  }, []);

  useEffect(() => {
    if (orphan) {
      return;
    }
    return requestDeck(setReceived, () => setReceived(null));
  }, [orphan]);

  useEffect(() => {
    if (!frame) {
      return;
    }
    frame.setTitle(received ? received.deck.title : null);
    frame.setStatus(
      orphan
        ? {
            kind: 'warning',
            title: 'Nothing to present here',
            text: 'Open the presenter view with the Presenter view button of a deck window.',
          }
        : received
          ? null
          : { kind: 'waiting', text: 'Waiting for the deck window…' },
    );
  }, [frame, received, orphan]);

  useEffect(() => {
    if (!received || !frame) {
      return;
    }
    documentFonts.replace(received.fonts);
    const media = new MediaUrls(received.media);
    const presenter = player.createPresenter(
      frame.main,
      { deck: received.deck, slides: received.slides, mediaUrl: media.url },
      { startAt: received.position, session },
    );
    return () => {
      presenter.destroy();
      media.revoke();
    };
  }, [received, frame, player, documentFonts, session]);

  return <div ref={host} className="h-dvh" />;
}
