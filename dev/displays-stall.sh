#!/usr/bin/env bash
# Main-loop stall test for Settings > Displays. Boots a nested headless sway
# (own dbus, never the live session), a 2880x1800 output at scale 2 like the
# laptop's eDP-1, opens the Displays page and picks every Scale, Resolution,
# Refresh rate and Rotation choice. Passes when the page logs
# "displays-cycle: done" within the time limit and the process stays under
# 60 % CPU afterwards; a hung main loop never logs it.
#   dev/displays-stall.sh [path/to/swaypplet]
set -uo pipefail
if [ -z "${SWPP_DBUS:-}" ]; then
  exec env SWPP_DBUS=1 dbus-run-session -- "$0" "$@"
fi
BIN="${1:-${SWAYPPLET_BIN:-swaypplet}}"
LIMIT="${SWPP_LIMIT:-60}"
RUNTIME="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
CFG="$(mktemp /tmp/swpp-stall-XXXX.conf)"; LOG="$(mktemp /tmp/swpp-stall-XXXX.log)"
APP="$(mktemp /tmp/swpp-stall-XXXX.app.log)"
printf 'output HEADLESS-1 resolution 2880x1800 position 0 0 scale 2\nxwayland disable\nfor_window [app_id="dev.swaypplet..*"] floating enable\n' > "$CFG"
export SWAYSOCK="$RUNTIME/sway-stall-$$.sock"; unset I3SOCK
WLR_HEADLESS_OUTPUTS=1 WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 sway -d --config "$CFG" >"$LOG" 2>&1 &
SWAY=$!; APPPID=""
cleanup() { [ -n "$APPPID" ] && kill "$APPPID" 2>/dev/null; kill "$SWAY" 2>/dev/null; rm -f "$CFG" "$LOG"; }
trap cleanup EXIT
for _ in $(seq 1 80); do swaymsg -t get_version >/dev/null 2>&1 && break; sleep 0.1; done
for _ in $(seq 1 40); do WD="$(grep -oE "wayland display '[^']+'" "$LOG" | head -1 | sed "s/.*'\(.*\)'/\1/")"; [ -n "$WD" ] && break; sleep 0.1; done
[ -n "${WD:-}" ] || { echo "no wayland display"; exit 2; }
export WAYLAND_DISPLAY="$WD" SWAYPPLET_DRY_SESSION=1 RUST_LOG=warn SWAYPPLET_PREVIEW_DISPLAYS_CYCLE=1
# The headless output is a screen of one's own, not an extra screen.
export SWAYPPLET_SCREENS="${SWAYPPLET_SCREENS:-HEADLESS-1}"
rm -f "$RUNTIME/swaypplet.pid"
"$BIN" --preview settings.displays >"$APP" 2>&1 &
APPPID=$!
for i in $(seq 1 "$LIMIT"); do
  grep -q 'displays-cycle: done' "$APP" && break
  kill -0 "$APPPID" 2>/dev/null || { echo "FAIL: exited"; tail -20 "$APP"; exit 1; }
  sleep 1
done
if ! grep -q 'displays-cycle: done' "$APP"; then
  echo "FAIL: stalled; last log:"; tail -5 "$APP"
  echo "cpu ticks: $(awk '{print $14+$15}' /proc/$APPPID/stat)"
  [ -n "${SWPP_KEEP_STALLED:-}" ] && { echo "pid $APPPID kept"; trap - EXIT; }
  exit 1
fi
echo "PASS: $(grep -c 'displays-cycle: step' "$APP") steps"
