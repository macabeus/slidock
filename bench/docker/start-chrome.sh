#!/bin/sh
# Starts the headless shell with Playwright 1.60's switches for `headless: true` (chromiumSwitches() and its headless
# additions) and the bench's own, one argument per line of chrome-args.txt, spaces kept. The shell's CDP port (9221)
# listens on the container's loopback only; socat forwards 9222, which the container publishes on the host's loopback.
set -eu
set --
while IFS= read -r arg; do
  [ -n "$arg" ] && set -- "$@" "$arg"
done < /opt/slidock-bench/chrome-args.txt
socat TCP-LISTEN:9222,fork,reuseaddr TCP:127.0.0.1:9221 &
exec /ms-playwright/chromium_headless_shell-1223/chrome-linux/headless_shell "$@"
