// The wasm leg's page (budgets/wasm.rs): starts the app's own compile worker, waits for `ready`, and compiles the
// file in `#deck` as the app does with Google Fonts off: `open` with fonts, `finish`, the bundled substitute faces the
// deck draws added from the app's `fonts/core/` and a second `finish`, then the zip. Then it asks the worker for its
// memory. The outcome lands in `globalThis.__budgets`: `running`; `ready` once the module has loaded, waiting for
// `globalThis.__budgetsGo()`; then `done` with the wall-clock time and the memory, or `failed`.
function compileInWorker(workerUrl) {
  const state = { state: 'running' };
  globalThis.__budgets = state;
  const file = document.getElementById('deck')?.files?.[0];
  const worker = new Worker(workerUrl, { type: 'module' });
  const waiting = new Map();
  let next = 1;
  let started;
  const ready = new Promise((resolve, reject) => (started = { resolve, reject }));
  const fail = (message) => {
    for (const request of waiting.values()) {
      request.reject(new Error(message));
    }
    waiting.clear();
    started.reject(new Error(message));
  };
  worker.onmessage = ({ data }) => {
    if (data.type === 'ready') {
      started.resolve();
      return;
    }
    if (data.type === 'error' && data.id === null) {
      fail(data.error.message);
      return;
    }
    const request = waiting.get(data.id);
    if (request === undefined) {
      return;
    }
    if (data.type === 'memory') {
      request.bytes = data.bytes;
    } else if (data.type === 'deck') {
      request.substitutes = data.substitutes ?? [];
    } else if (data.type === 'done') {
      waiting.delete(data.id);
      request.resolve(request);
    } else if (data.type === 'error') {
      waiting.delete(data.id);
      request.reject(new Error(`${data.error.kind}: ${data.error.message}`));
    }
  };
  worker.onerror = (event) => {
    event.preventDefault();
    fail(event.message || 'the compile worker failed to start');
  };
  const send = (message) =>
    new Promise((resolve, reject) => {
      const id = next++;
      waiting.set(id, { resolve, reject, bytes: null, substitutes: [] });
      worker.postMessage({ ...message, id }, message.type === 'addFont' ? [message.bytes] : []);
    });
  /** The bundled files of the faces `wanted` names, as the app finds them in its substitutes manifest. */
  const substituteFiles = async (wanted) => {
    if (wanted.length === 0) {
      return [];
    }
    const manifest = await (await fetch('/fonts/core/core.json')).json();
    const files = [];
    for (const face of wanted) {
      const entry = manifest.faces.find(
        (f) =>
          f.family.toLowerCase() === face.family.toLowerCase() && f.weight === face.weight && f.style === face.style,
      );
      if (entry !== undefined && !files.includes(entry.file)) {
        files.push(entry.file);
      }
    }
    return Promise.all(files.map(async (file) => (await fetch(`/fonts/core/${file}`)).arrayBuffer()));
  };
  (async () => {
    if (file === undefined) {
      throw new Error('no deck file is set');
    }
    await ready;
    await new Promise((resolve) => {
      globalThis.__budgetsGo = resolve;
      state.state = 'ready';
    });
    state.state = 'running';
    const begun = performance.now();
    await send({ type: 'open', file });
    const { substitutes } = await send({ type: 'finish' });
    const faces = await substituteFiles(substitutes);
    for (const bytes of faces) {
      await send({ type: 'addFont', bytes, source: 'bundled' });
    }
    if (faces.length > 0) {
      await send({ type: 'finish' });
    }
    await send({
      type: 'bundle',
      settings: { singleFile: false, singleFileThreshold: null, leaveOut: [], include: [] },
    });
    const ms = performance.now() - begun;
    const { bytes } = await send({ type: 'memory' });
    if (typeof bytes !== 'number') {
      throw new Error('the worker reported no memory');
    }
    return { state: 'done', ms, peakBytes: bytes };
  })()
    .then(
      (outcome) => Object.assign(state, outcome),
      (error) => Object.assign(state, { state: 'failed', message: String(error?.message ?? error) }),
    )
    .finally(() => worker.terminate());
}
