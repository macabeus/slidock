# parity

Parity of a run with slidock's goldens, at two levels: B, does this compiler compile and render each row as the
goldens do; C, where does a divergence start in the IR. Counts only: the reference and the per-row reports name
corpus rows and stay private, under `$SLIDOCK_PRIVATE/bench/cache/parity/golden-<commit12>/` and
`runs/<key>/parity.json`.

The goldens are slidock's own outputs, recorded by `cargo xtask golden --record` from the CLI built at one commit into
`$SLIDOCK_PRIVATE/golden/<commit>/` and pinned file by file in the committed `tests/golden/manifest.json`. Each
corpus pair's static-lane output (`compile --emit documents,report,ir --fonts`) sits in
`corpus/<presentation>.<frontend>/`.

1. `slidock-bench parity reference` checks every pinned file against its sha256 (a missing or changed file refuses
   the set), renders and scores each pair's golden output with this bench, and writes the hash-only reference
   (`reference.json`: per row the composed document's and the render's sha256, the class, scores and diagnostic
   counts), keyed under the bench's harness hash, renderer recipe, font set, metric versions and classifier.
   `--presentation` narrows it to some presentations: a check then covers only runs of those.
2. `slidock-bench parity check [--run latest] [--strict]` compares a static run with it: per row `identical`,
   `pixel-identical`, `drift-better`, `drift-same-class`, `regressed`, `missing` or `new`, with the first differing
   path of the row's IR slice against the golden IR; per IR container, equal or the first differing path. A run's
   row outside the holdout that the reference lacks is `unreferenced`. It exits 1 on a row that needs review and
   `allow.json` does not list, or on an unreferenced row; 2 when the run is stamped under another harness, renderer,
   font set or metrics than the reference, or the reference is of another golden manifest; and 3 when no row was
   compared.

`allow.json` lists reviewed divergences: the row, the reason, the start of its first differing IR path, the review
date and, optionally, the one status it accepts. A change to the goldens' outputs is either a decision, recorded with
a new golden recording and its reason, or a defect.
