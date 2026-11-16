#!/usr/bin/env bash
# Measures Keynote's movie of the transition catalog (transition-catalog.m4v, probes/keynote/RUNBOOK.md): where each
# transition plays and how long it lasts, and a contact sheet of each at 0, 25, 50, 75 and 100 % for reading its
# direction. Keynote's export holds each slide 5 s and plays slide N's transition between slides N and N + 1.
#
#   probes/keynote/movie-transitions.sh <transition-catalog.m4v> <out dir>
#
# Writes <out dir>/segments.tsv (start frame, end frame, start s, length s of every run of changing frames longer
# than 0.6 s; 30 fps), <out dir>/frames/t<k>_<f>.png and <out dir>/sheet_<k>.png. Needs ffmpeg and ImageMagick.
set -euo pipefail
movie="$1"
out="$2"
mkdir -p "$out/frames"
ffmpeg -v error -i "$movie" \
  -vf "scale=192:108,format=gray,tblend=all_mode=difference,signalstats,metadata=print:key=lavfi.signalstats.YAVG:file=$out/diff.txt" \
  -f null -
python3 -I - "$out/diff.txt" > "$out/segments.tsv" <<'PY'
import re, sys
values = [float(m.group(1)) for line in open(sys.argv[1]) for m in [re.search(r'YAVG=([\d.]+)', line)] if m]
runs, start = [], None
for i, v in enumerate(values + [0.0]):
    if v > 0.05 and start is None:
        start = i
    elif v <= 0.05 and start is not None:
        if runs and start - runs[-1][1] < 4:
            runs[-1] = (runs[-1][0], i)
        else:
            runs.append((start, i))
        start = None
print("start_frame\tend_frame\tstart_s\tlength_s")
for a, b in runs:
    if b - a >= 18:
        print(f"{a}\t{b}\t{a / 30:.3f}\t{(b - a) / 30:.3f}")
PY
# Transition k starts 5 s + 6.5 s × (k − 1) into the movie (5 s hold, 1.5 s transition).
count=$(( $(wc -l < "$out/segments.tsv") - 1 ))
for k in $(seq 1 42); do
  start=$(python3 -I -c "print(5 + 6.5 * ($k - 1))")
  frames=()
  for f in 0.0 0.25 0.5 0.75 1.0; do
    t=$(python3 -I -c "print(round($start + 0.05 + 1.4 * $f, 3))")
    ffmpeg -v error -y -ss "$t" -i "$movie" -frames:v 1 -vf scale=320:180 "$out/frames/t${k}_$f.png"
    frames+=("$out/frames/t${k}_$f.png")
  done
  magick "${frames[@]}" +append "$out/sheet_$k.png"
done
echo "$count runs of change longer than 0.6 s; 42 sheets in $out"
