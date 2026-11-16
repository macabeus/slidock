// The sample report's data: the synthetic bench of `fixtures.ts` in the shape `slidock-bench report` inlines, with
// every row of the latest and baseline runs, their images as `data:` URIs in the row details.
import { REPORT_SCHEMA, type ReportPayload, rowKey } from '../src/data/report.ts';
import { type Fixture, rowDetail, rowImage, rowImages } from './fixtures.ts';

export function samplePayload(fixture: Fixture): ReportPayload {
  const payload: ReportPayload = {
    schema: REPORT_SCHEMA,
    status: null,
    runs: fixture.runs,
    results: Object.fromEntries(fixture.results),
    parity: Object.fromEntries(fixture.parity),
    rows: {},
    images: {},
  };
  for (const run of [fixture.latest, fixture.baseline]) {
    const results = fixture.results.get(run);
    for (const id of Object.keys(results?.rows ?? {})) {
      const detail = rowDetail(fixture, run, id);
      if (!detail) {
        continue;
      }
      for (const kind of rowImages(fixture, run, id)) {
        const png = rowImage(fixture, run, id, kind);
        if (png) {
          const url = `data:image/png;base64,${png.toString('base64')}`;
          const served = detail.images[kind];
          detail.images[kind] = { url, from: served && 'from' in served ? served.from : 'run' };
        }
      }
      payload.rows[rowKey(run, id)] = detail;
    }
  }
  return payload;
}
