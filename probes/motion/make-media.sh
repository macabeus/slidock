#!/bin/sh
# Writes the probe decks' media: a one-second 64x36 H.264 clip (a colour card that counts 1..10, our own pixels)
# and its poster frame. The outputs are committed; xtask probes-motion embeds them. Rerun only to change them:
# encoder versions change the bytes, so regenerate the decks afterwards (cargo xtask probes-motion).
set -eu
cd "$(dirname "$0")/media"
ffmpeg -hide_banner -loglevel error -y -f lavfi -i "color=c=0x2f6fdf:s=64x36:r=10:d=1,drawbox=x='t*60':y=12:w=6:h=12:color=white:t=fill" \
  -c:v libx264 -preset veryslow -profile:v baseline -pix_fmt yuv420p -x264-params "threads=1:sliced-threads=0" \
  -flags +bitexact -fflags +bitexact -map_metadata -1 -movflags +faststart clip.mp4
ffmpeg -hide_banner -loglevel error -y -i clip.mp4 -frames:v 1 -flags +bitexact -fflags +bitexact -map_metadata -1 poster.png
ls -l clip.mp4 poster.png
