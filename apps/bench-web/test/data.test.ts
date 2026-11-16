import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

import { DATA_DIR, loadFixture, rowDetail, rowImages, runKey } from '../mock/fixtures';
import { answer } from '../mock/plugin';
import { samplePayload } from '../mock/report';
import {
  REPORT_DATA_ID,
  REPORT_DATA_MARKER,
  REPORT_DATA_SLOT,
  checkEmptyReportPage,
  injectReportData,
  readInlineData,
  scriptSafeJson,
} from '../src/data/inline';
import { REPORT_SCHEMA, imageKey, parseReportPayload, reportSource, rowKey } from '../src/data/report';
import { API, ApiError } from '../src/data/source';
import type { Results, RunsIndex } from '../src/schema';
import { fixture, mockSource } from './fixture';

function page(html: string): Document {
  return new DOMParser().parseFromString(html, 'text/html');
}

describe('the report data slot', () => {
  const template = `<!doctype html><html><head>${REPORT_DATA_SLOT}</head><body></body></html>`;

  it('reads as empty until filled, and as absent on the served app', () => {
    expect(readInlineData(page(template))).toEqual({ kind: 'empty' });
    expect(readInlineData(page('<!doctype html><html><head></head></html>'))).toEqual({ kind: 'none' });
    const broken = template.replace(REPORT_DATA_MARKER, '{"schema":');
    expect(readInlineData(page(broken))).toEqual({ kind: 'invalid' });
  });

  it('carries any text, a closing script tag included, through the page', () => {
    const value = { text: '</script><script>alert(1)</script> <!-- \u2028 \u2029 & é', n: 0.9707 };
    const json = scriptSafeJson(value);
    expect(json).not.toContain('<');
    expect(json).not.toContain('\u2028');
    const filled = injectReportData(template, value);
    const doc = page(filled);
    expect(doc.querySelectorAll('script')).toHaveLength(1);
    expect(readInlineData(doc)).toEqual({ kind: 'data', value });
  });

  it('names the marker slidock-bench report fills, and checks an unfilled page holds it only in the slot', () => {
    expect(REPORT_DATA_MARKER).toBe('__SLIDOCK_BENCH_DATA__');
    expect(() => checkEmptyReportPage(template)).not.toThrow();
    const bundled = template.replace('</body>', `<script>const m = "${REPORT_DATA_MARKER}";</script></body>`);
    expect(() => checkEmptyReportPage(bundled)).toThrow('markers: 2, slots: 1');
    expect(() => checkEmptyReportPage(template + REPORT_DATA_SLOT)).toThrow('slots: 2');
    expect(() => checkEmptyReportPage(injectReportData(template, {}))).toThrow('markers: 0');
  });

  it('refuses a page without exactly one slot', () => {
    expect(() => injectReportData('<html></html>', {})).toThrow(REPORT_DATA_ID);
    expect(() => injectReportData(template + REPORT_DATA_SLOT, {})).toThrow(REPORT_DATA_ID);
  });
});

describe('the report payload', () => {
  const payload = samplePayload(fixture);

  it('checks its envelope', () => {
    expect(parseReportPayload(JSON.parse(JSON.stringify(payload)))).toEqual(payload);
    expect(() => parseReportPayload({ ...payload, schema: 'other@1' })).toThrow(REPORT_SCHEMA);
    expect(() => parseReportPayload({ ...payload, runs: {} })).toThrow('runs index');
    expect(() => parseReportPayload({ ...payload, rows: null })).toThrow('rows');
    expect(() => parseReportPayload({ ...payload, public: { images: 'mismatch' } })).toThrow('public section');
    expect(parseReportPayload({ ...payload, public: { images: ['mismatch'] } }).public).toEqual({
      images: ['mismatch'],
    });
  });

  it('serves runs, results, rows and images by key', async () => {
    const source = reportSource(payload);
    expect((await source.runs()).runs).toHaveLength(fixture.runs.runs.length);
    expect(await source.results(fixture.latest)).toBe(fixture.results.get(fixture.latest));
    expect(await source.results('missing')).toBeNull();
    expect(await source.status()).toBeNull();
    const id = Object.keys(fixture.results.get(fixture.latest)?.rows ?? {})[0] ?? '';
    const detail = await source.row(fixture.latest, id);
    expect(detail?.id).toBe(id);
    const oracle = detail?.images.oracle;
    expect(oracle && 'url' in oracle && oracle.url.startsWith('data:image/png;base64,')).toBe(true);
    expect(source.imageUrl(fixture.latest, id, 'oracle')).toBeNull();
    const withImage = reportSource({ ...payload, images: { [imageKey('k', 'r', 'render')]: 'renders/r.png' } });
    expect(withImage.imageUrl('k', 'r', 'render')).toBe('renders/r.png');
    expect(rowKey('k', 'r')).toBe('k r');
    expect(source.scope).toEqual({ parity: true, public: null });
    expect(reportSource({ ...payload, parity: {}, public: { images: ['mismatch'] } }).scope).toEqual({
      parity: false,
      public: { images: ['mismatch'] },
    });
  });
});

describe('the API source', () => {
  it('reads the runs, maps 404 to null and fails on other errors', async () => {
    const { source, requests } = mockSource();
    expect((await source.runs()).schema).toBe('slidock-bench-runs@1');
    expect(await source.results('0'.repeat(64))).toBeNull();
    expect((await source.status())?.head.content).toBe(fixture.results.get(fixture.latest)?.stamp.compiler.content);
    expect(requests).toEqual([API.runs(), API.results('0'.repeat(64)), API.status()]);
    const failing = (status: number) => async () => new Response('', { status });
    const { apiSource } = await import('../src/data/source');
    await expect(apiSource(failing(500)).results('x')).rejects.toBeInstanceOf(ApiError);
    await expect(apiSource(failing(404)).runs()).rejects.toThrow('answered 404');
  });

  it('encodes row ids in paths', () => {
    expect(API.row('k', 'demo:kn-1:keynote#s1')).toBe('api/runs/k/rows/demo%3Akn-1%3Akeynote%23s1');
    expect(API.png('k', 'demo:od-p1:odp#t@250', 'heatmap')).toBe('api/png/k/demo%3Aod-p1%3Aodp%23t%40250/heatmap');
  });
});

describe('the dev mock', () => {
  it("names each results file by its stamp's run key, as the runs index does", () => {
    const read = (name: string) => JSON.parse(readFileSync(`${DATA_DIR}${name}`, 'utf8')) as Results;
    const index = JSON.parse(readFileSync(`${DATA_DIR}runs.json`, 'utf8')) as RunsIndex;
    const keys = new Set(index.runs.map((run) => run.key));
    for (const name of ['results.json', 'baseline-results.json', 'motion-results.json']) {
      expect(keys.has(runKey(read(name).stamp)), name).toBe(true);
    }
  });

  it('adds the runs the index lacks after its newest, oldest first', () => {
    const loaded = loadFixture();
    const times = loaded.runs.runs.map((run) => run.recordedAt);
    expect([...times].sort()).toEqual(times);
    expect(new Set(loaded.runs.runs.map((run) => run.lane))).toEqual(new Set(['static', 'host', 'fonts', 'motion']));
    expect(loaded.runs.runs.every((run) => loaded.results.has(run.key) || run.lane === 'static')).toBe(true);
  });

  it('answers the API paths and 404s the rest', () => {
    const id = 'demo-basics:od-p901:odp';
    const ok = (path: string) => answer(fixture, path).status;
    expect(ok('/runs')).toBe(200);
    expect(ok('/runs/latest/results')).toBe(200);
    expect(ok(`/runs/latest/rows/${encodeURIComponent(id)}`)).toBe(200);
    expect(ok(`/runs/${fixture.latest}/parity`)).toBe(200);
    expect(ok(`/runs/${fixture.baseline}/parity`)).toBe(404);
    expect(ok(`/png/latest/${encodeURIComponent(id)}/heatmap`)).toBe(200);
    expect(ok(`/png/latest/${encodeURIComponent(id)}/thumbnail`)).toBe(404);
    expect(ok('/runs/latest/rows/nope')).toBe(404);
    expect(ok('/secrets')).toBe(404);
    const png = answer(fixture, `/png/latest/${encodeURIComponent(id)}/oracle`).body;
    expect(Buffer.isBuffer(png) && png.subarray(1, 4).toString('ascii')).toBe('PNG');
  });

  it('gives a row without a render the command that produces it', () => {
    const failed = 'demo-basics:od-p904:odp';
    expect(rowImages(fixture, fixture.latest, failed)).toEqual(['oracle']);
    expect(rowDetail(fixture, fixture.latest, failed)?.images.render).toEqual({
      missing: `slidock-bench run --only ${failed}`,
    });
  });
});
