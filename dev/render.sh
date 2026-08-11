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
#   SWPP_SEED_CLIPBOARD=1 dev/render.sh --mode preview:clipboard   # rows to draw
#   dev/render.sh --mode keybinds --res 1600x1000                  # the held-Super sheet
#
# --mode keybinds copies the live session's bindsym lines into the nested
# config (SWPP_KEYBINDS_FROM overrides the source), because the sheet is
# derived from whatever config sway loaded and an empty one renders empty.
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
  # sway's parser wants the block across lines: a one-liner is read as an
  # unmatched '}' and the whole rule is dropped, which renders every surface
  # here unfrosted while looking like it worked.
  for ns in swaypplet swaypplet-launcher swaypplet-osd swaypplet-notification swaypplet-polkit swaypplet-keybinds; do
    printf 'layer_effects "%s" {\n    blur enable\n    blur_ignore_transparent enable\n}\n' "$ns"
  done
  # The keybinding sheet reads the config sway loaded, so a nested session
  # with no bindings renders an empty sheet. Borrow the outer session's
  # bindsym lines (harmless here — nothing presses them) so the harness
  # exercises the real IPC path against a realistic config.
  if [ "$MODE" = "keybinds" ]; then
    grep '^bindsym' "${SWPP_KEYBINDS_FROM:-$HOME/.config/sway/config}" 2>/dev/null || true
  fi
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

rm -f "$RUNTIME/swaypplet.pid"
case "$MODE" in
  polkit)    "$BIN" polkit-agent >/tmp/swpp-app.log 2>&1 & ;;
  keybinds)
    "$BIN" >/tmp/swpp-app.log 2>&1 &
    # The sheet is a surface of the running panel, so it needs the panel up
    # before the show command has anything to talk to.
    for _ in $(seq 1 200); do
      [ -e "$RUNTIME/swaypplet.pid" ] && break; sleep 0.1
    done
    sleep 1.5
    "$BIN" keybinds show >>/tmp/swpp-app.log 2>&1 || true
    ;;
  preview:*)
    "$BIN" --preview "${MODE#preview:}" >/tmp/swpp-app.log 2>&1 &
    # SWPP_SEED_CLIPBOARD=1 puts three selections on the nested session so
    # the clipboard section has rows to draw. It has to happen *after* the
    # app starts: the data-control watcher only ever sees selections made
    # while it is running, so seeding first shoots an empty list.
    if [ -n "${SWPP_SEED_CLIPBOARD:-}" ]; then
      sleep 2
      for t in "first copied line" "andra raden med åäö" "third one"; do
        printf '%s' "$t" | wl-copy && sleep 0.5
      done
    fi
    ;;
  *)         "$BIN" >/tmp/swpp-app.log 2>&1 & ;;
esac

mapped=""
for _ in $(seq 1 60); do
  swaymsg -t get_tree 2>/dev/null | grep -q '"app_id": *"[^"]*swaypplet' && { mapped=1; break; }
  sleep 0.1
done
if [ "$MODE" = "panel" ] || [ "$MODE" = "launcher" ]; then
  p="$(cat "$RUNTIME/swaypplet.pid" 2>/dev/null || true)"
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
