#!/usr/bin/env bash
# Frame-by-frame harness for swaypplet's glass transitions.
#
# A fade that is right at 200 ms cannot be judged by eye and cannot be caught
# by a screenshot. This boots the same nested headless sway dev/render.sh
# uses, stretches every animation with SWAYPPLET_ANIM_SCALE, drives one
# surface through a show and a hide, grabs every frame it can, and lays them
# out as a contact sheet. What you are looking for is a card that carries its
# tint the whole way and takes its frost with it, and never a frame that is a
# darkened rectangle with nothing in it.
#
# The output goes behind a high-contrast test pattern on purpose: blur over a
# flat colour is invisible, so a flat background hides the very thing being
# checked.
#
# Usage:
#   dev/filmstrip.sh [--bin PATH] [--sway PATH] [--scale N] [--surface notification|panel|launcher|osd]
#                    [--out DIR] [--res WxH]
#
# --sway lets a patched compositor be checked without restarting the session,
# which is the whole reason the nested harness exists.
#
# No `set -e`: this polls with `cond && break` loops, which trip the set -e +
# &&-list gotcha. Errors are checked explicitly instead.
set -uo pipefail

if [ -z "${SWPP_DBUS:-}" ]; then
  exec env SWPP_DBUS=1 dbus-run-session -- "$0" "$@"
fi

BIN="${SWAYPPLET_BIN:-swaypplet}"
SWAY_BIN="${SWAYPPLET_SWAY:-sway}"
SCALE="20"
SURFACE="notification"
OUT="/tmp/swaypplet-film"
RES="900x700"
CROP=""
while [ $# -gt 0 ]; do
  case "$1" in
    --bin) BIN="$2"; shift 2;; --sway) SWAY_BIN="$2"; shift 2;;
    --scale) SCALE="$2"; shift 2;; --surface) SURFACE="$2"; shift 2;;
    --out) OUT="$2"; shift 2;; --res) RES="$2"; shift 2;;
    --crop) CROP="$2"; shift 2;;
    *) echo "unknown arg: $1" >&2; exit 2;;
  esac
done
W="${RES%x*}"; H="${RES#*x}"
RUNTIME="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
CFG="$(mktemp /tmp/swpp-film-XXXX.conf)"
LOG="$(mktemp /tmp/swpp-film-XXXX.log)"
SOCK="$RUNTIME/sway-film-$$.sock"
BG="$(mktemp /tmp/swpp-film-XXXX.png)"
# Judging a fade means looking at the surface, not the desktop around it, so
# the sheet is cropped to where that surface actually sits.
if [ -z "$CROP" ]; then
  case "$SURFACE" in
    notification) CROP="384x190+$((W - 384))+0";;
    stack)        CROP="384x460+$((W - 384))+0";;
    osd)          CROP="${W}x220+0+$((H - 260))";;
    panel)        CROP="800x$((H - 40))+0+0";;
    *)            CROP="${W}x${H}+0+0";;
  esac
fi
rm -rf "$OUT"; mkdir -p "$OUT/frames"

# Blur is only legible over high-frequency detail. Colour bars are mostly flat
# fill, and a blurred flat fill looks exactly like an unblurred one, so the
# backdrop is a fine checkerboard: blur turns it to smooth grey and the frost
# becomes impossible to miss or to imagine.
ffmpeg -loglevel error -y -f lavfi \
  -i "nullsrc=size=${W}x${H},format=gray,geq=lum='if(mod(floor(X/5)+floor(Y/5),2),230,25)'" \
  -frames:v 1 "$BG" 2>/dev/null || { echo "ffmpeg could not build the backdrop"; exit 1; }

{
  printf 'output HEADLESS-1 resolution %sx%s position 0 0 scale 1\n' "$W" "$H"
  printf 'output HEADLESS-1 bg %s fill\n' "$BG"
  printf 'default_border none\nxwayland disable\n'
  # Mirror the live session's frost (users/modules/sway.nix in the nixos repo).
  printf 'blur enable\nblur_passes 1\nblur_radius 5\n'
  # sway's parser wants the block across lines: a one-liner is read as an
  # unmatched '}' and the whole rule is dropped, which renders every surface
  # here unfrosted while looking like it worked.
  for ns in swaypplet swaypplet-launcher swaypplet-osd swaypplet-notification swaypplet-polkit; do
    printf 'layer_effects "%s" {\n    blur enable\n    blur_ignore_transparent enable\n}\n' "$ns"
  done
} > "$CFG"

cleanup() {
  [ -n "${GRAB_PID:-}" ] && kill "$GRAB_PID" 2>/dev/null
  [ -n "${SWAY_PID:-}" ] && kill "$SWAY_PID" 2>/dev/null
  # Keep the compositor log next to the frames: "why is there no frost" is
  # answered there and nowhere else.
  restore_pid
  cp "$LOG" "$OUT/sway.log" 2>/dev/null
  cp "$CFG" "$OUT/sway.conf" 2>/dev/null
  rm -f "$CFG" "$LOG" "$BG"
  return 0
}
trap cleanup EXIT

export SWAYSOCK="$SOCK"
# scenefx's effects are GLES-only. A headless wlroots will happily pick the
# pixman renderer, and then the frost silently does not exist, which is the
# one thing this harness is for.
WLR_BACKENDS=headless WLR_RENDERER=gles2 WLR_LIBINPUT_NO_DEVICES=1 \
  "$SWAY_BIN" -d --config "$CFG" >"$LOG" 2>&1 &
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

# The pid file is a fixed path shared with the live session, and the nested
# instance would leave a dead pid in it for the session's mod+space to signal.
PIDFILE="$RUNTIME/swaypplet.pid"
SAVED_PID="$(cat "$PIDFILE" 2>/dev/null)"
restore_pid() { [ -n "${SAVED_PID:-}" ] && printf '%s' "$SAVED_PID" > "$PIDFILE"; return 0; }
rm -f "$PIDFILE"
# A private bus has no a11y registry, and GTK treats failing to reach it as
# fatal to GApplication registration, which leaves the app without its
# notification service. Nothing here needs the bridge.
export GTK_A11Y=none NO_AT_BRIDGE=1
SWAYPPLET_ANIM_SCALE="$SCALE" "$BIN" >"$OUT/app.log" 2>&1 &
APP_PID=$!

# Ready means the notification service is on the bus, not that a socket
# exists: XDG_RUNTIME_DIR is shared with the live session, so its sockets are
# always there and say nothing about this instance.
owned=""
for _ in $(seq 1 150); do
  if dbus-send --session --dest=org.freedesktop.DBus --print-reply=literal \
      /org/freedesktop/DBus org.freedesktop.DBus.NameHasOwner \
      string:org.freedesktop.Notifications 2>/dev/null | grep -q true; then
    owned=1; break
  fi
  kill -0 "$APP_PID" 2>/dev/null || break
  sleep 0.1
done
[ -n "$owned" ] || { echo "swaypplet never took org.freedesktop.Notifications"; head -30 "$OUT/app.log"; exit 1; }
sleep 1

# Grab as fast as grim will go. At --scale 20 a 200 ms exit is four seconds,
# so even a slow grab lands tens of frames inside the transition.
grab() {
  n=0
  while [ $n -lt 2000 ]; do
    grim -o HEADLESS-1 "$(printf '%s/frames/f%04d.png' "$OUT" "$n")" 2>/dev/null
    n=$((n+1))
  done
}
grab & GRAB_PID=$!

case "$SURFACE" in
  notification)
    # Long enough to hold still between the enter and the exit at this scale.
    notify-send -t $((300 * SCALE)) "filmstrip" "glass enter and exit"
    sleep $(awk "BEGIN{print (300*$SCALE + 1200*$SCALE/20)/1000}")
    ;;
  stack)
    # Several at once, to check the stacking and the collapsed tail rather
    # than one card's transition.
    i=0
    while [ $i -lt 5 ]; do
      notify-send -a "app$i" -t $((300 * SCALE)) "card $i" "body of card $i"
      sleep 0.4
      i=$((i + 1))
    done
    sleep $(awk "BEGIN{print (300*$SCALE + 1200*$SCALE/20)/1000}")
    ;;
  panel|launcher)
    # The pid file appears a beat after the bus name does.
    p=""
    for _ in $(seq 1 60); do
      p="$(cat "$PIDFILE" 2>/dev/null)"
      [ -n "$p" ] && break
      sleep 0.1
    done
    [ -n "$p" ] || { echo "no pid file, cannot toggle the panel"; exit 1; }
    kill -USR1 "$p" || { echo "USR1 to $p failed"; exit 1; }
    sleep $(awk "BEGIN{print 2 + 400*$SCALE/1000}")
    kill -USR1 "$p"
    sleep $(awk "BEGIN{print 2 + 400*$SCALE/1000}")
    ;;
  osd)
    "$BIN" osd --output-volume raise >/dev/null 2>&1
    sleep $(awk "BEGIN{print 4 + 400*$SCALE/1000}")
    ;;
  *) echo "unknown surface: $SURFACE" >&2; exit 2;;
esac

kill "$GRAB_PID" 2>/dev/null; GRAB_PID=""
kill "$APP_PID" 2>/dev/null

# Keep only the frames where something moved, so the sheet is the transition
# and not a hundred copies of the resting state. Byte-identical PNGs mean an
# identical frame, which is all the comparison this needs — no ImageMagick.
python3 - "$OUT" <<'PY_EOF'
import hashlib, os, shutil, sys
out = sys.argv[1]
src = os.path.join(out, "frames")
dst = os.path.join(out, "sheet")
os.makedirs(dst, exist_ok=True)
frames = sorted(f for f in os.listdir(src) if f.endswith(".png"))
kept, prev = 0, None
for f in frames:
    p = os.path.join(src, f)
    if os.path.getsize(p) == 0:
        continue
    h = hashlib.md5(open(p, "rb").read()).hexdigest()
    if h != prev:
        shutil.copy(p, os.path.join(dst, "%04d.png" % kept))
        kept += 1
        prev = h
# A 20x transition is over a hundred distinct frames, which is a contact
# sheet nobody can read. Keep every frame on disk for pulling one out, and
# thin an evenly spaced set down for the sheet itself.
pick = os.path.join(out, "pick")
os.makedirs(pick, exist_ok=True)
N = min(kept, 24)
for i in range(N):
    src_i = round(i * (kept - 1) / max(N - 1, 1))
    shutil.copy(os.path.join(dst, "%04d.png" % src_i), os.path.join(pick, "%02d.png" % i))
print("%d frames grabbed, %d distinct, %d on the sheet" % (len(frames), kept, N))
open(os.path.join(out, "count"), "w").write(str(kept))
PY_EOF

KEPT="$(cat "$OUT/count" 2>/dev/null || echo 0)"
if [ "$KEPT" -gt 0 ]; then
  # Labelled contact sheet: every distinct frame, numbered, so a frame that
  # looks wrong can be named and pulled out of $OUT/sheet/ on its own.
  # Crop to the surface before tiling; frames on disk stay full-output.
  magick mogrify -crop "$CROP" +repage "$OUT"/pick/*.png
  magick montage "$OUT"/pick/*.png \
    -label '%t' -font DejaVu-Sans -pointsize 13 \
    -tile 4x -geometry '330x+4+4' -background '#202020' -fill '#d5c4a1' \
    "$OUT/filmstrip.png"
  # Same frames as a clip, for judging the motion rather than the stills.
  ffmpeg -loglevel error -y -framerate 8 -i "$OUT/sheet/%04d.png" \
    -c:v libx264 -pix_fmt yuv420p -vf "pad=ceil(iw/2)*2:ceil(ih/2)*2" "$OUT/film.mp4"
fi
echo "wrote $OUT/filmstrip.png (${KEPT} distinct frames, ${SCALE}x, ${SURFACE}, display=$WD)"
