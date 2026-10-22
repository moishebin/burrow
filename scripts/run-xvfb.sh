#!/usr/bin/env bash
# Runs the Linux build of Burrow on a virtual X display (Xvfb), for testing with real input
# while nobody is at the screen (D-015).
#
#   scripts/run-xvfb.sh               start (or restart) Burrow on :99, DevTools on :9222
#   scripts/run-xvfb.sh stop          stop Burrow and the display
#   BURROW_XVFB_DISPLAY=:98 …         another display (e.g. a second checkout testing at once)
#
# Then: DISPLAY=:99 xdotool mousemove 100 200 click 1
#       python3 scripts/xshot.py /tmp/shot.png
#       node scripts/cdp.mjs sidebar eval "view.state.spaces.length"
set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
display="${BURROW_XVFB_DISPLAY:-:99}"
data="${BURROW_DATA_DIR:-$repo/target/xvfb-data}"

pkill -f "$repo/target/debug/burrow" 2>/dev/null || true
if [[ "${1:-}" == "stop" ]]; then
  pkill -f "Xvfb $display" 2>/dev/null || true
  exit 0
fi
sleep 0.5

if ! pgrep -f "Xvfb $display" >/dev/null; then
  Xvfb "$display" -screen 0 1600x1000x24 -nolisten tcp >/dev/null 2>&1 &
  sleep 1
fi

ln -sfn "$repo/ui" "$repo/target/debug/ui"
mkdir -p "$data"
# Force X11 (WSLg also offers Wayland) and software GL (Xvfb has no GPU).
env -u WAYLAND_DISPLAY DISPLAY="$display" BURROW_DATA_DIR="$data" \
  nohup "$repo/target/debug/burrow" --ozone-platform=x11 --disable-gpu --remote-debugging-port=9222 \
  >"$data/stdout.log" 2>&1 &
echo "Burrow on $display (data: $data, log: $data/stdout.log)"
