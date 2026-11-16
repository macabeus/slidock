#!/bin/sh
# Rebuilds every fixture in this directory: make_fixtures.py with fontTools, then in Docker the reference WOFF2
# encoder and decoder (google/woff2 at a pinned commit) and W3C eot-utils 1.1 (Debian bookworm) for the outputs
# fontTools can't make, sfntly's MicroType Express encoder and libeot's decoder (both at pinned commits).
#
# Usage: make_fixtures.sh <dir with the downloaded fonts> <python with fontTools 4.60.1 and brotli 1.1.0>
set -eu
# fontTools stamps head.modified with this time (2026-10-30T02:23:29Z).
export SOURCE_DATE_EPOCH=1793406209
here=$(cd "$(dirname "$0")" && pwd)
"$2" -I "$here/make_fixtures.py" "$1" "$here"
woff2=slidock-woff2:0f4d304
printf '%s\n' \
  'FROM debian:bookworm-slim' \
  'RUN apt-get update && apt-get install -y --no-install-recommends build-essential cmake libbrotli-dev git ca-certificates eot-utils=1.1-1+b1 && rm -rf /var/lib/apt/lists/*' \
  'RUN git clone https://github.com/google/woff2.git /src && cd /src && git checkout 0f4d304faa1c62994536dc73510305c7357da8d4 && mkdir out && cd out && cmake .. && make woff2_compress woff2_decompress && cp woff2_compress woff2_decompress /usr/local/bin/' |
  docker build -q -t "$woff2" - >/dev/null
docker run --rm -v "$here:/w" -w /w "$woff2" sh -c '
  set -eu
  woff2_compress pair.ttc >/dev/null
  for f in *.woff2; do cp "$f" /tmp/x.woff2; woff2_decompress /tmp/x.woff2 >/dev/null; mv /tmp/x.ttf "$f.g.ttf"; done
  mkeot barlow.ttf http://example.com/ > barlow-mkeot.eot'
sfntly=slidock-sfntly:a56f578
printf '%s\n' \
  'FROM debian:bookworm-slim' \
  'RUN apt-get update && apt-get install -y --no-install-recommends openjdk-17-jdk-headless git ca-certificates && rm -rf /var/lib/apt/lists/*' \
  'RUN git clone https://github.com/googlefonts/sfntly.git /src && cd /src && git checkout a56f5782f209771aa226063757d57e6b5c948478 && cd java && mkdir /out && javac -nowarn -encoding UTF-8 -cp lib/icu4j-4_8_1_1.jar:lib/icu4j-charset-4_8_1_1.jar:lib/jcommander-1.27.jar -d /out $(find src -name "*.java")' |
  docker build -q -t "$sfntly" - >/dev/null
libeot=slidock-libeot:0407abd
printf '%s\n' \
  'FROM debian:bookworm-slim' \
  'RUN apt-get update && apt-get install -y --no-install-recommends build-essential autoconf automake libtool pkg-config git ca-certificates && rm -rf /var/lib/apt/lists/*' \
  'RUN git clone https://github.com/umanwizard/libeot.git /src && cd /src && git checkout 0407abddc581d32e9871ee41535183ee1d924d85 && autoreconf -fi && ./configure && make && make install && ldconfig' |
  docker build -q -t "$libeot" - >/dev/null
docker run --rm -v "$here:/w" -w /w "$sfntly" sh -c '
  set -eu
  for f in barlow oswald robotomono roboto-hdmx; do
    java -cp /out:/src/java/lib/icu4j-4_8_1_1.jar:/src/java/lib/icu4j-charset-4_8_1_1.jar \
      com.google.typography.font.tools.sfnttool.SfntTool -e -x "$f.ttf" "$f-mtx.eot"
  done'
docker run --rm -v "$here:/w" -w /w "$libeot" sh -c '
  set -eu
  for f in *-mtx.eot; do eot2ttf "$f" "$f.libeot.ttf"; done'
