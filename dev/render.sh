#!/usr/bin/env bash
# Headless render harness for swaypplet visual validation.
#
# Boots a nested headless sway under its OWN dbus session (so swaypplet's
# GApplication single-instance lock doesn't defer to the live-session copy),
# runs a swaypplet binary inside it, opens the requested surface, and captures
# a PNG with grim. Lets us iterate on CSS/layout with real screenshots without
# a nixos rebuild or touching the live session.
#
# Usage:
#   dev/render.sh [--bin PATH] [--res WxH] [--out FILE] [--mode panel|launcher|polkit|preview:NAME] [--css FILE]
# No `set -e`: this harness polls with `cond && break` loops, which trip the
# set -e + &&-list gotcha (a false test exits the script). Errors are checked
# explicitly with `|| { …; exit 1; }` instead.
set -uo pipefail

# Re-exec under a private dbus session bus so GApplication single-instance
# doesn't hand off to the live-session swaypplet.
if [ -z "${SWPP_DBUS:-}" ]; then
  exec env SWPP_DBUS=1 dbus-run-session -- "$0" "$@"
fi

BIN="${SWAYPPLET_BIN:-swaypplet}"
RES="1200x1600"; OUT="/tmp/swaypplet-shot.png"; MODE="panel"; CSS="${SWAYPPLET_CSS:-}"
while [ $# -gt 0 ]; do
  case "$1" in
    --bin) BIN="$2"; shift 2;; --res) RES="$2"; shift 2;;
    --out) OUT="$2"; shift 2;; --mode) MODE="$2"; shift 2;;
    --css) CSS="$2"; shift 2;; *) echo "unknown arg: $1" >&2; exit 2;;
  esac
done
W="${RES%x*}"; H="${RES#*x}"
RUNTIME="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
CFG="$(mktemp /tmp/swpp-sway-XXXX.conf)"
LOG="$(mktemp /tmp/swpp-sway-XXXX.log)"
SOCK="$RUNTIME/sway-render-$$.sock"
{
  printf 'output HEADLESS-1 resolution %sx%s position 0 0 scale 1\n' "$W" "$H"
  printf 'default_border none\nxwayland disable\n'
  # Preview windows are normal toplevels; float them so they render at their
  # natural requested size instead of being tiled to fill the output.
  printf 'for_window [app_id="dev.swaypplet..*"] floating enable\n'
  # Mirror the live session's swayfx frost (users/modules/sway.nix in the
  # nixos repo) so screenshots show the glass the way users see it. On a
  # plain sway binary these lines log config errors and are ignored — the
  # harness still boots, just unfrosted.
  printf 'blur enable\nblur_passes 1\nblur_radius 5\n'
  for ns in swaypplet swaypplet-launcher swaypplet-osd swaypplet-notification swaypplet-polkit; do
    printf 'layer_effects "%s" { blur enable; blur_ignore_transparent enable; }\n' "$ns"
  done
} > "$CFG"

cleanup() { [ -n "${SWAY_PID:-}" ] && kill "$SWAY_PID" 2>/dev/null || true; rm -f "$CFG" "$LOG"; }
trap cleanup EXIT

export SWAYSOCK="$SOCK"
# -d so the "Running compositor on wayland display 'X'" line (INFO level) is
# logged; we parse the display name from it.
WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 sway -d --config "$CFG" >"$LOG" 2>&1 &
SWAY_PID=$!

for _ in $(seq 1 80); do swaymsg -t get_version >/dev/null 2>&1 && break; sleep 0.1; done
swaymsg -t get_version >/dev/null 2>&1 || { echo "sway IPC never came up"; cat "$LOG"; exit 1; }

WD=""
for _ in $(seq 1 40); do
  WD="$(grep -oE "wayland display '[^']+'" "$LOG" | head -1 | sed "s/.*'\(.*\)'/\1/")"
  [ -n "$WD" ] && break; sleep 0.1
done
[ -n "$WD" ] || { echo "could not determine WAYLAND_DISPLAY"; cat "$LOG"; exit 1; }
export WAYLAND_DISPLAY="$WD"
[ -n "$CSS" ] && export SWAYPPLET_CSS="$CSS"

rm -f /tmp/swaypplet.pid
case "$MODE" in
  polkit)    "$BIN" polkit-agent >/tmp/swpp-app.log 2>&1 & ;;
  preview:*) "$BIN" --preview "${MODE#preview:}" >/tmp/swpp-app.log 2>&1 & ;;
  *)         "$BIN" >/tmp/swpp-app.log 2>&1 & ;;
esac

mapped=""
for _ in $(seq 1 60); do
  swaymsg -t get_tree 2>/dev/null | grep -q '"app_id": *"[^"]*swaypplet' && { mapped=1; break; }
  sleep 0.1
done
if [ "$MODE" = "panel" ] || [ "$MODE" = "launcher" ]; then
  p="$(cat /tmp/swaypplet.pid 2>/dev/null || true)"
  [ -n "$p" ] && kill -USR1 "$p" 2>/dev/null || true
  for _ in $(seq 1 40); do
    swaymsg -t get_tree 2>/dev/null | grep -q '"app_id": *"[^"]*swaypplet' && { mapped=1; break; }
    sleep 0.1
  done
fi

# Capture once GTK has actually painted. The headless paint can lag the map by
# a variable margin, so re-grim until the PNG is clearly non-blank (a blank
# solid-color frame compresses to a tiny file) rather than guessing one delay.
captured=""
for _ in $(seq 1 10); do
  sleep 0.5
  grim -o HEADLESS-1 "$OUT" 2>/dev/null || true
  sz=$(stat -c '%s' "$OUT" 2>/dev/null || echo 0)
  if [ "$sz" -gt 6000 ]; then captured=1; break; fi
done
[ -z "$mapped" ] && { echo "WARNING: no swaypplet surface in tree"; echo "--- app log ---"; head -30 /tmp/swpp-app.log; }
[ -z "$captured" ] && echo "WARNING: capture stayed blank after retries"
echo "wrote $OUT (${W}x${H}, mode=$MODE, display=$WD)"
