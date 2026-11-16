// The deck's shipped faces in the app's own document, so the slides (each in a shadow root of the stage) draw with
// exactly the files the download ships: document-level faces reach shadow trees in every engine.
import type { FontFile } from '@slidock/types';

export type MakeFontFace = (family: string, source: ArrayBuffer, descriptors: FontFaceDescriptors) => FontFace;

const browserFontFace: MakeFontFace = (family, source, descriptors) => new FontFace(family, source, descriptors);

/** The faces one deck added to a `FontFaceSet`; `replace` swaps them for a new finish's. */
export class DocumentFonts {
  readonly #set: Pick<FontFaceSet, 'add' | 'delete'> | null;
  readonly #make: MakeFontFace;
  #faces: FontFace[] = [];

  constructor(
    set: Pick<FontFaceSet, 'add' | 'delete'> | null = typeof document === 'undefined' ? null : document.fonts,
    make: MakeFontFace = browserFontFace,
  ) {
    this.#set = typeof FontFace === 'undefined' && make === browserFontFace ? null : set;
    this.#make = make;
  }

  /** Puts `fonts` in the document in place of the faces added before. */
  replace(fonts: readonly FontFile[]) {
    const set = this.#set;
    if (!set) {
      return;
    }
    const next = fonts.map((font) => {
      const face = this.#make(font.family, font.bytes, {
        weight: font.weight,
        style: font.style,
        display: 'block',
        ...(font.unicodeRange === null ? {} : { unicodeRange: font.unicodeRange }),
      });
      // A file the browser refuses fails its own load; the slides then draw the next family of their stack.
      face.loaded.catch(() => undefined);
      return face;
    });
    for (const face of this.#faces) {
      set.delete(face);
    }
    for (const face of next) {
      set.add(face);
    }
    this.#faces = next;
  }

  clear() {
    this.replace([]);
  }
}
