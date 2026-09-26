#!/usr/bin/env bash
# Frame timing for swaypplet's transitions, in numbers.
#
# filmstrip.sh shows what a transition looks like; this says whether it
# keeps up. It boots the same nested headless sway, runs swaypplet with
# GDK_DEBUG=frames, opens and closes the panel and pops notifications a few
# times, and summarises the per-frame log that `SWAYPPLET_FRAME_STATS`
# makes swaypplet write (src/frame_stats.rs):
#
#   frames    frames drawn while something moved (a gap over 150 ms starts
#             a new animation and is not counted as an interval)
#   late      frames that came more than 25 ms after the previous one,
#             which is a skipped refresh at 60 Hz: what reads as a stutter
#   work p50/p95/max   ms of client work per frame (style, layout,
#             snapshot, render submit)
#   first p50/max      the same for a window's first frame, which is where
#             a surface that opens slowly shows
#
# Run it on two binaries on the same compositor to compare them:
#   dev/frame-bench.sh --bin ../swaypplet-old/target/release/swaypplet --sway …/bin/sway
#   dev/frame-bench.sh --bin target/release/swaypplet --sway …/bin/sway
#
# --pin first puts two terminals that redraw every 50 ms on workspace 24 and
# pins it, the way the binding does: the load a live pin puts on the
# compositor (a full-size capture per window frame) and on the main thread.
#
# --gate exits 1 when a limit below is crossed, for .githooks/pre-push. The
# limits sit above what the 2026-09-26 build measured over six runs (late
# 6.5-10.4 %, work p95 2.3-5.8 ms, first frame up to 270 ms), so noise does
# not trip them and a real regression does.
#
# The headless backend's refresh is simulated, so absolute numbers are not
# the laptop's; differences between two binaries on it are.
set -uo pipefail

if [ -z "${SWPP_DBUS:-}" ]; then
  exec env SWPP_DBUS=1 dbus-run-session -- "$0" "$@"
fi

BIN="target/release/swaypplet"
SWAY_BIN="${SWAYPPLET_SWAY:-sway}"
ROUNDS=6
RES="1400x900"
THEME="dark"
PIN=""
GATE=""
while [ $# -gt 0 ]; do
  case "$1" in
    --bin) BIN="$2"; shift 2;; --sway) SWAY_BIN="$2"; shift 2;;
    --rounds) ROUNDS="$2"; shift 2;; --res) RES="$2"; shift 2;;
    --theme) THEME="$2"; shift 2;;
    --pin) PIN=1; shift;;
    --gate) GATE=1; shift;;
    *) echo "unknown arg: $1" >&2; exit 2;;
  esac
done
W="${RES%x*}"; H="${RES#*x}"
RUNTIME="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
CFG="$(mktemp /tmp/swpp-bench-XXXX.conf)"
LOG="$(mktemp /tmp/swpp-bench-XXXX.log)"
APP="$(mktemp /tmp/swpp-bench-XXXX.app)"
SOCK="$RUNTIME/sway-bench-$$.sock"

{
  printf 'output HEADLESS-1 resolution %sx%s position 0 0 scale 1\n' "$W" "$H"
  printf 'output HEADLESS-1 bg #505860 solid_color\n'
  printf 'default_border none\nxwayland disable\n'
  printf 'blur enable\nblur_passes 1\nblur_radius 5\n'
} > "$CFG"

PIDFILE="$RUNTIME/swaypplet.pid"
SAVED_PID="$(cat "$PIDFILE" 2>/dev/null)"
cleanup() {
  [ -n "${APP_PID:-}" ] && kill "$APP_PID" 2>/dev/null
  [ -n "${SWAY_PID:-}" ] && kill "$SWAY_PID" 2>/dev/null
  # The pid file is shared with the live session; put its pid back.
  [ -n "${SAVED_PID:-}" ] && printf '%s' "$SAVED_PID" > "$PIDFILE"
  [ -n "${SWPP_BENCH_KEEP:-}" ] && cp "${STATS:-/dev/null}" "$SWPP_BENCH_KEEP"
  [ -n "${SWPP_BENCH_APPLOG:-}" ] && cp "$APP" "$SWPP_BENCH_APPLOG"
  rm -f "$CFG" "$LOG" "$APP" "${STATS:-}"
  return 0
}
trap cleanup EXIT

export SWAYSOCK="$SOCK"
# swaypplet reads I3SOCK first; an inherited one reaches the live session.
unset I3SOCK
WLR_BACKENDS=headless WLR_RENDERER=gles2 WLR_LIBINPUT_NO_DEVICES=1 \
  "$SWAY_BIN" -d --config "$CFG" >"$LOG" 2>&1 &
SWAY_PID=$!
for _ in $(seq 1 80); do swaymsg -t get_version >/dev/null 2>&1 && break; sleep 0.1; done
swaymsg -t get_version >/dev/null 2>&1 || { echo "sway IPC never came up"; tail -20 "$LOG"; exit 1; }
WD=""
for _ in $(seq 1 40); do
  WD="$(grep -oE "wayland display '[^']+'" "$LOG" | head -1 | sed "s/.*'\(.*\)'/\1/")"
  [ -n "$WD" ] && break; sleep 0.1
done
[ -n "$WD" ] || { echo "no WAYLAND_DISPLAY"; exit 1; }
export WAYLAND_DISPLAY="$WD"

rm -f "$PIDFILE"
export GTK_A11Y=none NO_AT_BRIDGE=1 SWAYPPLET_MODE="$THEME"
STATS="$(mktemp /tmp/swpp-bench-XXXX.stats)"
SWAYPPLET_FRAME_STATS="$STATS" "$BIN" >"$APP" 2>&1 &
APP_PID=$!
p=""
for _ in $(seq 1 100); do p="$(cat "$PIDFILE" 2>/dev/null)"; [ -n "$p" ] && break; sleep 0.1; done
[ -n "$p" ] || { echo "no pid file"; tail -20 "$APP"; exit 1; }
sleep 2
if [ -n "$PIN" ]; then
  ticker='i=0; while :; do i=$((i+1)); printf "\033[4%dm %05d %s \033[0m\n" $((i%6+1)) $i "$(date +%T.%N)"; sleep 0.05; done'
  for _ in 1 2; do
    swaymsg "workspace number 24" >/dev/null 2>&1
    swaymsg exec "timeout 120 alacritty -e sh -c '${SWPP_BENCH_PIN_CMD:-$ticker}'" >/dev/null 2>&1
    sleep 1.2
  done
  "$BIN" pin >/dev/null 2>&1
  sleep 0.6
  swaymsg "workspace number 1" >/dev/null 2>&1
  sleep 2
fi
# Everything before this line is startup; the summary starts after it.
start=$(wc -l < "$STATS")

i=0
while [ "$i" -lt "$ROUNDS" ]; do
  kill -USR1 "$p"; sleep 1.0
  kill -USR1 "$p"; sleep 1.0
  i=$((i + 1))
done
i=0
while [ "$i" -lt "$ROUNDS" ]; do
  notify-send -t 1200 "bench $i" "a notification to animate in and out"
  sleep 2.0
  i=$((i + 1))
done
sleep 1

tail -n +"$((start + 1))" "$STATS" | awk -v gate="$GATE" '
  function sort(a, n,   i, j, v) { for (i = 2; i <= n; i++) { v = a[i]; j = i - 1; while (j > 0 && a[j] > v) { a[j + 1] = a[j]; j-- } a[j + 1] = v } }
  {
    split($3, iv, "="); split($4, wk, "="); i = iv[2] + 0; w = wk[2] + 0
    if (i == 0 || i > 150) { f++; first[f] = w; next }
    n++; work[n] = w; if (i > 25) late++
  }
  END {
    if (n == 0) { print "no frames"; exit 1 }
    sort(work, n); sort(first, f)
    lp = 100 * late / n; p95 = work[int(n * 0.95) + 1]; fmax = first[f]
    printf "frames %d  late %d (%.1f%%)  work p50 %.2f p95 %.2f max %.2f ms  first p50 %.1f max %.1f ms (%d)\n", n, late, lp, work[int(n * 0.5) + 1], p95, work[n], first[int(f * 0.5) + 1], fmax, f
    if (gate) {
      bad = 0
      if (lp > 15) { print "gate: more than 15 % of frames late"; bad = 1 }
      if (p95 > 8) { print "gate: p95 work per frame over 8 ms"; bad = 1 }
      if (fmax > 400) { print "gate: a first frame over 400 ms"; bad = 1 }
      exit bad
    }
  }'
