#!/bin/sh
# Writes the shadow probe decks' media, all our own pixels: a 1000x600 JPEG (a four-colour gradient with white discs
# and a dark bar, tagged 72 dpi so LibreOffice reads it as 1000 x 600 pt), a one-second 320x180 H.264 clip of
# ffmpeg's test pattern and its poster frame. The outputs are committed; xtask probes-shadows embeds them. Rerun only
# to change them: encoder versions change the bytes, so regenerate the decks afterwards (cargo xtask probes-shadows).
set -eu
cd "$(dirname "$0")/media"
magick -size 1000x600 xc: -sparse-color bilinear '0,0 #04455E 999,0 #FE7867 0,599 #F9A825 999,599 #2E7D32' \
  -fill white -draw 'circle 250,300 250,180' -draw 'circle 700,200 700,140' -fill '#1F2937' -draw 'rectangle 560,380 940,460' \
  -strip -density 72 -units PixelsPerInch -sampling-factor 4:2:0 -quality 88 photo.jpg
ffmpeg -hide_banner -loglevel error -y -f lavfi -i "testsrc=size=320x180:rate=10:duration=1" \
  -c:v libx264 -preset veryslow -profile:v baseline -pix_fmt yuv420p -x264-params "threads=1:sliced-threads=0" \
  -flags +bitexact -fflags +bitexact -map_metadata -1 -movflags +faststart clip.mp4
ffmpeg -hide_banner -loglevel error -y -i clip.mp4 -frames:v 1 -flags +bitexact -fflags +bitexact -map_metadata -1 poster.png
ls -l photo.jpg clip.mp4 poster.png
