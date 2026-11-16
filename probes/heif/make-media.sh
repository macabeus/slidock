#!/bin/sh
# Writes the HEIF reader's and the web app's HEIC fixtures with Apple's encoder (macOS `sips`), from pictures drawn
# here (our own pixels): 8-bit HEVC Main Still Picture, as an iPhone writes its photos.
# - single.heic: 64 x 48, one coded image, nclx colour: four quadrants (red, green, blue, yellow), a white bar top left.
# - grid.heic: 1100 x 600, a grid of 3 x 2 tiles of 512 x 512 (the last column and row overhang the image), tagged
#   Display P3: each tile's area in its own colour, a white bar top left.
# - odd.heic: 61 x 45, the encoder's 62 x 46 coded image cut to it by a clean aperture (`clap`): single.heic's
#   quadrants and bar.
# The outputs are committed; rerun only to change them (encoder versions change the bytes), then the tests that pin
# their structure.
set -eu
cd "$(dirname "$0")"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
magick -size 64x48 xc:'#D23C3C' \
  -fill '#3CA03C' -draw 'rectangle 32,0 63,23' \
  -fill '#3C50C8' -draw 'rectangle 0,24 31,47' \
  -fill '#E6C83C' -draw 'rectangle 32,24 63,47' \
  -fill white -draw 'rectangle 4,4 19,9' -depth 8 "PNG24:$work/single.png"
magick -size 1100x600 xc:'#D23C3C' \
  -fill '#3CA03C' -draw 'rectangle 512,0 1023,511' \
  -fill '#3C50C8' -draw 'rectangle 1024,0 1099,511' \
  -fill '#E6C83C' -draw 'rectangle 0,512 511,599' \
  -fill '#8C3CB4' -draw 'rectangle 512,512 1023,599' \
  -fill '#3CB4B4' -draw 'rectangle 1024,512 1099,599' \
  -fill white -draw 'rectangle 16,16 207,63' -depth 8 "PNG24:$work/grid.png"
magick -size 61x45 xc:'#D23C3C' \
  -fill '#3CA03C' -draw 'rectangle 32,0 60,23' \
  -fill '#3C50C8' -draw 'rectangle 0,24 31,44' \
  -fill '#E6C83C' -draw 'rectangle 32,24 60,44' \
  -fill white -draw 'rectangle 4,4 19,9' -depth 8 "PNG24:$work/odd.png"
sips -s format heic "$work/single.png" --out single.heic >/dev/null
sips -s format heic "$work/odd.png" --out odd.heic >/dev/null
sips -m '/System/Library/ColorSync/Profiles/Display P3.icc' -s format heic "$work/grid.png" --out grid.heic >/dev/null
ls -l single.heic grid.heic odd.heic
