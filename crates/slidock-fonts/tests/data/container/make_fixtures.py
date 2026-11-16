"""Builds the container test fixtures from OFL fonts with fontTools 4.60.1 (and brotli 1.1.0).

Usage: python -I make_fixtures.py <dir with the downloaded fonts> <output dir>

The sources and their licences are listed in README.md. Each font is
subset to a few dozen characters, keeping its hinting, layout and variations. For each subset font `<name>.ttf`
this writes `<name>.woff` (WOFF 1.0), `<name>.woff2` (glyf/loca transformed), and `<name>.woff2.ft.ttf`, what
fontTools decodes the WOFF2 into: the oracle the Rust decoder is compared with. It also writes `pair.ttc`, a
collection of two of them, which `make_fixtures.sh` compresses with the reference WOFF2 encoder, and
`roboto-hdmx.ttf`, `roboto.ttf` with an `hdmx` table, which `make_fixtures.sh` compresses with MicroType Express.
"""

import io
import sys
from pathlib import Path

from fontTools import subset
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.t2CharStringPen import T2CharStringPen
from fontTools.ttLib import TTCollection, TTFont, newTable, woff2

TEXT = "AaBbCcGgOoQqRrSs0123456789 .,;:!?&@#()[]{}ÀÉçñüß€–—“”ﬁ"

SOURCES = {
    "barlow": "BarlowCondensed-Regular.ttf",
    "fira": "FiraSans-Regular.ttf",
    "oswald": "Oswald[wght].ttf",
    "pacifico": "Pacifico-Regular.ttf",
    "robotomono": "RobotoMono[wght].ttf",
    "roboto": "Roboto-Regular.ttf",
}


def subset_font(path: Path) -> TTFont:
    options = subset.Options()
    options.layout_features = ["*"]
    options.name_IDs = ["*"]
    options.name_languages = ["*"]
    options.hinting = True
    options.notdef_outline = True
    options.recalc_bounds = False
    options.recalc_timestamp = False
    font = TTFont(path)
    subsetter = subset.Subsetter(options)
    subsetter.populate(text=TEXT)
    subsetter.subset(font)
    return font


def save(font: TTFont, path: Path, flavor=None, transform=None) -> None:
    buffer = io.BytesIO()
    font.flavor = None
    font.save(buffer, reorderTables=True)
    if flavor is None:
        path.write_bytes(buffer.getvalue())
        return
    buffer.seek(0)
    if flavor == "woff":
        clone = TTFont(buffer)
        clone.flavor = "woff"
        clone.save(path, reorderTables=True)
        return
    out = io.BytesIO()
    woff2.compress(buffer, out, transform_tables=transform)
    path.write_bytes(out.getvalue())


def oracle(woff2_path: Path) -> None:
    woff2.decompress(str(woff2_path), str(woff2_path.with_name(woff2_path.name + ".ft.ttf")))


def to_cff(source: TTFont) -> TTFont:
    """The same glyphs as a CFF-flavoured OpenType font."""
    glyph_set = source.getGlyphSet()
    order = source.getGlyphOrder()
    charstrings = {}
    for name in order:
        pen = T2CharStringPen(source["hmtx"][name][0], glyph_set)
        glyph_set[name].draw(pen)
        charstrings[name] = pen.getCharString()
    builder = FontBuilder(source["head"].unitsPerEm, isTTF=False)
    builder.setupGlyphOrder(order)
    builder.setupCharacterMap(source.getBestCmap())
    names = source["name"]
    family = names.getDebugName(1) + " CFF"
    builder.setupCFF(family.replace(" ", ""), {"FullName": family}, charstrings, {})
    builder.setupHorizontalMetrics({name: source["hmtx"][name] for name in order})
    hhea = source["hhea"]
    builder.setupHorizontalHeader(ascent=hhea.ascent, descent=hhea.descent)
    builder.setupNameTable({"familyName": family, "styleName": "Regular"})
    builder.setupOS2(usWeightClass=source["OS/2"].usWeightClass)
    builder.setupPost()
    return builder.font


def with_hdmx(font: TTFont) -> TTFont:
    """An `hdmx` table at three sizes whose widths stray up to two pixels from the scaled advance widths."""
    hdmx = newTable("hdmx")
    hdmx.version = 0
    hdmx.hdmx = {}
    units_per_em = font["head"].unitsPerEm
    for ppem in (11, 16, 23):
        widths = {}
        for index, name in enumerate(font.getGlyphOrder()):
            advance = font["hmtx"][name][0]
            scaled = ((64 * ppem * advance + units_per_em // 2) // units_per_em + 32) // 64
            widths[name] = max(0, scaled + (index * ppem) % 5 - 2)
        hdmx.hdmx[ppem] = widths
    font["hdmx"] = hdmx
    return font


def main(source_dir: Path, out_dir: Path) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    fonts = {}
    for name, file in SOURCES.items():
        font = subset_font(source_dir / file)
        fonts[name] = font
        ttf = out_dir / f"{name}.ttf"
        save(font, ttf)
        font = TTFont(ttf)
        save(font, out_dir / f"{name}.woff", "woff")
        save(font, out_dir / f"{name}.woff2", "woff2")
        oracle(out_dir / f"{name}.woff2")

    barlow = TTFont(out_dir / "barlow.ttf")
    save(barlow, out_dir / "barlow-hmtx.woff2", "woff2", {"glyf", "loca", "hmtx"})
    oracle(out_dir / "barlow-hmtx.woff2")
    save(barlow, out_dir / "barlow-null.woff2", "woff2", set())
    oracle(out_dir / "barlow-null.woff2")

    cff = to_cff(barlow)
    save(cff, out_dir / "barlow-cff.otf")
    cff = TTFont(out_dir / "barlow-cff.otf")
    save(cff, out_dir / "barlow-cff.woff", "woff")
    save(cff, out_dir / "barlow-cff.woff2", "woff2")
    oracle(out_dir / "barlow-cff.woff2")

    save(with_hdmx(TTFont(out_dir / "roboto.ttf")), out_dir / "roboto-hdmx.ttf")

    collection = TTCollection()
    collection.fonts = [TTFont(out_dir / "barlow.ttf"), TTFont(out_dir / "fira.ttf")]
    collection.save(out_dir / "pair.ttc", shareTables=True)


if __name__ == "__main__":
    main(Path(sys.argv[1]), Path(sys.argv[2]))
