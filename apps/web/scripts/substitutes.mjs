#!/usr/bin/env node
// Writes the bundled substitutes to crates/slidock-substitutes/fonts/, which the CLI builds in and the app serves at
// fonts/core/: WOFF2 builds of Liberation 2.1.5, DejaVu Sans 2.37, Carlito and Caladea, their licences and core.json,
// the manifest both read. Every download is pinned by URL and SHA-256, and so is every font file taken from it.
//
//   SLIDOCK_FONTTOOLS_PYTHON=<python with fonttools==4.60.1 and brotli==1.1.0> node scripts/substitutes.mjs
//   node scripts/substitutes.mjs --check     # the committed files match core.json (no network, no Python)
//
// Downloads are kept in SLIDOCK_SUBSTITUTES_CACHE (default: <repo>/target/substitutes). The WOFF2 encoder is
// fontTools' (`fontTools.ttLib.woff2 compress`), which writes the same bytes for the same input and versions.
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const APP = fileURLToPath(new URL('..', import.meta.url));
const OUT = join(APP, '..', '..', 'crates', 'slidock-substitutes', 'fonts');
const CACHE = process.env.SLIDOCK_SUBSTITUTES_CACHE ?? join(APP, '..', '..', 'target', 'substitutes');

const CARLITO = 'https://raw.githubusercontent.com/googlefonts/carlito/3a810cab78ebd6e2e4eed42af9e8453c4f9b850a';
const GOOGLE_FONTS = 'https://raw.githubusercontent.com/google/fonts/6c1dbe27dcf7d817d4b62cec033ba77fa8e2a5ec';

/** Each source: where it comes from, its licence, and its faces with the SHA-256 of each TTF. */
const SOURCES = [
  {
    name: 'Liberation 2.1.5',
    archive: {
      url: 'https://github.com/liberationfonts/liberation-fonts/files/7261482/liberation-fonts-ttf-2.1.5.tar.gz',
      sha256: '7191c669bf38899f73a2094ed00f7b800553364f90e2637010a69c0e268f25d0',
      root: 'liberation-fonts-ttf-2.1.5',
    },
    licence: { spdx: 'OFL-1.1', path: 'LICENSE', file: 'Liberation-LICENSE.txt' },
    faces: [
      [
        'Liberation Sans',
        400,
        'normal',
        'LiberationSans-Regular',
        '76d04c18ea243f426b7de1f3ad208e927008f961dc5945e5aad352d0dfde8ee8',
      ],
      [
        'Liberation Sans',
        700,
        'normal',
        'LiberationSans-Bold',
        '788abee4c806d660e8aee46689dd8540cd4bb98da03dcc9d171ce3efd99a9173',
      ],
      [
        'Liberation Sans',
        400,
        'italic',
        'LiberationSans-Italic',
        'e5bae5c4cde31f22142753855f4f8fb86da6ff39955ed3c0a11248b0d16948b0',
      ],
      [
        'Liberation Sans',
        700,
        'italic',
        'LiberationSans-BoldItalic',
        '698da70fc191cc5f33ad4d6d3fe830fe4624b898ea2e3169955928b7c491f1ee',
      ],
      [
        'Liberation Serif',
        400,
        'normal',
        'LiberationSerif-Regular',
        '058ea80864aef09a23f45cbec2bb5400bc3dfbdea01c3f10538a21fcb497fb74',
      ],
      [
        'Liberation Serif',
        700,
        'normal',
        'LiberationSerif-Bold',
        'd754ba427cfe0bca54ae052384baa8f842da5bd6550ad4da024ac441e7a7d5ce',
      ],
      [
        'Liberation Serif',
        400,
        'italic',
        'LiberationSerif-Italic',
        '0e3dea9f8d613e006ccfa62201f33e265d19167bd0907725c3e145368b04fc2e',
      ],
      [
        'Liberation Serif',
        700,
        'italic',
        'LiberationSerif-BoldItalic',
        'f17db8af71e24d2066b587546021d4f0b296be389512b658dec3c09affeb11a7',
      ],
      [
        'Liberation Mono',
        400,
        'normal',
        'LiberationMono-Regular',
        'f2b83c763e8afd21709333370bed4774337fae82267937e2b5aea7e2fbd922c1',
      ],
      [
        'Liberation Mono',
        700,
        'normal',
        'LiberationMono-Bold',
        'bd62a0672d0b9b6710b01df434c80ad54fa5f0835207eb7b17b7a761463067bb',
      ],
      [
        'Liberation Mono',
        400,
        'italic',
        'LiberationMono-Italic',
        '605c01c711b44480a7508d349dfbf3264e81fa43d69e61cfa7d10b86e764c4d1',
      ],
      [
        'Liberation Mono',
        700,
        'italic',
        'LiberationMono-BoldItalic',
        '79451f3c09fe25116098853b7a2ca6e2436220ccc11af022979adbcf195be130',
      ],
    ],
  },
  {
    name: 'DejaVu 2.37',
    archive: {
      url: 'https://github.com/dejavu-fonts/dejavu-fonts/releases/download/version_2_37/dejavu-fonts-ttf-2.37.tar.bz2',
      sha256: 'fa9ca4d13871dd122f61258a80d01751d603b4d3ee14095d65453b4e846e17d7',
      root: 'dejavu-fonts-ttf-2.37',
      fonts: 'ttf',
    },
    licence: { spdx: 'Bitstream-Vera', path: 'LICENSE', file: 'DejaVu-LICENSE.txt' },
    faces: [
      ['DejaVu Sans', 400, 'normal', 'DejaVuSans', '7da195a74c55bef988d0d48f9508bd5d849425c1770dba5d7bfc6ce9ed848954'],
      [
        'DejaVu Sans',
        700,
        'normal',
        'DejaVuSans-Bold',
        'e6476c1b80502924294eed40894c5b18e06c181444ca953e5334262df9c27724',
      ],
      [
        'DejaVu Sans',
        400,
        'italic',
        'DejaVuSans-Oblique',
        '4af75fa16ee6d3ad43e1ecec41862c24954af26a55c6bb1ebb27bd486a50f5f4',
      ],
      [
        'DejaVu Sans',
        700,
        'italic',
        'DejaVuSans-BoldOblique',
        'eb436dca0c2594b73d8b603b892e374fdfd8d885d25ffb4f18df4c4c0b49e50f',
      ],
    ],
  },
  {
    name: 'Carlito (googlefonts/carlito@3a810cab)',
    files: `${CARLITO}/fonts/ttf`,
    licence: {
      spdx: 'OFL-1.1',
      url: `${CARLITO}/OFL.txt`,
      sha256: '58402f82a7c332a700294988fe7554fbb0a63a8d27ccc1ee3bbc640311990a00',
      file: 'Carlito-OFL.txt',
    },
    faces: [
      ['Carlito', 400, 'normal', 'Carlito-Regular', 'f6418f708baede9789daef5d458c0f53d2a888af9820e8062934e504fedc6595'],
      ['Carlito', 700, 'normal', 'Carlito-Bold', 'bb5d20f79b82599ec72983597437373a80f2d2085fa91fc144fd74e876a594db'],
      ['Carlito', 400, 'italic', 'Carlito-Italic', '0b019225e58d702bfedcbd35c21696769f8ee115cb6343f84c2f240312450d1c'],
      [
        'Carlito',
        700,
        'italic',
        'Carlito-BoldItalic',
        'b32928186c119599e03ca6a1ffc680fdcb7fac95772f4b95d989cf6cd3861517',
      ],
    ],
  },
  {
    name: 'Caladea (google/fonts@6c1dbe27)',
    files: `${GOOGLE_FONTS}/ofl/caladea`,
    licence: {
      spdx: 'OFL-1.1',
      url: `${GOOGLE_FONTS}/ofl/caladea/OFL.txt`,
      sha256: 'ccdab61d371d8c8683a128a92cd7d498dbdb1d37689f7cb21f1bf6b16658d213',
      file: 'Caladea-OFL.txt',
    },
    faces: [
      ['Caladea', 400, 'normal', 'Caladea-Regular', 'f1e899278b7b4491aba5b6a8253c4b04c050cc59b21865be5c37559a775153cd'],
      ['Caladea', 700, 'normal', 'Caladea-Bold', 'ae3cb2dcbc925809dd29d2a44e9802211cab66be541bacbfc9c08c74b27c3742'],
      ['Caladea', 400, 'italic', 'Caladea-Italic', '4359a8e24f748b6447b1ff6d7a174febe70961d29f8bb8634b56dacd740a3deb'],
      [
        'Caladea',
        700,
        'italic',
        'Caladea-BoldItalic',
        'ccabaa7b7e2fdf253d2b1a5fa699dd8a3df8d835a9eb285ad82631a677eb76c0',
      ],
    ],
  },
];

const sha256 = (bytes) => createHash('sha256').update(bytes).digest('hex');

function checked(bytes, expected, what) {
  const actual = sha256(bytes);
  if (actual !== expected) {
    throw new Error(`${what}: SHA-256 ${actual}, expected ${expected}`);
  }
  return bytes;
}

/** The bytes at `url`, from the cache when present, checked against `expected`. */
async function download(url, expected) {
  mkdirSync(CACHE, { recursive: true });
  const cached = join(CACHE, expected);
  if (existsSync(cached)) {
    return checked(readFileSync(cached), expected, url);
  }
  const response = await fetch(url, { redirect: 'follow' });
  if (!response.ok) {
    throw new Error(`${url}: HTTP ${response.status}`);
  }
  const bytes = checked(Buffer.from(await response.arrayBuffer()), expected, url);
  writeFileSync(cached, bytes);
  return bytes;
}

/** Reads `paths` out of a tar archive (gzip or bzip2) with the system's tar. */
function untar(archive, paths) {
  const dir = mkdtempSync(join(CACHE, 'tar-'));
  try {
    const file = join(dir, 'archive');
    writeFileSync(file, archive);
    execFileSync('tar', ['-xf', file, '-C', dir, ...paths]);
    return Object.fromEntries(paths.map((path) => [path, readFileSync(join(dir, path))]));
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

function woff2(ttf, name) {
  const python = process.env.SLIDOCK_FONTTOOLS_PYTHON;
  if (!python) {
    throw new Error('set SLIDOCK_FONTTOOLS_PYTHON to a Python with fonttools==4.60.1 and brotli==1.1.0');
  }
  const dir = mkdtempSync(join(CACHE, 'woff2-'));
  try {
    const input = join(dir, `${name}.ttf`);
    const output = join(dir, `${name}.woff2`);
    writeFileSync(input, ttf);
    execFileSync(python, ['-I', '-m', 'fontTools.ttLib.woff2', 'compress', '-o', output, input], { stdio: 'pipe' });
    return readFileSync(output);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

async function build() {
  mkdirSync(OUT, { recursive: true });
  for (const entry of readdirSync(OUT)) {
    rmSync(join(OUT, entry));
  }
  const faces = [];
  for (const source of SOURCES) {
    const ttfs = {};
    let licence;
    if (source.archive) {
      const { url, sha256: expected, root, fonts = '' } = source.archive;
      const archive = await download(url, expected);
      const fontPath = (name) => [root, fonts, `${name}.ttf`].filter(Boolean).join('/');
      const files = untar(archive, [
        ...source.faces.map(([, , , name]) => fontPath(name)),
        `${root}/${source.licence.path}`,
      ]);
      for (const [, , , name] of source.faces) {
        ttfs[name] = files[fontPath(name)];
      }
      licence = files[`${root}/${source.licence.path}`];
    } else {
      for (const [, , , name, expected] of source.faces) {
        ttfs[name] = await download(`${source.files}/${name}.ttf`, expected);
      }
      licence = await download(source.licence.url, source.licence.sha256);
    }
    writeFileSync(join(OUT, source.licence.file), licence);
    for (const [family, weight, style, name, expected] of source.faces) {
      const bytes = woff2(checked(ttfs[name], expected, `${name}.ttf`), name);
      const file = `${name}.woff2`;
      writeFileSync(join(OUT, file), bytes);
      faces.push({
        family,
        weight,
        style,
        file,
        sha256: sha256(bytes),
        licence: source.licence.spdx,
        licenceFile: source.licence.file,
        source: source.name,
        ttfSha256: expected,
      });
    }
  }
  writeFileSync(join(OUT, 'core.json'), `${JSON.stringify({ faces }, null, 2)}\n`);
  const total = faces.reduce((sum, face) => sum + readFileSync(join(OUT, face.file)).length, 0);
  console.log(`wrote ${faces.length} faces, ${(total / 1048576).toFixed(2)} MiB, to ${OUT}`);
}

function check() {
  const manifest = JSON.parse(readFileSync(join(OUT, 'core.json'), 'utf8'));
  const problems = [];
  const expected = SOURCES.flatMap((source) => source.faces.map((face) => `${face[3]}.woff2`));
  const listed = manifest.faces.map((face) => face.file);
  if (JSON.stringify(listed) !== JSON.stringify(expected)) {
    problems.push(`core.json lists ${listed.join(', ')}; expected ${expected.join(', ')}`);
  }
  for (const face of manifest.faces) {
    const path = join(OUT, face.file);
    if (!existsSync(path)) {
      problems.push(`${face.file} is missing`);
    } else if (sha256(readFileSync(path)) !== face.sha256) {
      problems.push(`${face.file} does not match core.json`);
    }
    if (!existsSync(join(OUT, face.licenceFile))) {
      problems.push(`${face.licenceFile} is missing`);
    }
  }
  if (problems.length > 0) {
    throw new Error(problems.join('\n'));
  }
  console.log(`${manifest.faces.length} faces match core.json`);
}

if (process.argv.includes('--check')) {
  check();
} else {
  await build();
}
