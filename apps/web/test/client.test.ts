import type { FromWorker, ToWorker } from '@slidock/types';
import { describe, expect, it, vi } from 'vitest';

import { CompileClient, type CompilerStatus, RequestError } from '../src/compile/client';
import type { WorkerPort } from '../src/compile/port';
import { sampleDeck } from '../src/testing/sample-deck';

/** A port the test drives by hand. */
function manualPort() {
  const sent: ToWorker[] = [];
  let receive: (message: FromWorker) => void = () => {};
  let crash: (message: string) => void = () => {};
  const port: WorkerPort & { terminated: boolean } = {
    terminated: false,
    post: (message) => sent.push(message),
    listen(onMessage, onCrash) {
      receive = onMessage;
      crash = onCrash;
    },
    terminate() {
      port.terminated = true;
    },
  };
  return {
    port,
    sent,
    receive: (message: FromWorker) => receive(message),
    crash: (message: string) => crash(message),
  };
}

const file = new File(['x'], 'Deck.pptx');

describe('CompileClient', () => {
  it('holds requests until the worker is ready, then sends them in order with fresh ids', async () => {
    const { port, sent, receive } = manualPort();
    const statuses: CompilerStatus[] = [];
    const client = new CompileClient(port, (status) => statuses.push(status));
    const open = client.open(file);
    const finish = client.finish();
    expect(sent).toEqual([]);
    receive({ type: 'ready', version: '1.2.3' });
    expect(statuses).toEqual([{ state: 'ready', version: '1.2.3' }]);
    expect(sent).toEqual([
      { type: 'open', id: 1, file },
      { type: 'finish', id: 2 },
    ]);
    receive({ type: 'done', id: 1, report: null });
    await expect(open).resolves.toBeUndefined();
    const { deck, slides, media } = sampleDeck();
    receive({ type: 'deck', id: 2, deck, slides, media });
    receive({ type: 'done', id: 2, report: null });
    await expect(finish).resolves.toEqual({ deck, slides, media });
  });

  it("routes progress, diagnostics and needs to their request's handlers only", async () => {
    const { port, receive } = manualPort();
    const client = new CompileClient(port);
    receive({ type: 'ready', version: '1' });
    const onProgress = vi.fn();
    const onDiagnostics = vi.fn();
    const onNeeds = vi.fn();
    const open = client.open(file, { onProgress, onDiagnostics, onNeeds });
    receive({ type: 'progress', id: 1, phase: 'open' });
    receive({ type: 'progress', id: 99, phase: 'finish' });
    receive({ type: 'diagnostics', id: 1, diagnostics: [] });
    receive({ type: 'needs', id: 1, needs: [] });
    receive({ type: 'done', id: 1, report: null });
    await open;
    expect(onProgress.mock.calls).toEqual([['open']]);
    expect(onDiagnostics).toHaveBeenCalledTimes(1);
    expect(onNeeds).toHaveBeenCalledTimes(1);
  });

  it('rejects a request with its error', async () => {
    const { port, receive } = manualPort();
    const client = new CompileClient(port);
    receive({ type: 'ready', version: '1' });
    const open = client.open(file);
    const error = { kind: 'refused' as const, message: 'not a zip', diagnostics: [] };
    receive({ type: 'error', id: 1, error });
    await expect(open).rejects.toEqual(new RequestError(error));
    await open.catch((caught: unknown) => expect((caught as RequestError).error).toBe(error));
  });

  it('a finish that ends without a deck is an internal error', async () => {
    const { port, receive } = manualPort();
    const client = new CompileClient(port);
    receive({ type: 'ready', version: '1' });
    const finish = client.finish();
    receive({ type: 'done', id: 1, report: null });
    await expect(finish).rejects.toThrow(/without a deck/);
  });

  it('streams bundle chunks in order and resolves with the report', async () => {
    const { port, receive } = manualPort();
    const client = new CompileClient(port);
    receive({ type: 'ready', version: '1' });
    const chunks: number[] = [];
    const bundle = client.bundle({ singleFile: false, singleFileThreshold: null, leaveOut: [], include: [] }, (bytes) =>
      chunks.push(bytes.byteLength),
    );
    receive({ type: 'zipChunk', id: 1, bytes: new ArrayBuffer(3) });
    receive({ type: 'zipChunk', id: 1, bytes: new ArrayBuffer(5) });
    const report = { files: [{ path: 'index.html', bytes: 8 }], bytes: 8, singleFile: false, fontsLeftOut: [] };
    receive({ type: 'done', id: 1, report });
    await expect(bundle).resolves.toEqual(report);
    expect(chunks).toEqual([3, 5]);
  });

  it('a load failure fails the status, queued requests and later ones', async () => {
    const { port, receive } = manualPort();
    const statuses: CompilerStatus[] = [];
    const client = new CompileClient(port, (status) => statuses.push(status));
    const open = client.open(file);
    receive({ type: 'error', id: null, error: { kind: 'internal', message: 'no wasm', diagnostics: [] } });
    expect(statuses).toEqual([{ state: 'failed', message: 'no wasm' }]);
    await expect(open).rejects.toThrow('no wasm');
    await expect(client.finish()).rejects.toThrow('no wasm');
  });

  it('a crash fails the requests in flight', async () => {
    const { port, receive, crash } = manualPort();
    const client = new CompileClient(port);
    receive({ type: 'ready', version: '1' });
    const open = client.open(file);
    crash('worker script error');
    await expect(open).rejects.toThrow('worker script error');
    expect(client.status).toEqual({ state: 'failed', message: 'worker script error' });
  });

  it('terminate stops the worker and cancels pending requests', async () => {
    const { port, receive } = manualPort();
    const client = new CompileClient(port);
    receive({ type: 'ready', version: '1' });
    const open = client.open(file);
    client.terminate();
    expect(port.terminated).toBe(true);
    await open.catch((caught: unknown) => expect((caught as RequestError).error.kind).toBe('cancelled'));
    await expect(open).rejects.toBeInstanceOf(RequestError);
  });
});
