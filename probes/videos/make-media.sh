#!/bin/sh
# Writes the video probe's movies (our own content: ffmpeg's test patterns and tones), for the download's video
# compression. The outputs are committed; xtask probes-videos embeds them. Rerun only to change them: encoder versions
# change the bytes, so regenerate the deck afterwards (cargo xtask probes-videos). Needs ffmpeg with libx264 and, for
# the alpha movie, macOS's VideoToolbox HEVC encoder.
#
# - sound.mp4: 1280 x 720, 30 fps, 2 s, H.264 at a high bitrate with an AAC tone: drawn smaller than it is.
# - turned.mp4: stored 960 x 540 with a quarter-turn display matrix (it shows 540 x 960), no sound.
# - tiny.mp4: 320 x 180, 15 fps, 2 s at a low bitrate: already small, so it stays as it is.
# - pcm.mov: QuickTime, 1280 x 720, 25 fps, 1.6 s, with 16-bit PCM sound: drawn twice (once trimmed).
# - alpha.mov: HEVC with an alpha layer, 320 x 240, 1 s: H.264 has no alpha, so it stays as it is.
# - own.mp4: 640 x 360, 30 fps, 2 s at a high bitrate, its colour unstated: drawn larger than it is, so it is
#   re-encoded at its own size, its frames as decoded.
# - muted.mp4: 640 x 360, 30 fps, 2 s at a low bitrate with an AAC tone, every use muted: drawn at its own size and
#   already under the bitrate, so its video is copied as it is and its sound left out.
# - quiet.mp4: 960 x 540, 30 fps, 2 s with an AAC tone, played at a volume of 0: re-encoded smaller, without sound.
# - <name>.png: each movie's first frame as it shows, 160 pixels wide: its poster.
set -eu
cd "$(dirname "$0")/media"
ff() { ffmpeg -hide_banner -loglevel error -y "$@"; }
x264='-c:v libx264 -preset medium -pix_fmt yuv420p -colorspace bt709 -color_primaries bt709 -color_trc bt709'
# shellcheck disable=SC2086
ff -f lavfi -i 'testsrc2=s=1280x720:r=30:d=2' -f lavfi -i 'sine=f=440:r=48000:d=2' -filter:a 'volume=0.5' \
  $x264 -crf 20 -g 30 -c:a aac -b:a 96k -ac 2 -map_metadata -1 -fflags +bitexact -movflags +faststart sound.mp4
# shellcheck disable=SC2086
ff -f lavfi -i 'testsrc2=s=960x540:r=30:d=2' $x264 -crf 22 -g 30 -map_metadata -1 -fflags +bitexact turned-plain.mp4
# The display matrix goes on as the stream is copied (an encode would turn the pixels instead).
ff -display_rotation 90 -i turned-plain.mp4 -c copy -map_metadata -1 -fflags +bitexact turned.mp4
rm turned-plain.mp4
# shellcheck disable=SC2086
ff -f lavfi -i 'testsrc2=s=320x180:r=15:d=2' $x264 -b:v 40k -g 30 -map_metadata -1 -fflags +bitexact tiny.mp4
# shellcheck disable=SC2086
ff -f lavfi -i 'testsrc2=s=1280x720:r=25:d=1.6' -f lavfi -i 'sine=f=660:r=44100:d=1.6' -filter:a 'volume=0.5' \
  $x264 -crf 22 -g 25 -c:a pcm_s16le -ac 2 -map_metadata -1 -fflags +bitexact pcm.mov
ff -f lavfi -i 'testsrc2=s=640x360:r=30:d=2' -c:v libx264 -preset medium -pix_fmt yuv420p -crf 12 -g 30 \
  -map_metadata -1 -fflags +bitexact own.mp4
ff -f lavfi -i "testsrc2=s=320x240:r=30:d=1,format=bgra,geq=r='r(X,Y)':g='g(X,Y)':b='b(X,Y)':a='if(lt(X,160),255,96)'" \
  -c:v hevc_videotoolbox -alpha_quality 0.75 -tag:v hvc1 -map_metadata -1 -fflags +bitexact alpha.mov
# shellcheck disable=SC2086
ff -f lavfi -i 'testsrc2=s=640x360:r=30:d=2' -f lavfi -i 'sine=f=550:r=48000:d=2' -filter:a 'volume=0.5' \
  $x264 -b:v 150k -g 30 -c:a aac -b:a 128k -ac 2 -map_metadata -1 -fflags +bitexact -movflags +faststart muted.mp4
# shellcheck disable=SC2086
ff -f lavfi -i 'testsrc2=s=960x540:r=30:d=2' -f lavfi -i 'sine=f=330:r=48000:d=2' -filter:a 'volume=0.5' \
  $x264 -crf 20 -g 30 -c:a aac -b:a 128k -ac 2 -map_metadata -1 -fflags +bitexact -movflags +faststart quiet.mp4
for movie in sound.mp4 turned.mp4 tiny.mp4 pcm.mov alpha.mov own.mp4 muted.mp4 quiet.mp4; do
  ff -i "$movie" -frames:v 1 -vf 'scale=160:-2' -map_metadata -1 -fflags +bitexact "${movie%.*}.png"
done
ls -l sound.mp4 turned.mp4 tiny.mp4 pcm.mov alpha.mov own.mp4 muted.mp4 quiet.mp4 ./*.png
