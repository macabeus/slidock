import type { CompiledDeck } from '../compile/client';

// A small synthetic deck in the shape the worker hands over: four slides (one hidden) with notes, a build step, an
// image and two font faces. Tests and the dev demo page use it; its content is made up for slidock.

const W = 720;
const H = 405;
const IMAGE_SHA = 'a'.repeat(64);
export const SAMPLE_IMAGE_PATH = `media/${IMAGE_SHA}.svg`;

const SAMPLE_SVG =
  '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 160 90"><rect width="160" height="90" rx="8" fill="#38bdf8"/>' +
  '<circle cx="50" cy="45" r="22" fill="#fbbf24"/><rect x="88" y="26" width="48" height="38" rx="4" fill="#0f172a"/></svg>';

function slide(id: string, background: string, body: string): string {
  return (
    `<main data-slide="${id}" style="position:relative;width:${W}pt;height:${H}pt;overflow:hidden">` +
    `<div data-sd-bg="" style="position:absolute;left:0pt;top:0pt;width:${W}pt;height:${H}pt;background:${background}"></div>` +
    body +
    '</main>'
  );
}

function text(id: string, left: number, top: number, width: number, size: number, color: string, content: string) {
  return (
    `<div data-el="${id}" data-kind="shape" style="position:absolute;left:${left}pt;top:${top}pt;width:${width}pt;height:${size * 2}pt">` +
    `<div data-sd-text="" style="position:absolute;left:0pt;top:0pt;width:100%;height:100%;font-family:&quot;Liberation Sans&quot;,sans-serif">` +
    `<p style="line-height:1.2"><span style="font-size:${size}pt;color:${color}">${content}</span></p></div></div>`
  );
}

export function sampleDeck(): CompiledDeck {
  const slides = [
    {
      id: 's1',
      html: slide(
        's1',
        'linear-gradient(135deg,#1e1b4b,#831843)',
        text('s1.title', 48, 140, 624, 44, '#ffffff', 'Quarterly review') +
          text('s1.sub', 48, 220, 624, 20, '#fbcfe8', 'Made with slidock'),
      ),
    },
    {
      id: 's2',
      html: slide(
        's2',
        '#ffffff',
        text('s2.title', 48, 32, 624, 32, '#0f172a', 'Highlights') +
          '<div data-el="s2.list" data-kind="shape" style="position:absolute;left:48pt;top:110pt;width:360pt;height:220pt">' +
          '<div data-sd-text="" style="position:absolute;left:0pt;top:0pt;width:100%;height:100%;font-family:&quot;Liberation Sans&quot;,sans-serif">' +
          '<ul style="margin:0;padding-left:18pt;font-size:20pt;color:#334155;line-height:1.5">' +
          '<li data-p="0">Shipped the viewer</li><li data-p="1">Fonts in every download</li><li data-p="2">Presenter view</li>' +
          '</ul></div></div>' +
          `<img data-el="s2.photo" data-kind="picture" alt="A chart" src="${SAMPLE_IMAGE_PATH}" style="position:absolute;left:432pt;top:120pt;width:240pt;height:135pt">`,
      ),
    },
    {
      id: 's3',
      html: slide('s3', '#fef3c7', text('s3.title', 48, 160, 624, 36, '#78350f', 'Backup numbers (hidden)')),
    },
    {
      id: 's4',
      html: slide('s4', '#0f172a', text('s4.title', 48, 160, 624, 40, '#f472b6', 'Thank you')),
    },
  ];
  return {
    deck: {
      format: 'slidock-deck@1',
      player: null,
      id: 'c0ffee'.padEnd(64, '0'),
      title: 'Quarterly review',
      lang: 'en',
      size: { w: W, h: H },
      source: { format: 'pptx', name: 'Quarterly review.pptx', sha256: 'b'.repeat(64), bytes: 48213 },
      slides: [
        {
          id: 's1',
          layout: null,
          hidden: false,
          number: 1,
          title: 'Quarterly review',
          notes: ['Welcome everyone.', 'Keep this under a minute.'],
          failed: null,
          timeline: [],
          transition: null,
          fonts: [0],
        },
        {
          id: 's2',
          layout: null,
          hidden: false,
          number: 2,
          title: 'Highlights',
          notes: ['Pause after each bullet.'],
          failed: null,
          timeline: [
            {
              target: { element: 's2.list', paragraphs: [0, 2] },
              trigger: 'click',
              kind: 'entrance',
              delivery: 'byParagraph',
              delayMs: 0,
              durationMs: 500,
              native: 'pptx:fade',
              fallback: 'dissolve',
            },
          ],
          transition: { native: 'pptx:fade', fallback: 'dissolve', durationMs: 700, advance: null },
          fonts: [0, 1],
        },
        {
          id: 's3',
          layout: null,
          hidden: true,
          number: null,
          title: 'Backup numbers',
          notes: [],
          failed: null,
          timeline: [],
          transition: null,
          fonts: [0],
        },
        {
          id: 's4',
          layout: null,
          hidden: false,
          number: 3,
          title: 'Thank you',
          notes: [],
          failed: null,
          timeline: [],
          transition: null,
          fonts: [0],
        },
      ],
      media: [{ path: SAMPLE_IMAGE_PATH, mime: 'image/svg+xml', sha256: IMAGE_SHA, bytes: SAMPLE_SVG.length }],
      fonts: [
        { family: 'Liberation Sans', weight: 400, style: 'normal', source: 'bundled', licence: 'SIL OFL 1.1' },
        { family: 'Liberation Sans', weight: 700, style: 'normal', source: 'bundled', licence: 'SIL OFL 1.1' },
      ],
      diagnostics: {
        total: 2,
        error: 0,
        decline: 1,
        lossy: 0,
        warning: 1,
        info: 0,
        codes: [
          { code: 'PX3101', severity: 'decline', count: 1 },
          { code: 'MID5401', severity: 'warning', count: 1 },
        ],
      },
    },
    slides,
    media: [
      { path: SAMPLE_IMAGE_PATH, mime: 'image/svg+xml', blob: new Blob([SAMPLE_SVG], { type: 'image/svg+xml' }) },
    ],
    fonts: [],
  };
}

export const SAMPLE_DIAGNOSTICS = [
  {
    code: 'PX3101',
    severity: 'decline' as const,
    message: 'A SmartArt graphic shows as its fallback picture',
    feature: 'smartart',
    container: 's2',
    node: null,
  },
  {
    code: 'MID5401',
    severity: 'warning' as const,
    message: 'Calibri is not available; Carlito draws it',
    feature: 'fonts',
    container: null,
    node: null,
  },
];
