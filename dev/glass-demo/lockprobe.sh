#!/usr/bin/env bash
# Answer one question: does a GtkGLArea render on an ext-session-lock-v1
# surface? See src/lockprobe.rs for why that cannot be answered with
# `swaypplet --preview lock`.
#
# The lock is taken inside a nested headless sway, never in the host session.
# `ext-session-lock-v1` deliberately leaves a session locked when its locker
# dies, so a probe that got this wrong would lock you out of your own desktop;
# GLASS_DEMO_HOST_DISPLAY is exported here so the binary can refuse to run
# against the socket this script was started from, and it also refuses without
# GLASS_DEMO_LOCK_PROBE=1, and it unlocks on an unconditional timer.
#
# usage: ./lockprobe.sh [outdir] [wallpaper]
set -euo pipefail

OUT=${1:-}
WALLPAPER=${2:-$HOME/Pictures/wallpapers/pluto-4k.jpg}
HERE=$(cd "$(dirname "$0")" && pwd)
BIN=${GLASS_DEMO_BIN:-$HERE/../../target/debug/glass-demo}

[ -x "$BIN" ] || { echo "no binary at $BIN; cargo build first" >&2; exit 1; }

# The socket to refuse. Captured before the nested compositor exists, so it is
# unambiguously the outer one.
export GLASS_DEMO_HOST_DISPLAY=${WAYLAND_DISPLAY:-}
[ -n "$GLASS_DEMO_HOST_DISPLAY" ] || {
  echo "WAYLAND_DISPLAY unset; refusing to run without a host socket to exclude" >&2
  exit 1
}

LOG=$(mktemp)
CONF=$(mktemp)
trap 'rm -f "$CONF"' EXIT
SHOT_ARG=""
[ -n "$OUT" ] && { mkdir -p "$OUT"; SHOT_ARG="--shot $OUT"; }

cat >"$CONF" <<EOF
output HEADLESS-1 mode 1600x1000
default_border none
exec env GLASS_DEMO_LOCK_PROBE=1 \
    GLASS_DEMO_HOST_DISPLAY="$GLASS_DEMO_HOST_DISPLAY" \
    "$BIN" --lock $SHOT_ARG "$WALLPAPER" > "$LOG" 2>&1; swaymsg exit
EOF

WLR_BACKENDS=headless \
WLR_LIBINPUT_NO_DEVICES=1 \
WLR_HEADLESS_OUTPUTS=1 \
  sway -c "$CONF" >/dev/null 2>&1 &
SWAY=$!

for _ in $(seq 1 30); do
  sleep 1
  kill -0 "$SWAY" 2>/dev/null || break
done
kill "$SWAY" 2>/dev/null || true
wait "$SWAY" 2>/dev/null || true

cat "$LOG"
rm -f "$LOG"
