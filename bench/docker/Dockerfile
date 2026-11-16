# The bench's renderer: one pinned Chromium (the headless shell Playwright 1.60 runs for `headless: true`) and one
# pinned poppler, so both sides of every comparison render the same way on every machine. The pinned font set is
# mounted read-only at /fonts, the only folder fontconfig sees. The bench drives Chromium over CDP from the host:
# the headless shell listens on the container's loopback only, and socat publishes that port on 9222.
#
# The base is the linux/arm64 image of mcr.microsoft.com/playwright:v1.60.0-noble (index digest
# sha256:9bd26ad900bb5e0f4dee75839e957a89ae89c2b7ab1e76050e559790e946b948), pinned by its own digest: poppler
# rasterises the same page to different bytes on arm64 and amd64.
FROM --platform=linux/arm64 mcr.microsoft.com/playwright:v1.60.0-noble@sha256:beddddcf96f19abec078cd2f3da676f40abd460e0e734ad5eed73627ce161d33

# Packages come from noble's release pocket only, which stays fixed after the release; snapshot.ubuntu.com lacks the
# arm64 ports archive.
RUN sed -i -E 's/^Suites: .*/Suites: noble/' /etc/apt/sources.list.d/ubuntu.sources \
 && apt-get update \
 && apt-get install -y --no-install-recommends poppler-utils=24.02.0-1ubuntu9 fontconfig socat=1.8.0.0-4build3 \
 && rm -rf /var/lib/apt/lists/*

COPY fonts.conf /etc/slidock-bench/fonts.conf
ENV FONTCONFIG_FILE=/etc/slidock-bench/fonts.conf
COPY chrome-args.txt start-chrome.sh /opt/slidock-bench/
RUN test -x /ms-playwright/chromium_headless_shell-1223/chrome-linux/headless_shell

EXPOSE 9222
ENTRYPOINT ["/opt/slidock-bench/start-chrome.sh"]
