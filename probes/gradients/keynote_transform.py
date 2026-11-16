"""K2: Keynote transform gradients patched into a copy of K1 (probes/gradients/README.md).

K1 is Keynote 14.5's import of probes/out/gradient-probe.pptx, saved as gradient-probe-keynote.key. Keynote's import
writes angle gradients only, so this script rewrites the gradients of chosen styles in a copy: a transform (start, end
and base size) in place of the angle, inflections, opacity, P3 stops, the advanced flag, and a gradient with neither
an angle nor a transform. It walks the protobuf wire format without a schema, so every field it does not touch keeps
its bytes, re-frames each IWA file it changed with literal-only Snappy, and fixes the MessageInfo lengths. Files it does
not change are copied byte for byte.

Styles are shared between slides, so a patch moves every slide drawn with that style; `--cases` writes, per patched
style, the case and the drawables that use it (from the slide files), for the bench rows and the decoding afterwards.

    python3 -I probes/gradients/keynote_transform.py K1.key OUT.key [--cases OUT.json]
    python3 -I probes/gradients/keynote_transform.py --round-trip K1.key   # unpatched: same messages, exit 1 if not

Stdlib only. Never open the output in Keynote from a script: a person opens it by hand and exports its PDF.
"""

import json
import struct
import sys
import zipfile

# --- varints, Snappy and IWA framing -------------------------------------------------------------------------------


def varint(b, i):
    r = s = 0
    while True:
        c = b[i]
        i += 1
        r |= (c & 0x7F) << s
        s += 7
        if c < 0x80:
            return r, i


def put_varint(n):
    out = bytearray()
    while True:
        c = n & 0x7F
        n >>= 7
        if n:
            out.append(c | 0x80)
        else:
            out.append(c)
            return bytes(out)


def snappy_decode(src):
    n, i = varint(src, 0)
    out = bytearray()
    while i < len(src):
        t = src[i]
        i += 1
        kind = t & 3
        if kind == 0:
            ln = t >> 2
            if ln >= 60:
                nb = ln - 59
                ln = int.from_bytes(src[i:i + nb], "little")
                i += nb
            ln += 1
            out += src[i:i + ln]
            i += ln
            continue
        if kind == 1:
            ln = ((t >> 2) & 7) + 4
            off = ((t >> 5) << 8) | src[i]
            i += 1
        elif kind == 2:
            ln = (t >> 2) + 1
            off = int.from_bytes(src[i:i + 2], "little")
            i += 2
        else:
            ln = (t >> 2) + 1
            off = int.from_bytes(src[i:i + 4], "little")
            i += 4
        start = len(out) - off
        if off <= 0 or start < 0:
            raise ValueError("snappy: bad copy offset")
        for j in range(ln):
            out.append(out[start + j])
    if len(out) != n:
        raise ValueError("snappy: length mismatch")
    return bytes(out)


def snappy_literal(data):
    """A valid Snappy block holding `data` as literals only (no copies)."""
    out = bytearray(put_varint(len(data)))
    i = 0
    while i < len(data):
        piece = data[i:i + 65536]
        n = len(piece) - 1
        if n < 60:
            out.append(n << 2)
        elif n < 1 << 8:
            out += bytes([60 << 2, n])
        elif n < 1 << 16:
            out += bytes([61 << 2]) + n.to_bytes(2, "little")
        else:
            out += bytes([62 << 2]) + n.to_bytes(3, "little")
        out += piece
        i += len(piece)
    return bytes(out)


def unframe(b):
    """An IWA file's chunks, decompressed: [(header byte, uncompressed bytes)]."""
    chunks = []
    i = 0
    while i < len(b):
        ln = int.from_bytes(b[i + 1:i + 4], "little")
        chunks.append((b[i], snappy_decode(b[i + 4:i + 4 + ln])))
        i += 4 + ln
    return chunks


CHUNK = 65536


def frame(stream):
    """An IWA file of literal-only Snappy chunks of at most 64 KiB each."""
    out = bytearray()
    for i in range(0, len(stream), CHUNK):
        block = snappy_literal(stream[i:i + CHUNK])
        out += bytes([0]) + len(block).to_bytes(3, "little") + block
    return bytes(out)


# --- schema-less protobuf: fields in file order, each with its own bytes --------------------------------------------


def parse(b):
    """[(field, wire, value, raw)]: value is an int (varint) or bytes; raw is the entry's bytes (tag included)."""
    out = []
    i = 0
    while i < len(b):
        start = i
        tag, i = varint(b, i)
        field, wire = tag >> 3, tag & 7
        if wire == 0:
            value, i = varint(b, i)
        elif wire == 1:
            value = b[i:i + 8]
            i += 8
        elif wire == 2:
            ln, i = varint(b, i)
            value = b[i:i + ln]
            i += ln
        elif wire == 5:
            value = b[i:i + 4]
            i += 4
        else:
            raise ValueError(f"wire type {wire}")
        if i > len(b):
            raise ValueError("truncated message")
        out.append((field, wire, value, b[start:i]))
    return out


def entry(field, wire, value):
    tag = put_varint(field << 3 | wire)
    if wire == 0:
        return (field, wire, value, tag + put_varint(value))
    if wire == 2:
        return (field, wire, value, tag + put_varint(len(value)) + value)
    return (field, wire, value, tag + value)


def join(entries):
    return b"".join(e[3] for e in entries)


def last(entries, field):
    found = [e for e in entries if e[0] == field]
    return found[-1] if found else None


def f32(x):
    return struct.pack("<f", x)


def edit(message, path, change):
    """`message` with the sub-message at `path` (field numbers; the last occurrence of each) replaced by
    `change(entries) -> entries`."""
    entries = parse(message)
    if not path:
        return join(change(entries))
    field = path[0]
    at = max((k for k, e in enumerate(entries) if e[0] == field and e[1] == 2), default=None)
    if at is None:
        raise KeyError(f"field {field} absent")
    entries[at] = entry(field, 2, edit(entries[at][2], path[1:], change))
    return join(entries)


def put(entries, field, wire, value):
    """Sets a singular field: the last occurrence replaced, or appended."""
    out = [e for e in entries if e[0] != field]
    return out + [entry(field, wire, value)]


# --- the gradient patches -------------------------------------------------------------------------------------------

# Style type -> path to its FillArchive's parent; the fill is field 1 there, its gradient field 2 of the fill.
FILL_PARENT = {
    3015: (11,),  # TSD.ShapeStyleArchive.shape_properties
    2025: (1, 11),  # TSWP.ShapeStyleArchive.super.shape_properties
    9: (11,),  # KN.SlideStyleArchive.slide_properties
    6004: (11,),  # TST.CellStyleArchive.cell_properties
}


def point(x, y):
    return join([entry(1, 5, f32(x)), entry(2, 5, f32(y))])


def transform(start, end, base):
    return join([
        entry(1, 2, point(*start)),
        entry(2, 2, point(*end)),
        entry(3, 2, point(*base)),
    ])


def gradient_patch(case):
    """A change of GradientArchive entries for a case: `transform` [start, end, base] replaces the angle (field 5);
    `drop_angle`; `advanced`; `opacity`; `inflections` (per stop); `p3_stop` (index); `alphas` (per stop); `type`."""

    def change(entries):
        if "type" in case:
            entries = put(entries, 1, 0, case["type"])
        if "transform" in case or case.get("drop_angle"):
            entries = [e for e in entries if e[0] != 5]
        if "transform" in case:
            entries = [e for e in entries if e[0] != 6] + [entry(6, 2, transform(*case["transform"]))]
        if "advanced" in case:
            entries = put(entries, 4, 0, int(case["advanced"]))
        if "opacity" in case:
            entries = put(entries, 3, 5, f32(case["opacity"]))
        stops = [k for k, e in enumerate(entries) if e[0] == 2]
        for n, k in enumerate(stops):
            stop = entries[k][2]
            infl = case.get("inflections")
            if infl is not None and n < len(infl):
                stop = join(put(parse(stop), 3, 5, f32(infl[n])))
            alphas = case.get("alphas")
            if alphas is not None and n < len(alphas):
                stop = edit(stop, (1,), lambda c, a=alphas[n]: put(c, 6, 5, f32(a)))
            if case.get("p3_stop") == n:
                stop = edit(stop, (1,), lambda c: put(put(put(put(c, 3, 5, f32(0.0)), 4, 5, f32(0.25)), 5, 5, f32(1.0)), 12, 0, 2))
            entries[k] = entry(2, 2, stop)
        return entries

    return change


# The K2 cases, by the K1 style they patch (decoded from K1: the style each drawable uses). A shared
# style moves every slide that draws with it; `slides` names the K1 cases each one reaches.
CASES = [
    {"id": "T01", "style": 548, "slides": "G01 G12(T07: turned 30°) G13 G17 G35 G39 G42(bar 0°) G43(2)",
     "what": "horizontal, base = natural, Keynote's +1 pt ends (retro-like)",
     "transform": [[-1, 100], [601, 100], [600, 200]]},
    {"id": "T02", "style": 620, "slides": "G02 G03 G14(T08: flipH) G15(flipV) G16 G43(2)",
     "what": "oblique (0, 0) → (100, 100) in a 100 × 100 base: iso-lines skewed by the frame",
     "transform": [[0, 0], [100, 100], [100, 100]]},
    {"id": "T03", "style": 759, "slides": "G04 (150 × 450)", "what": "start and end outside the box",
     "transform": [[-50, -100], [200, 550], [150, 450]]},
    {"id": "T04", "style": 829, "slides": "G05 (300 × 300) G42(bar 90°) G43(2)", "what": "start = end",
     "transform": [[150, 150], [150, 150], [300, 300]]},
    {"id": "T05", "style": 1735, "slides": "G18 G19 G23 G26 G27 G28 G31 G32 G33 G34 G43(16) (radials at 315°)",
     "what": "radial, base 100 × 100, start (50, 50), radius 50: the box's ellipse", "type": 1,
     "transform": [[50, 50], [100, 50], [100, 100]]},
    {"id": "T06", "style": 1874, "slides": "G20 G25 G30 (radials at 135°)", "what": "radial off-centre",
     "type": 1, "transform": [[25, 25], [75, 25], [100, 100]]},
    {"id": "T09", "style": 899, "slides": "G06 G43(2)", "what": "horizontal, inflection 0.2",
     "transform": [[0, 100], [600, 100], [600, 200]], "inflections": [0.2]},
    {"id": "T10", "style": 969, "slides": "G07 (3 stops)", "what": "horizontal, inflections 0.8 and 0.3",
     "transform": [[0, 100], [600, 100], [600, 200]], "inflections": [0.8, 0.3]},
    {"id": "T11", "style": 1039, "slides": "G08", "what": "horizontal, opacity 0.5",
     "transform": [[0, 100], [600, 100], [600, 200]], "opacity": 0.5},
    {"id": "T12", "style": 1109, "slides": "G09", "what": "horizontal, last stop P3 (0, 0.25, 1)",
     "transform": [[0, 100], [600, 100], [600, 200]], "p3_stop": 1},
    {"id": "T13", "style": 1179, "slides": "G10 (10 stops)", "what": "horizontal, alpha stops (Keynote rasters)",
     "transform": [[0, 100], [600, 100], [600, 200]], "alphas": [1, 0.2, 1, 0.2, 1, 0.2, 1, 0.2, 1, 0.2]},
    {"id": "T14", "style": 3018, "slides": "G36 (background)", "what": "background, oblique in a 960 × 540 base",
     "transform": [[0, 0], [960, 540], [960, 540]]},
    {"id": "T15", "style": 3156, "slides": "G38 (cell lin 90)", "what": "cell, transform left → right",
     "transform": [[0, 50], [300, 50], [300, 100]]},
    {"id": "T15b", "style": 3158, "slides": "G38 (cell radial)", "what": "cell radial at 135° (does it move?)",
     "angle": 2.356194490192345},
    {"id": "T16", "style": 2014, "slides": "G22 G24 (radials at 45°)", "what": "advancedGradient = 1 with the angle",
     "advanced": 1},
    {"id": "T17", "style": 1944, "slides": "G21", "what": "neither an angle nor a transform", "drop_angle": True},
]


def case_change(case):
    if "angle" in case:
        return lambda entries: [e for e in entries if e[0] != 5] + [entry(5, 2, join([entry(2, 5, f32(case["angle"]))]))]
    return gradient_patch(case)


# --- IWA objects ----------------------------------------------------------------------------------------------------


def objects(stream):
    """[(archive info bytes, [(message info bytes, payload)])] in stream order."""
    out = []
    i = 0
    while i < len(stream):
        ln, i = varint(stream, i)
        info = stream[i:i + ln]
        i += ln
        messages = []
        for field, _, value, _ in parse(info):
            if field != 2:
                continue
            length = (last(parse(value), 3) or (3, 0, 0, b""))[2]
            messages.append((value, stream[i:i + length]))
            i += length
        out.append((info, messages))
    if i != len(stream):
        raise ValueError("object stream overruns")
    return out


def rebuild(objs):
    out = bytearray()
    for info, messages in objs:
        entries = parse(info)
        infos = iter(messages)
        fixed = []
        for e in entries:
            if e[0] == 2:
                mi, payload = next(infos)
                mi_entries = parse(mi)
                if (last(mi_entries, 3) or (3, 0, 0, b""))[2] != len(payload):
                    mi = join(put(mi_entries, 3, 0, len(payload)))
                fixed.append(entry(2, 2, mi))
            else:
                fixed.append(e)
        info = join(fixed)
        out += put_varint(len(info)) + info
        for _, payload in messages:
            out += payload
    return bytes(out)


def object_id(info):
    e = last(parse(info), 1)
    return e[2] if e else None


def patch_stream(stream, by_style, done):
    objs = objects(stream)
    changed = False
    for k, (info, messages) in enumerate(objs):
        oid = object_id(info)
        case = by_style.get(oid)
        if case is None or not messages:
            continue
        mi, payload = messages[0]
        kind = (last(parse(mi), 1) or (1, 0, None, b""))[2]
        if kind not in FILL_PARENT:
            raise ValueError(f"{case['id']}: object {oid} is of type {kind}, not a style with a fill")
        path = FILL_PARENT[kind] + (1, 2)
        payload = edit(payload, path, case_change(case))
        objs[k] = (info, [(mi, payload)] + messages[1:])
        done.add(oid)
        changed = True
    return (rebuild(objs) if changed else None), changed


def stream_of(data):
    return b"".join(chunk for _, chunk in unframe(data))


def is_iwa(name):
    return name.startswith("Index/") and name.endswith(".iwa")


def copy_with(src, dst, replace):
    with zipfile.ZipFile(src) as zin, zipfile.ZipFile(dst, "w") as zout:
        for item in zin.infolist():
            data = replace.get(item.filename)
            zout.writestr(item, zin.read(item.filename) if data is None else data)


def patch(src, dst, cases_out=None):
    by_style = {case["style"]: case for case in CASES}
    done = set()
    replace = {}
    with zipfile.ZipFile(src) as z:
        for name in z.namelist():
            if not is_iwa(name):
                continue
            stream, changed = patch_stream(stream_of(z.read(name)), by_style, done)
            if changed:
                replace[name] = frame(stream)
    missing = sorted(set(by_style) - done)
    if missing:
        raise SystemExit(f"styles not found in {src}: {missing}")
    copy_with(src, dst, replace)
    if cases_out:
        with open(cases_out, "w") as f:
            json.dump({"source": src, "files": sorted(replace), "cases": CASES}, f, indent=2, sort_keys=True)
            f.write("\n")
    return replace


def round_trip(src):
    """Re-frames every IWA file without a change: each must decompress to the same objects, messages and bytes."""
    bad = []
    with zipfile.ZipFile(src) as z:
        for name in z.namelist():
            if not is_iwa(name):
                continue
            stream = stream_of(z.read(name))
            again = rebuild(objects(stream))
            if again != stream or stream_of(frame(again)) != stream:
                bad.append(name)
    return bad


def main(argv):
    if len(argv) == 2 and argv[0] == "--round-trip":
        bad = round_trip(argv[1])
        print("round trip: " + ("identical" if not bad else "differs in " + ", ".join(bad)))
        return 1 if bad else 0
    if len(argv) in (2, 4) and (len(argv) == 2 or argv[2] == "--cases"):
        replaced = patch(argv[0], argv[1], argv[3] if len(argv) == 4 else None)
        print(f"wrote {argv[1]}: {len(CASES)} cases in {', '.join(sorted(replaced))}")
        return 0
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
