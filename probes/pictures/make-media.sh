#!/bin/sh
# Writes the picture-types probe's raster media: a 32x18 card (a coral square on teal, our own pixels) as HEIC and as
# AVIF. The outputs are committed; xtask probes-pictures embeds them. Rerun only to change them: encoder versions change
# the bytes, so regenerate the decks afterwards (cargo xtask probes-pictures).
set -eu
cd "$(dirname "$0")/media"
for ext in heic avif; do
  magick -size 32x18 xc:'#04455E' -fill '#FE7867' -draw 'rectangle 9,2 22,15' -strip -quality 60 "card.$ext"
done
ls -l card.heic card.avif
