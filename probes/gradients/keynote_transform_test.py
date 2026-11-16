"""Tests of keynote_transform.py on a synthetic package (no Keynote file needed).

    python3 -I probes/gradients/keynote_transform_test.py
"""

import io
import os
import sys
import unittest
import zipfile

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import keynote_transform as kt  # noqa: E402


def color(r, g, b):
    return kt.join([kt.entry(1, 0, 1), kt.entry(3, 5, kt.f32(r)), kt.entry(4, 5, kt.f32(g)),
                    kt.entry(5, 5, kt.f32(b)), kt.entry(6, 5, kt.f32(1.0))])


def stop(fraction, c):
    return kt.join([kt.entry(1, 2, c), kt.entry(2, 5, kt.f32(fraction)), kt.entry(3, 5, kt.f32(0.5))])


def angle_gradient():
    """A GradientArchive as Keynote's import writes one, with an unknown field 99 that must survive."""
    return kt.join([
        kt.entry(1, 0, 0),
        kt.entry(2, 2, stop(0.0, color(0, 0, 0))),
        kt.entry(2, 2, stop(1.0, color(1, 1, 1))),
        kt.entry(3, 5, kt.f32(1.0)),
        kt.entry(4, 0, 0),
        kt.entry(5, 2, kt.join([kt.entry(2, 5, kt.f32(0.0))])),
        kt.entry(99, 0, 7),
    ])


def shape_style(gradient):
    """TSWP.ShapeStyleArchive: super (1) → shape_properties (11) → fill (1) → gradient (2), with neighbours."""
    fill = kt.join([kt.entry(2, 2, gradient)])
    props = kt.join([kt.entry(1, 2, fill), kt.entry(4, 0, 1)])
    return kt.join([kt.entry(1, 2, kt.join([kt.entry(2, 2, b"name"), kt.entry(11, 2, props)])), kt.entry(5, 0, 3)])


def archive(oid, kind, payload, extra=b""):
    mi = kt.join([kt.entry(1, 0, kind), kt.entry(2, 2, bytes([1, 0, 5])), kt.entry(3, 0, len(payload))]) + extra
    info = kt.join([kt.entry(1, 0, oid), kt.entry(2, 2, mi)])
    return kt.put_varint(len(info)) + info + payload


def package(stream):
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as z:
        z.writestr("Index/DocumentStylesheet.iwa", kt.frame(stream))
        z.writestr("Metadata/DocumentIdentifier", b"id")
    return buf.getvalue()


class KeynoteTransformTest(unittest.TestCase):
    def stream(self):
        other = b"x" * 70000  # an object larger than one chunk
        return archive(548, 2025, shape_style(angle_gradient())) + archive(7, 1, other, kt.entry(5, 2, bytes([3]))[3])

    def test_round_trip_keeps_every_byte(self):
        stream = self.stream()
        self.assertEqual(kt.rebuild(kt.objects(stream)), stream)
        self.assertEqual(kt.stream_of(kt.frame(stream)), stream)

    def test_patch_replaces_the_angle_with_a_transform_and_fixes_the_length(self):
        stream = self.stream()
        case = {"id": "T", "style": 548, "transform": [[-1, 100], [601, 100], [600, 200]], "inflections": [0.2]}
        patched, changed = kt.patch_stream(stream, {548: case}, set())
        self.assertTrue(changed)
        objs = kt.objects(patched)
        self.assertEqual([kt.object_id(info) for info, _ in objs], [548, 7])
        self.assertEqual(objs[1][1][0][1], b"x" * 70000)
        style = objs[0][1][0][1]
        gradient = kt.parse(style)
        for field in (1, 11, 1, 2):
            gradient = kt.parse(kt.last(gradient, field)[2])
        fields = [e[0] for e in gradient]
        self.assertNotIn(5, fields)
        self.assertIn(99, fields)
        transform = kt.parse(kt.last(gradient, 6)[2])
        start = kt.parse(kt.last(transform, 1)[2])
        self.assertEqual(kt.last(start, 1)[2], kt.f32(-1))
        first = kt.parse([e for e in gradient if e[0] == 2][0][2])
        self.assertEqual(kt.last(first, 3)[2], kt.f32(0.2))
        # The neighbours of the edited messages keep their bytes.
        top = kt.parse(style)
        self.assertEqual(kt.last(top, 5)[3], kt.entry(5, 0, 3)[3])

    def test_patch_package_copies_other_files(self):
        import tempfile
        with tempfile.TemporaryDirectory() as d:
            src, dst = os.path.join(d, "in.key"), os.path.join(d, "out.key")
            with open(src, "wb") as f:
                f.write(package(self.stream()))
            saved = kt.CASES
            kt.CASES = [{"id": "T", "style": 548, "advanced": 1}]
            try:
                kt.patch(src, dst)
            finally:
                kt.CASES = saved
            with zipfile.ZipFile(src) as a, zipfile.ZipFile(dst) as b:
                self.assertEqual(a.namelist(), b.namelist())
                self.assertEqual(a.read("Metadata/DocumentIdentifier"), b.read("Metadata/DocumentIdentifier"))
                self.assertNotEqual(a.read("Index/DocumentStylesheet.iwa"), b.read("Index/DocumentStylesheet.iwa"))
            self.assertEqual(kt.round_trip(dst), [])


if __name__ == "__main__":
    unittest.main()
