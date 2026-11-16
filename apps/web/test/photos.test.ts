import type { DeckDiagnostic } from '@slidock/types';
import { describe, expect, it } from 'vitest';

import { DeckSession, type View } from '../src/compile/session';
import { decodePhotos, explainPhotos } from '../src/media/needs';
import {
  type DecodePort,
  type DecodeReply,
  type DecodeRequest,
  PhotoUndecodable,
  workerPhotoDecoder,
} from '../src/media/photos';
import { testFontsEnvironment } from '../src/testing/fake-fonts';
import { decodedFile, fakePhotoDecoder, gridPlan, mediaNeed } from '../src/testing/fake-photos';
import { FakeWorker } from '../src/testing/fake-worker';
import { sampleDeck } from '../src/testing/sample-deck';
import { sampleFontsScript } from '../src/testing/sample-fonts';
import { testNetwork } from './fonts/network';

const A = 'a'.repeat(64);
const B = 'b'.repeat(64);
const C = 'c'.repeat(64);

function omitted(id: string): DeckDiagnostic {
  return {
    code: 'MID5601',
    severity: 'decline',
    message: `Media ${id} is image/heic, which the output cannot show`,
    feature: null,
    container: 's1',
    node: null,
  };
}

describe('decodePhotos', () => {
  it('decodes each planned photo in order, adds it, and says what became of the others', async () => {
    const plan = gridPlan(2, 1);
    const decoder = fakePhotoDecoder((need) => (need.id === B ? ['no HEVC here'] : decodedFile(need.plan)));
    const added: string[] = [];
    const progress: number[] = [];
    const result = await decodePhotos(
      [mediaNeed(A, plan), mediaNeed(B, plan), mediaNeed(C, null, 'its primary image is a `iovl` item')],
      decoder,
      async (id, bytes) => {
        added.push(`${id.slice(0, 1)} ${bytes.byteLength}`);
      },
      (done) => progress.push(done),
    );
    expect(decoder.decoded).toEqual([A, B]);
    expect(added).toEqual([`a ${decodedFile(plan).bytes.byteLength}`]);
    expect(progress).toEqual([1, 2]);
    expect(result.outcomes).toEqual([
      { id: A, state: 'decoded', path: 'webcodecs', elapsed: 5 },
      { id: B, state: 'undecodable', reasons: ['no HEVC here'] },
      { id: C, state: 'unplanned', reason: 'its primary image is a `iovl` item' },
    ]);
    expect([...result.files.keys()]).toEqual([A]);
  });

  it('keeps a photo the compiler refuses omitted, with its reason, and stops when the deck is gone', async () => {
    const plan = gridPlan(1, 1);
    const refused = await decodePhotos(
      [mediaNeed(A, plan)],
      fakePhotoDecoder(() => decodedFile(plan)),
      () => Promise.reject(new Error('the decoded file is 2 × 2')),
    );
    expect(refused.outcomes).toEqual([{ id: A, state: 'refused', reason: 'the decoded file is 2 × 2' }]);
    expect(refused.files.size).toBe(0);
    const stopped = await decodePhotos(
      [mediaNeed(A, plan)],
      fakePhotoDecoder(() => decodedFile(plan)),
      () => Promise.resolve(),
      () => {},
      () => false,
    );
    expect(stopped.outcomes).toEqual([]);
  });

  it('never adds a photo decoded once the deck is gone', async () => {
    const plan = gridPlan(1, 1);
    let open = true;
    const added: string[] = [];
    const decoder = fakePhotoDecoder((need) => {
      // The user closes the deck while this photo decodes.
      open = false;
      return need.id === A ? decodedFile(plan) : ['stopped'];
    });
    const result = await decodePhotos(
      [mediaNeed(A, plan), mediaNeed(B, plan)],
      decoder,
      async (id) => {
        added.push(id);
      },
      () => {},
      () => open,
    );
    expect(added).toEqual([]);
    expect(decoder.decoded).toEqual([A]);
    expect(result.outcomes).toEqual([]);
  });
});

describe('explainPhotos', () => {
  it('says what to do about each photo left out, and leaves the other diagnostics as they are', () => {
    const other = { ...omitted('d'.repeat(64)), message: 'Media dd… is image/emf, which the output cannot show' };
    const explained = explainPhotos(
      [omitted(A), omitted(B), omitted(C), other],
      [
        { id: A, state: 'undecodable', reasons: ['no'] },
        { id: B, state: 'unplanned', reason: 'its primary image is a `iovl` item' },
        { id: C, state: 'decoded', path: 'native', elapsed: 1 },
      ],
    );
    expect(explained.map((diagnostic) => diagnostic.message)).toEqual([
      `Media ${A} is a HEIC photo, which this browser cannot decode: open the deck in Chrome or Safari, or export it to PPTX from Keynote, which converts its photos to JPEG`,
      `Media ${B} is a HEIC photo slidock cannot decode (its primary image is a \`iovl\` item): export the deck to PPTX from Keynote, which converts its photos to JPEG`,
      omitted(C).message,
      other.message,
    ]);
    expect(explained.every((diagnostic) => diagnostic.severity === 'decline')).toBe(true);
  });

  it("follows a photo drawn from its preview with why and what to do, and leaves a decoded one's as it is", () => {
    const preview = (id: string): DeckDiagnostic => ({
      ...omitted(id),
      code: 'MID5609',
      severity: 'lossy',
      message: `Media ${id} is drawn from its 256×192 preview: the 1600×1200 HEIC original is not decoded here`,
    });
    const explained = explainPhotos(
      [preview(A), preview(B)],
      [
        { id: A, state: 'undecodable', reasons: ['no'] },
        { id: B, state: 'decoded', path: 'webcodecs', elapsed: 1 },
      ],
    );
    expect(explained.map((diagnostic) => diagnostic.message)).toEqual([
      `${preview(A).message}. This browser cannot decode it: open the deck in Chrome or Safari, or export it to PPTX from Keynote, which converts its photos to JPEG`,
      preview(B).message,
    ]);
  });
});

describe('workerPhotoDecoder', () => {
  /** A decode worker that answers each request from `answer`. */
  function fakePort(answer: (request: DecodeRequest) => DecodeReply | 'crash' | 'silent') {
    const port: DecodePort & { requests: DecodeRequest[]; terminated: number } = {
      requests: [],
      terminated: 0,
      onmessage: null,
      onerror: null,
      postMessage(message) {
        port.requests.push(message);
        const reply = answer(message);
        if (reply === 'silent') {
          return;
        }
        setTimeout(() => {
          if (reply === 'crash') {
            port.onerror?.({ message: 'boom', preventDefault() {} } as ErrorEvent);
          } else {
            port.onmessage?.({ data: reply } as MessageEvent<DecodeReply>);
          }
        }, 0);
      },
      terminate() {
        port.terminated += 1;
      },
    };
    return port;
  }

  it('sends the plan and file to one worker and resolves or rejects each photo', async () => {
    const plan = gridPlan(1, 1);
    let starts = 0;
    const port = fakePort(({ id }) =>
      id === 1
        ? { id, ok: true, bytes: new ArrayBuffer(3), mime: 'image/jpeg', path: 'native', elapsed: 2 }
        : { id, ok: false, reasons: ['no'] },
    );
    const decoder = workerPhotoDecoder(() => {
      starts += 1;
      return port;
    });
    const need = { ...mediaNeed(A, plan), plan };
    await expect(decoder.decode(need)).resolves.toMatchObject({ path: 'native', elapsed: 2 });
    await expect(decoder.decode(need)).rejects.toMatchObject({ reasons: ['no'] });
    expect(starts).toBe(1);
    expect(port.requests.map(({ id, plan: sent }) => [id, sent === plan])).toEqual([
      [1, true],
      [2, true],
    ]);
    decoder.dispose();
    expect(port.terminated).toBe(1);
  });

  it('gives up on a photo that takes longer than its timeout, stopping its worker', async () => {
    const plan = gridPlan(1, 1);
    const fired: (() => void)[] = [];
    const cleared: unknown[] = [];
    const timers = {
      setTimeout: (run: () => void, ms: number) => {
        expect(ms).toBe(5_000);
        fired.push(run);
        return fired.length;
      },
      clearTimeout: (timer: unknown) => cleared.push(timer),
    };
    const ports: ReturnType<typeof fakePort>[] = [];
    const decoder = workerPhotoDecoder(
      () => {
        // The first worker never answers; the next one does.
        const index = ports.length;
        const port = fakePort(({ id }) =>
          index === 0
            ? 'silent'
            : { id, ok: true, bytes: new ArrayBuffer(1), mime: 'image/jpeg', path: 'native', elapsed: 1 },
        );
        ports.push(port);
        return port;
      },
      5_000,
      timers,
    );
    const need = { ...mediaNeed(A, plan), plan };
    const hung = decoder.decode(need);
    fired[0]!();
    await expect(hung).rejects.toMatchObject({ reasons: ['decoding took longer than 5 s'] });
    expect(ports[0]?.terminated).toBe(1);
    await expect(decoder.decode(need)).resolves.toMatchObject({ path: 'native' });
    expect(ports).toHaveLength(2);
    expect(cleared).toContain(2);
  });

  it('fails the pending photos when the worker crashes, and starts another for the next', async () => {
    const plan = gridPlan(1, 1);
    let starts = 0;
    const decoder = workerPhotoDecoder(() => {
      starts += 1;
      return fakePort(({ id }) =>
        starts === 1
          ? 'crash'
          : { id, ok: true, bytes: new ArrayBuffer(1), mime: 'image/jpeg', path: 'webcodecs', elapsed: 1 },
      );
    });
    const need = { ...mediaNeed(A, plan), plan };
    await expect(decoder.decode(need)).rejects.toMatchObject({ reasons: ['the decode worker failed (boom)'] });
    await expect(decoder.decode(need)).resolves.toMatchObject({ path: 'webcodecs' });
    expect(starts).toBe(2);
  });
});

describe('DeckSession with HEIC photos', () => {
  function session(worker: FakeWorker, decoder: ReturnType<typeof fakePhotoDecoder>) {
    const views: View[] = [];
    const deckSession = new DeckSession(
      () => worker,
      testFontsEnvironment(),
      () => decoder,
    );
    deckSession.subscribe(() => views.push(deckSession.getSnapshot().view));
    deckSession.connect();
    return { deckSession, views };
  }

  it('decodes the photos after open, adds them before finish, and shows the progress', async () => {
    const plan = gridPlan(2, 1);
    const worker = new FakeWorker({ deck: sampleDeck(), mediaNeeds: [mediaNeed(A, plan), mediaNeed(B, plan)] });
    const decoder = fakePhotoDecoder((need) => decodedFile(need.plan));
    const { deckSession, views } = session(worker, decoder);
    await deckSession.openFile(new File(['x'], 'photos.key'));
    const view = deckSession.getSnapshot().view;
    expect(view.kind).toBe('deck');
    // addMedia twice, between open and finish.
    expect(worker.sent.map(({ type }) => type)).toEqual(['open', 'addMedia', 'addMedia', 'finish']);
    expect(worker.media.map(({ need }) => need)).toEqual([A, B]);
    expect(decoder.disposed).toBe(1);
    const shown = views.flatMap((v) => (v.kind === 'compiling' && v.photos ? [v.photos] : []));
    expect(shown[0]).toEqual({ total: 2, done: 0, active: true });
    expect(shown).toContainEqual({ total: 2, done: 1, active: true });
    expect(shown.at(-1)).toEqual({ total: 2, done: 2, active: false });
    if (view.kind === 'deck') {
      expect(view.photos.map(({ state }) => state)).toEqual(['decoded', 'decoded']);
    }
  });

  it('leaves a photo this browser cannot decode omitted, its diagnostic saying what to do', async () => {
    const plan = gridPlan(1, 1);
    const worker = new FakeWorker({ deck: sampleDeck(), mediaNeeds: [mediaNeed(A, plan)], diagnostics: [omitted(A)] });
    const { deckSession, views } = session(
      worker,
      fakePhotoDecoder(() => ['no HEVC decoder']),
    );
    await deckSession.openFile(new File(['x'], 'photos.key'));
    const view = deckSession.getSnapshot().view;
    expect(worker.sent.map(({ type }) => type)).toEqual(['open', 'finish']);
    if (view.kind !== 'deck') {
      throw new Error('no deck');
    }
    const message = view.diagnostics.find(({ code }) => code === 'MID5601')?.message;
    expect(message).toContain('this browser cannot decode: open the deck in Chrome or Safari');
    expect(views.some((v) => v.kind === 'compiling' && v.photos?.total === 1)).toBe(true);
  });

  it('shows no decoding step for a deck without photos', async () => {
    const plain = new FakeWorker({ deck: sampleDeck() });
    const { deckSession, views } = session(
      plain,
      fakePhotoDecoder(() => ['unused']),
    );
    await deckSession.openFile(new File(['x'], 'plain.key'));
    expect(views.every((v) => v.kind !== 'compiling' || v.photos === null)).toBe(true);
    expect(plain.sent.map(({ type }) => type)).toEqual(['open', 'finish']);
  });

  it('stops a running decode on a cancel or another file, never running two decoders at once', async () => {
    const plan = gridPlan(1, 1);
    // A new compile worker for each load, as a load during a compile restarts it.
    const workers: FakeWorker[] = [];
    let live = 0;
    let most = 0;
    const started: (() => void)[] = [];
    const decoders: { disposed: number }[] = [];
    // Each decoder's photo decodes until the decoder is disposed, as a worker does until it is stopped.
    const deckSession = new DeckSession(
      () => {
        const worker = new FakeWorker({ deck: sampleDeck(), mediaNeeds: [mediaNeed(A, plan)] });
        workers.push(worker);
        return worker;
      },
      testFontsEnvironment(),
      () => {
        live += 1;
        most = Math.max(most, live);
        let fail: (error: Error) => void = () => {};
        const decoder = {
          disposed: 0,
          decode: () =>
            new Promise<never>((_, reject) => {
              fail = reject;
              started.shift()?.();
            }),
          dispose: () => {
            if (decoder.disposed === 0) {
              live -= 1;
            }
            decoder.disposed += 1;
            fail(new PhotoUndecodable(['the decode worker was stopped']));
          },
        };
        decoders.push(decoder);
        return decoder;
      },
    );
    deckSession.connect();
    const decoding = () => new Promise<void>((resolve) => started.push(resolve));

    let next = decoding();
    const first = deckSession.openFile(new File(['x'], 'one.key'));
    await next;
    next = decoding();
    const second = deckSession.openFile(new File(['x'], 'two.key'));
    await next;
    // The first decode was stopped before the second decoder started.
    expect(decoders[0]?.disposed).toBeGreaterThan(0);
    expect(most).toBe(1);
    deckSession.cancel();
    await Promise.all([first, second]);
    expect(decoders[1]?.disposed).toBeGreaterThan(0);
    expect(live).toBe(0);
    expect(workers.flatMap((worker) => worker.sent.map(({ type }) => type))).not.toContain('addMedia');
    expect(deckSession.getSnapshot().view.kind).toBe('empty');
  });

  it('adds the decoded photos again to a deck the fonts flow reopens', async () => {
    const plan = gridPlan(1, 1);
    const worker = new FakeWorker(sampleFontsScript({ mediaNeeds: [mediaNeed(A, plan)] }));
    const network = testNetwork();
    const decoder = fakePhotoDecoder(() => decodedFile(plan));
    const deckSession = new DeckSession(
      () => worker,
      testFontsEnvironment({ fetch: network.fetch }),
      () => decoder,
    );
    deckSession.connect();
    await deckSession.openFile(new File(['x'], 'photos.key'));
    const fonts = deckSession.getSnapshot().fonts!;
    await fonts.start();
    expect(worker.media.map(({ need }) => need)).toEqual([A]);
    // Turning Google Fonts off once its faces arrived opens the deck again: the photo is added again, not decoded.
    await fonts.setGoogle(false);
    const types = worker.sent.map(({ type }) => type);
    const reopened = types.lastIndexOf('open');
    expect(reopened).toBeGreaterThan(0);
    expect(types.slice(reopened, reopened + 2)).toEqual(['open', 'addMedia']);
    expect(worker.media.map(({ need }) => need)).toEqual([A]);
    expect(decoder.decoded).toEqual([A]);
  });
});
