#!/usr/bin/env bash
# Capture one PNG per preset without touching the live session.
#
# The demo needs frame callbacks to render at all, and a session whose outputs
# are asleep (`swaymsg -t get_outputs` shows `power: false`) does not send any
# — GTK's frame clock stalls after two frames and grim blocks on screencopy.
# So this runs the demo inside a nested headless sway, which drives frames
# regardless, and lets the demo's own `--shot` mode read the framebuffer back
# with glReadPixels. Nothing is captured from the real outputs and the user's
# displays are left as they were.
#
# usage: ./capture.sh <outdir> [wallpaper] [WIDTHxHEIGHT]
set -euo pipefail

OUT=${1:?usage: capture.sh <outdir> [wallpaper] [WxH] [testcard 0..1]}
WALLPAPER=${2:-$HOME/Pictures/wallpapers/pluto-4k.jpg}
GEOM=${3:-1600x1000}
CARD=${4:-0.45}
HERE=$(cd "$(dirname "$0")" && pwd)
BIN=${GLASS_DEMO_BIN:-$HERE/../../target/debug/glass-demo}

[ -x "$BIN" ] || { echo "no binary at $BIN; cargo build first" >&2; exit 1; }
mkdir -p "$OUT"

CONF=$(mktemp)
trap 'rm -f "$CONF"' EXIT
cat >"$CONF" <<EOF
output HEADLESS-1 mode ${GEOM/x/x}
default_border none
exec env GLASS_DEMO_TESTCARD=$CARD "$BIN" --shot "$OUT" "$WALLPAPER"; swaymsg exit
EOF

# A nested sway on the headless backend: its own wayland socket, its own
# output, no input devices to grab.
WLR_BACKENDS=headless \
WLR_LIBINPUT_NO_DEVICES=1 \
WLR_HEADLESS_OUTPUTS=1 \
  sway -c "$CONF" >"$OUT/sway.log" 2>&1 &
SWAY=$!

for _ in $(seq 1 60); do
  sleep 1
  kill -0 "$SWAY" 2>/dev/null || break
done
kill "$SWAY" 2>/dev/null || true
wait "$SWAY" 2>/dev/null || true

ls -1 "$OUT"/*.png 2>/dev/null || { echo "no frames captured; see $OUT/sway.log" >&2; exit 1; }
