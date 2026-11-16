# LibreOffice headless for the font probe decks, pinned: the Debian image by digest and its packages by snapshot date.
FROM debian:trixie-slim@sha256:f6b08b09a05725f9431502d3b5714f5c4576e82c1a84fa1debe0c24a7e0c3ec4
ARG SNAPSHOT=20261030T000000Z
RUN rm -f /etc/apt/sources.list.d/debian.sources \
 && echo "deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/${SNAPSHOT} trixie main" > /etc/apt/sources.list \
 && apt-get update \
 && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends libreoffice-impress-nogui python3-uno fontconfig \
 && rm -rf /var/lib/apt/lists/* \
 && rm -f /usr/share/fonts/truetype/dejavu/*.ttf && fc-cache -f
# Only the probe's own fonts (and OpenSymbol) are installed, so LibreOffice embeds only them.
RUN useradd -m probe
USER probe
WORKDIR /work
