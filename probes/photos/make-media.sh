#!/bin/sh
# Writes the photo probe's raster media (our own pixels: blurred plasma, shapes): photographs larger than the probe
# draws them, for the download's picture compression. The outputs are committed; xtask probes-pictures embeds them.
# Rerun only to change them: encoder versions change the bytes, so regenerate the decks afterwards
# (cargo xtask probes-pictures).
#
# - photo.jpg: 2400 x 1600, drawn twice at different sizes.
# - crop.png: 1000 x 750, opaque, drawn with a quarter cut off each side.
# - logo.png: 1200 x 1200 with transparency.
# - turned.jpg: stored 1600 x 1200 with EXIF orientation 6 (a quarter turn clockwise): it shows 1200 x 1600.
# - fill.png: 800 x 800, an ellipse's image fill.
# - bg.jpg: 3000 x 1688, the slide's background: smaller than the size kept for a full slide, so it is not scaled.
# - anim.gif: 64 x 64, two frames: animated, so it stays as it is.
# - preview.png: 4 x 2, the PNG preview beside the probe's SVG.
set -eu
cd "$(dirname "$0")/media"
magick -size 600x400 -seed 7 plasma:'#2b6cb0'-'#f6ad55' -blur 0x3 -resize 2400x1600 -fill '#1a202c' \
  -draw 'circle 1700,500 1700,800' -fill white -draw 'rectangle 200,1250 1100,1300' \
  -draw 'rectangle 200,1350 900,1400' -strip -quality 85 photo.jpg
magick -size 250x188 -seed 11 plasma:'#38a169'-'#e53e3e' -blur 0x4 -resize '1000x750!' -fill '#f7fafc' \
  -draw 'rectangle 250,188 750,562' -fill '#2d3748' -draw 'circle 500,375 500,475' -strip \
  -define png:compression-level=9 PNG24:crop.png
magick -size 1200x1200 xc:none -fill '#d53f8c' -draw 'circle 600,600 600,80' -fill '#ffffffc0' \
  -draw 'roundrectangle 300,500 900,700 40,40' -strip PNG32:logo.png
magick -size 400x300 -seed 3 plasma:'#805ad5'-'#ecc94b' -blur 0x2 -resize 1600x1200 -fill black \
  -draw 'polygon 100,100 500,100 300,400' -strip -quality 85 turned-plain.jpg
# An Exif APP1 segment after SOI: a big-endian TIFF whose IFD0 states Orientation (274) = 6.
python3 - <<'PY'
import struct
tiff = b"MM\x00\x2a" + struct.pack(">I", 8) + struct.pack(">H", 1) + struct.pack(">HHI", 274, 3, 1) \
    + struct.pack(">HH", 6, 0) + struct.pack(">I", 0)
body = b"Exif\x00\x00" + tiff
plain = open("turned-plain.jpg", "rb").read()
open("turned.jpg", "wb").write(plain[:2] + b"\xff\xe1" + struct.pack(">H", len(body) + 2) + body + plain[2:])
PY
rm turned-plain.jpg
magick -size 200x200 -seed 5 plasma:'#319795'-'#f56565' -blur 0x4 -resize 800x800 -fill '#fefcbf' \
  -draw 'circle 400,400 400,240' -strip -define png:compression-level=9 PNG24:fill.png
magick -size 750x422 -seed 9 plasma:'#edf2f7'-'#a0aec0' -blur 0x4 -resize '3000x1688!' -strip -quality 80 bg.jpg
magick -size 64x64 -delay 50 xc:'#e53e3e' xc:'#3182ce' -loop 0 anim.gif
magick -size 4x2 xc:'#FE7867' -strip PNG24:preview.png
ls -l photo.jpg crop.png logo.png turned.jpg fill.png bg.jpg anim.gif preview.png
