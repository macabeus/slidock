# Grown-text probe (Keynote, attended)

A text box Keynote sizes to its text (geometry flags without `WIDTH_VALID`) wraps at the slide's edges, measured on
the slide, not in its group's coordinates: a label under an arrow, centred 262.875 pt into its group, would
otherwise wrap at 525.75 pt where Keynote draws it on one line. The probe holds such boxes with only our own text;
the oracle is Keynote 14.5's PDF export, so it is made on the Mac by hand.

## 1. The deck

`cargo xtask probes-grown-text` writes `probes/out/grown-text-probe.pptx`: one 960 × 540 pt slide of 24 pt Liberation
Sans labels (`wrap="none"`, `a:spAutoFit`, 40 pt wide frames) in groups as wide as their arrows. The module doc of
`xtask/src/probes_grown_text.rs` lists what each box pins.

Import it into Keynote 14.5 and save it as `~/.slidock/probes-incoming/keynote/grown-text-probe.key`. Then check what
Keynote kept:

```sh
slidock compile ~/.slidock/probes-incoming/keynote/grown-text-probe.key --emit ir --phase source -o /tmp/grown
```

Every label must read `"autofit": {"kind": "resize", "width": true, …}` and stay a child of its group (the nested one
two groups deep). If Keynote's import gives the labels a fixed width (`"kind": "none"` or no `"width": true`), or
ungroups them, build the slide by hand instead (§2).

## 2. By hand, when the import does not keep the property

New Keynote document, Basic White, 1920 × 1080; every label is a Text box (Insert → Text Box), Liberation Sans 48 pt,
black:

1. **Centred in a group:** an arrow (Shapes → right arrow) at x 1120, y 80, 480 × 80. A text box holding only `x`,
   alignment centre, centred under the arrow at x ≈ 1360. Select both, Arrange → Group. Then double-click into the
   group and type the label `A centred label wider than its group` (the group keeps its width; the text box grows
   from its centre).
2. **Centred, no group:** the same label, ungrouped, centred at x 1360, y 360.
3. **Left in a group:** an arrow at x 1400, y 560, 400 × 60; a left-aligned `x` text box at x 1420 under it; group;
   type `Left label running past the edge` into it.
4. **Right in a group:** an arrow at x 600, y 560, 600 × 60; a right-aligned `x` text box whose right edge is at
   x 1000; group; type `A right label grows left`.
5. **Centred in nested groups:** an arrow at x 480, y 880, 400 × 60 and a centred `x` text box at x 680 under it;
   group them; add an arrow at x 80, y 800, 1200 × 60; group it with the first group; type
   `A nested centred label, two groups deep` into the label.

Check with `slidock compile … --emit ir --phase source` as in §1: each label `resize` with `"width": true`, inside its
group(s).

## 3. Oracle and bench entry

Copy the deck to `~/.slidock/corpus/grown-text-probe.key`, add the entry below to `~/.slidock/bench/corpus/manifest.json`
(sorted keys, 2-space indent; `bytes`, `sha256` and `rows`, the slide ids in order, from the deck), then let the bench
export the oracle through Keynote: `slidock-bench oracle --presentation grown-text-probe --record`.

```json
{
  "features": ["element.group", "geometry.preset", "text.autofit.resize", "text.run"],
  "id": "grown-text-probe",
  "oracle": {
    "exporter": { "build": "7045.0.17", "bundleId": "com.apple.iWork.Keynote", "version": "14.5" },
    "fonts": ["LiberationSans-Regular"],
    "kind": "keynote-export",
    "source": "grown-text-probe.key"
  },
  "sources": [{ "bytes": 0, "file": "grown-text-probe.key", "frontend": "keynote", "rows": ["kn-…"], "sha256": "…" }],
  "tier": "probe",
  "title": "Grown-text probe (Keynote 14.5): text boxes sized to their text inside groups"
}
```

Its row fails if a grouped label wraps at its group again: with the group's coordinates, the centred, right and nested
labels wrap into several lines where Keynote draws one.
