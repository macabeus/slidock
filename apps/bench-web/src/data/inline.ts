// The single-file report's data slot. `vite build --mode report` writes an empty slot into the page's head:
//
//   <script type="application/json" id="slidock-bench-data">__SLIDOCK_BENCH_DATA__</script>
//
// and `slidock-bench report` replaces that slot element with one holding the report's JSON, every `<` written as
// `\u003c` so no text in it can close the script element. The marker text is in the page only there: the page's code
// builds it from the id, and the report build fails when the marker appears anywhere else, so replacing the marker
// alone works too. This module is shared by the build (which can fill the slot from a file) and the page (which reads
// it).

export const REPORT_DATA_ID = 'slidock-bench-data';
/** `__SLIDOCK_BENCH_DATA__`, built from the id so the bundled page code does not hold the marker text. */
export const REPORT_DATA_MARKER = `__${REPORT_DATA_ID.toUpperCase().replaceAll('-', '_')}__`;

/** The empty slot the report build writes. */
export const REPORT_DATA_SLOT = `<script type="application/json" id="${REPORT_DATA_ID}">${REPORT_DATA_MARKER}</script>`;

/** JSON that is safe inside a `<script>` element: `<`, U+2028 and U+2029 are escaped. */
export function scriptSafeJson(value: unknown): string {
  return JSON.stringify(value)
    .replaceAll('<', '\\u003c')
    .replaceAll('\u2028', '\\u2028')
    .replaceAll('\u2029', '\\u2029');
}

/** Fills a report page's empty slot with `value`. Throws when the page has no slot, or more than one. */
export function injectReportData(html: string, value: unknown): string {
  const first = html.indexOf(REPORT_DATA_SLOT);
  if (first < 0 || html.indexOf(REPORT_DATA_SLOT, first + 1) >= 0) {
    throw new Error(`the page must hold exactly one ${REPORT_DATA_ID} slot`);
  }
  const filled = REPORT_DATA_SLOT.replace(REPORT_DATA_MARKER, () => scriptSafeJson(value));
  return html.slice(0, first) + filled + html.slice(first + REPORT_DATA_SLOT.length);
}

/** Throws unless an unfilled report page holds the marker text exactly once, inside its one slot. */
export function checkEmptyReportPage(html: string): void {
  const markers = html.split(REPORT_DATA_MARKER).length - 1;
  const slots = html.split(REPORT_DATA_SLOT).length - 1;
  if (markers !== 1 || slots !== 1) {
    throw new Error(
      `the report page must hold ${REPORT_DATA_MARKER} once, in its ${REPORT_DATA_ID} slot (markers: ${markers}, slots: ${slots})`,
    );
  }
}

export type InlineData = { kind: 'none' } | { kind: 'empty' } | { kind: 'data'; value: unknown } | { kind: 'invalid' };

/** What the page's slot holds: no slot (the served app), the unfilled marker, the report's data, or broken JSON. */
export function readInlineData(doc: Document): InlineData {
  const node = doc.getElementById(REPORT_DATA_ID);
  if (!node) {
    return { kind: 'none' };
  }
  const text = (node.textContent ?? '').trim();
  if (text === REPORT_DATA_MARKER) {
    return { kind: 'empty' };
  }
  try {
    return { kind: 'data', value: JSON.parse(text) as unknown };
  } catch {
    return { kind: 'invalid' };
  }
}
