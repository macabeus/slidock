#!/bin/sh
# Exports the callout probe decks to PDF with the pinned LibreOffice (the bench's slidock-probes-lo image, built from
# probes/fonts/Dockerfile; LibreOffice 25.2.3.2), with no network and the bench's Liberation Sans as the only text
# font: the oracles of the callout-probe-pptx and callout-probe-odp bench entries.
#
#   probes/callouts/oracle.sh <fonts dir> <out dir>
#
# <fonts dir> holds the bench's LiberationSans-*.ttf (`slidock-bench fonts link <dir> --releases`, then copy the files
# themselves: the container cannot follow links out of its mount). <out dir> receives in/ (copies of the decks), pdf/
# (one PDF per deck) and version.txt. Check each PDF's fonts with pdffonts (Liberation Sans only). A PDF's bytes
# change with every export (/CreationDate, /ID); its pages rasterise the same.
set -eu
fonts="$(cd "$1" && pwd)"
mkdir -p "$2"
out="$(cd "$2" && pwd)"
here="$(cd "$(dirname "$0")" && pwd)"
image="slidock-probes-lo:trixie-20261030"
mkdir -p "$out/in" "$out/pdf"
for deck in callout-probe.pptx callout-probe.odp; do
  cp "$here/../out/$deck" "$out/in/$deck"
done
docker run --rm --network none -v "$out:/work" -v "$fonts:/usr/local/share/fonts:ro" -e HOME=/tmp/home "$image" sh -c \
  'soffice --version > /work/version.txt 2>/dev/null; cd /work/in && for deck in *; do
     mkdir -p "/work/pdf/${deck##*.}" && soffice --headless --norestore --convert-to pdf --outdir "/work/pdf/${deck##*.}" "$deck"
   done'
cat "$out/version.txt"
ls -l "$out"/pdf/*/*.pdf
