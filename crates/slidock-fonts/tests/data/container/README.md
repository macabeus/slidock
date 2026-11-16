# Container fixtures

Fonts for the `container` tests, all under the SIL Open Font License 1.1 (the `OFL-*.txt` files, one per family;
none declares a Reserved Font Name). They are modified versions of these files:

| Fixture | Source | SHA-256 of the source |
|---|---|---|
| `barlow` | google/fonts@60824dce48f7dd28fe7d65559f2da1f6e04e585b `ofl/barlowcondensed/BarlowCondensed-Regular.ttf` | `583cec5da3b84bc4dc7c9c72e2a565c94d34e431518b19d7e250b7830ad5f996` |
| `fira` | google/fonts@60824dce48f7dd28fe7d65559f2da1f6e04e585b `ofl/firasans/FiraSans-Regular.ttf` | `c29556a2719bf613ef3d5e070e40d903a8965d9c081beca1375dc1e6e0f93c23` |
| `oswald` | google/fonts@60824dce48f7dd28fe7d65559f2da1f6e04e585b `ofl/oswald/Oswald[wght].ttf` | `5b38c246e255a12f5712d640d56bcced0472466fc68983d2d0410ec0457c2817` |
| `pacifico` | google/fonts@60824dce48f7dd28fe7d65559f2da1f6e04e585b `ofl/pacifico/Pacifico-Regular.ttf` | `5b6c0d5334a7bf77dea52b975c5a0c408878c0f7115ed5b6fb151f634b7bf701` |
| `robotomono` | google/fonts@60824dce48f7dd28fe7d65559f2da1f6e04e585b `ofl/robotomono/RobotoMono[wght].ttf` | `66a80e79d17e4c7cabd162e2916578a4cc08fd19eef6e2a643305eae9c567b2b` |
| `roboto` | googlefonts/roboto-3-classic release v3.011, `Roboto_v3.011.zip` member `hinted/static/Roboto-Regular.ttf` | `b6ef3b40740949948f0e21ea1e7f2e8a5414e1ef3d95bf57dbd71926d8291cba` |

Each `<name>.ttf` is its source subset to about fifty characters (hinting, layout and variations kept). From those:

- `<name>.woff`: WOFF 1.0 (fontTools).
- `<name>.woff2`: WOFF2 with `glyf`/`loca` transformed (fontTools); `barlow-hmtx.woff2` also transforms `hmtx`,
  `barlow-null.woff2` transforms nothing.
- `barlow-cff.otf` (and its `.woff`, `.woff2`): Barlow's glyphs redrawn as CFF outlines.
- `pair.ttc`: a collection of `barlow` and `fira`; `pair.woff2` is it compressed by the reference encoder.
- `*.woff2.ft.ttf`: what fontTools 4.60.1 decodes each WOFF2 into; `*.woff2.g.ttf`: what the reference decoder
  (google/woff2@0f4d304faa1c62994536dc73510305c7357da8d4) decodes it into.
- `barlow-mkeot.eot`: `barlow.ttf` wrapped as EOT 2.2 by W3C eot-utils 1.1 `mkeot` (which writes the header's name
  strings big-endian).
- `roboto-hdmx.ttf`: `roboto.ttf` with an `hdmx` table at 11, 16 and 23 ppem whose widths stray up to two pixels
  from the scaled advance widths.
- `<name>-mtx.eot` for `barlow`, `oswald`, `robotomono` and `roboto-hdmx`: MicroType Express compressed EOTs made by
  sfntly's `SfntTool -e -x` (googlefonts/sfntly@a56f5782f209771aa226063757d57e6b5c948478); `<name>-mtx.eot.libeot.ttf`
  is what libeot's `eot2ttf` (umanwizard/libeot@0407abddc581d32e9871ee41535183ee1d924d85) decodes each into. libeot
  drops `hdmx`.

`make_fixtures.sh` rebuilds them all (fontTools in a venv, the reference tools in Docker).
