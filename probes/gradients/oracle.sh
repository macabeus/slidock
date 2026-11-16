#!/bin/sh
# Exports the gradient probe decks to PDF with the pinned LibreOffice (the bench's slidock-probes-lo image, built from
# probes/fonts/Dockerfile; LibreOffice 25.2.3.2), with no network and the bench's Liberation Sans as the only text
# font: the oracles of the gradient-probe-pptx, gradient-probe-odp and gradient-probe-odp12 bench entries.
#
#   probes/gradients/oracle.sh <fonts dir> <out dir>
#
# <fonts dir> holds the bench's LiberationSans-*.ttf (`slidock-bench fonts link <dir> --releases`, then copy the files
# themselves: the container cannot follow links out of its mount). <out dir> receives in/ (copies of the decks), pdf/
# (one PDF per deck), version.txt and pdf/pptx/gradient-probe.lo-valid.pdf: the pages of gradient-probe.pdf whose
# slides gradient-probe.cases.json marks LibreOffice-valid, in slide order (poppler's pdfseparate and pdfunite). Check
# each PDF's fonts with pdffonts (Liberation Sans only). A PDF's bytes change with every export (/CreationDate, /ID);
# its pages rasterise the same.
set -eu
fonts="$(cd "$1" && pwd)"
mkdir -p "$2"
out="$(cd "$2" && pwd)"
here="$(cd "$(dirname "$0")" && pwd)"
image="slidock-probes-lo:trixie-20261030"
mkdir -p "$out/in" "$out/pdf"
for deck in gradient-probe.pptx gradient-probe.odp gradient-probe-odp12.odp; do
  cp "$here/../out/$deck" "$out/in/$deck"
done
docker run --rm --network none -v "$out:/work" -v "$fonts:/usr/local/share/fonts:ro" -e HOME=/tmp/home "$image" sh -c \
  'soffice --version > /work/version.txt 2>/dev/null; cd /work/in && for deck in *; do
     mkdir -p "/work/pdf/${deck##*.}" && soffice --headless --norestore --convert-to pdf --outdir "/work/pdf/${deck##*.}" "$deck"
   done'
# The LibreOffice-valid slides of the PPTX, by their 1-based numbers in the cases file.
pages="$(python3 -I -c 'import json, sys
cases = json.load(open(sys.argv[1]))
deck = next(d for d in cases["decks"] if d["file"] == "gradient-probe.pptx")
print(" ".join(str(s["slide"]) for s in deck["slides"] if s["loValid"]))' "$here/../out/gradient-probe.cases.json")"
split="$out/split"
rm -rf "$split" && mkdir -p "$split"
pdfseparate "$out/pdf/pptx/gradient-probe.pdf" "$split/%d.pdf"
set --
for page in $pages; do
  set -- "$@" "$split/$page.pdf"
done
pdfunite "$@" "$out/pdf/pptx/gradient-probe.lo-valid.pdf"
rm -rf "$split"
cat "$out/version.txt"
echo "LibreOffice-valid PPTX slides: $pages"
ls -l "$out"/pdf/*/*.pdf
