// The bundled substitutes, the floor under every other source: Liberation 2.1.5, Carlito, Caladea and DejaVu Sans as
// static WOFF2 next to the app (`fonts/core/`, the files of `slidock-substitutes` that vite.config.ts serves and
// copies, written by `scripts/substitutes.mjs`). Only the faces the deck draws with no file yet (the deck message's
// `substitutes`) are fetched, each checked against the SHA-256 its manifest records.
import type { FaceStyle, SubstituteFace } from '@slidock/types';

import type { Fetch } from './google';

/** The manifest's path under the app's base URL. */
export const CORE_MANIFEST = 'fonts/core/core.json';

export interface CoreFace {
  family: string;
  weight: number;
  style: FaceStyle;
  /** Next to the manifest. */
  file: string;
  /** Of the WOFF2 file. */
  sha256: string;
  /** SPDX id. */
  licence: string;
}

export interface CoreManifest {
  faces: CoreFace[];
}

function isCoreFace(value: unknown): value is CoreFace {
  if (typeof value !== 'object' || value === null) {
    return false;
  }
  const face = value as Record<string, unknown>;
  return (
    typeof face.family === 'string' &&
    typeof face.weight === 'number' &&
    (face.style === 'normal' || face.style === 'italic') &&
    typeof face.file === 'string' &&
    /^[A-Za-z0-9._-]+\.woff2$/.test(face.file) &&
    typeof face.sha256 === 'string' &&
    /^[0-9a-f]{64}$/.test(face.sha256) &&
    typeof face.licence === 'string'
  );
}

export function parseManifest(json: unknown): CoreManifest {
  const faces = typeof json === 'object' && json !== null ? (json as { faces?: unknown }).faces : undefined;
  if (!Array.isArray(faces) || !faces.every(isCoreFace)) {
    throw new Error('the substitutes manifest cannot be read');
  }
  return { faces };
}

/** The manifest's file for a substitute face the worker names; `undefined` when the manifest has none. */
export function substituteFile(manifest: CoreManifest, face: SubstituteFace): CoreFace | undefined {
  const family = face.family.toLowerCase();
  return manifest.faces.find(
    (entry) => entry.family.toLowerCase() === family && entry.weight === face.weight && entry.style === face.style,
  );
}

export async function sha256Hex(bytes: ArrayBuffer): Promise<string> {
  const digest = await crypto.subtle.digest('SHA-256', bytes);
  return Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, '0')).join('');
}

/** Fetches the manifest from the app's own origin. */
export async function loadManifest(fetch: Fetch, base: string): Promise<CoreManifest> {
  const response = await fetch(`${base}${CORE_MANIFEST}`, { credentials: 'same-origin' });
  if (!response.ok) {
    throw new Error(`the substitutes manifest answered HTTP ${response.status}`);
  }
  return parseManifest(await response.json());
}

/** Fetches one substitute face and checks its bytes against the manifest. */
export async function loadFace(fetch: Fetch, base: string, face: CoreFace): Promise<ArrayBuffer> {
  const directory = CORE_MANIFEST.slice(0, CORE_MANIFEST.lastIndexOf('/') + 1);
  const response = await fetch(`${base}${directory}${face.file}`, { credentials: 'same-origin' });
  if (!response.ok) {
    throw new Error(`${face.file} answered HTTP ${response.status}`);
  }
  const bytes = await response.arrayBuffer();
  if ((await sha256Hex(bytes)) !== face.sha256) {
    throw new Error(`${face.file} is not the file the manifest records`);
  }
  return bytes;
}
