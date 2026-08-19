#!/usr/bin/env bash
# Screenshot `swaypplet --preview lock` with each glass backend, for comparing
# the GL refraction against the GSK blur it replaces.
#
# Uses a nested headless sway rather than dev/render.sh's nested session,
# because a session whose outputs are asleep (`swaymsg -t get_outputs` showing
# `power: false`) sends no frame callbacks: GTK's frame clock stalls after two
# frames and grim blocks on screencopy. Nothing is captured from the real
# outputs. `--preview lock` builds a plain toplevel and never takes a session
# lock, so this is safe to run against a live desktop.
#
# usage: ./lock-glass-shot.sh <outdir> [wallpaper] [WIDTHxHEIGHT]
set -euo pipefail

OUT=${1:?usage: lock-glass-shot.sh <outdir> [wallpaper] [WxH]}
WALLPAPER=${2:-$HOME/Pictures/wallpapers/pluto-4k.jpg}
GEOM=${3:-1400x900}
HERE=$(cd "$(dirname "$0")" && pwd)
BIN=${SWAYPPLET_BIN:-$HERE/../target/debug/swaypplet}

[ -x "$BIN" ] || { echo "no binary at $BIN; cargo build first" >&2; exit 1; }
mkdir -p "$OUT"

# shoot <name> <backend> [extra env...]
# With LIVE=1 it grabs two frames a second apart instead of one, which is the
# only way to tell a live backdrop from a cached one: if the refraction is
# sampling a stale frame the two are identical.
shoot() {
  local name=$1 backend=$2 conf grab
  conf=$(mktemp)
  if [ "${LIVE:-0}" = 1 ]; then
    grab="sleep 4; grim \"\$OUT/$name-a.png\"; sleep 1; grim \"\$OUT/$name-b.png\"; swaymsg exit"
  else
    grab="sleep 4; grim \"\$OUT/$name.png\"; swaymsg exit"
  fi
  cat >"$conf" <<EOF
output HEADLESS-1 mode $GEOM
default_border none
exec env SWAYPPLET_LOCK_GLASS=$backend \
    SWAYPPLET_LOCK_LIVE=${LIVE:-0} \
    SWAYPPLET_LOCK_VIDEO="${VIDEO:-}" \
    GST_PLUGIN_SYSTEM_PATH_1_0="${GST_PLUGIN_SYSTEM_PATH_1_0:-}" \
    SWAYPPLET_LOCK_GLASS_STATS=1 RUST_LOG=info \
    SWAYPPLET_LOCK_WALLPAPER="$WALLPAPER" \
    "$BIN" --preview lock >"$OUT/$name.log" 2>&1
exec_always --no-startup-id sh -c '$grab'
EOF
  # grim runs inside the nested compositor, so it needs that WAYLAND_DISPLAY;
  # sway's exec inherits it, which is why the capture is spawned from the
  # config rather than from here.
  sed -i "s|\$OUT|$OUT|g" "$conf"

  WLR_BACKENDS=headless \
  WLR_LIBINPUT_NO_DEVICES=1 \
  WLR_HEADLESS_OUTPUTS=1 \
    sway -c "$conf" >"$OUT/sway-$name.log" 2>&1 &
  local pid=$!
  for _ in $(seq 1 25); do
    sleep 1
    kill -0 "$pid" 2>/dev/null || break
  done
  kill "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  rm -f "$conf"
}

shoot gl gl
shoot gsk gsk

ls -1 "$OUT"/*.png 2>/dev/null || { echo "no frames; see $OUT/sway-*.log" >&2; exit 1; }
