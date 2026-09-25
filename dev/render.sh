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
#   dev/render.sh --mode jump --res 1280x800                       # the Super+Tab card, live
#   SWPP_SHOTS=3 dev/render.sh --mode jump --res 1280x800          # a series, and the diff
#   SWPP_PEEK=24 dev/render.sh --mode jump --res 1280x800          # the bar's peek at workspace 24
#   SWPP_PIN=24 dev/render.sh --mode jump --res 1280x800           # workspace 24 pinned, from 1
#   SWPP_PIN=24 SWPP_PINS_OPEN=1 dev/render.sh --mode jump ...     # and the bar's pins popover
#   SWPP_OUTPUTS=2 dev/render.sh --mode keybinds --res 1280x800    # every output, side by side
#   SWPP_OUTPUTS=2 dev/render.sh --mode osd --res 1280x800         # the OSD card, on each output
#   dev/render.sh --mode screenshot --res 1200x800                 # the region selector
#   dev/render.sh --mode notifications --res 700x900                # the popup stack
#   SWPP_SELECT_RECT=120,90,540,330 dev/render.sh --mode screenshot # with a selection drawn
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
# See preset-sheet.sh: the PATH sway can be a build whose liquid glass shader
# does not compile headless; SWAY_BIN and GRIM_BIN name working ones.
SWAY_BIN="${SWAY_BIN:-sway}"
GRIM_BIN="${GRIM_BIN:-grim}"
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
  # A wallpaper, for a shot that is about one: the glass frosts what is
  # behind it, and `look.tint` derives the palette from whatever sway's `bg`
  # line names (swaypplet src/palette.rs). Without this the desktop is black,
  # which is the best case for contrast and the wrong one for colour.
  #   SWPP_WALLPAPER=~/Pictures/wallpapers/x.jpg dev/render.sh --mode panel
  [ -n "${SWPP_WALLPAPER:-}" ] && printf 'output HEADLESS-1 bg "%s" fill\n' "$SWPP_WALLPAPER"
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
  # Same for the jump list: its chord column is looked up from the config
  # sway loaded, so a nested session with no bindings renders a column of
  # dashes and the screenshot misrepresents the feature.
  if [ "$MODE" = "keybinds" ] || [ "$MODE" = "jump" ]; then
    grep '^bindsym' "${SWPP_KEYBINDS_FROM:-$HOME/.config/sway/config}" 2>/dev/null || true
  fi
} > "$CFG"

cleanup() { [ -n "${SWAY_PID:-}" ] && kill "$SWAY_PID" 2>/dev/null || true; rm -f "$CFG" "$LOG"; }
trap cleanup EXIT

export SWAYSOCK="$SOCK"
# The nested socket, not the live session's. `sway_ipc::connect` tries
# I3SOCK before SWAYSOCK (src/sway_ipc.rs), so a harness client that inherits
# the live session's I3SOCK sends its glass replays and IPC there — to the
# desktop on the user's screen, not to this one.
unset I3SOCK
# -d so the "Running compositor on wayland display 'X'" line (INFO level) is
# logged; we parse the display name from it.
# SWPP_OUTPUTS=n gives the session n outputs, each --res, side by side, and
# the shot covers all of them: for a surface that has to be on every screen.
OUTPUTS="${SWPP_OUTPUTS:-1}"
for n in $(seq 2 "$OUTPUTS"); do
  printf 'output HEADLESS-%s resolution %sx%s position %s 0 scale 1\n' "$n" "$W" "$H" "$(( (n - 1) * W ))" >> "$CFG"
done
GRIM_OUTPUT=(-o HEADLESS-1)
[ "$OUTPUTS" -gt 1 ] && GRIM_OUTPUT=()
WLR_HEADLESS_OUTPUTS="$OUTPUTS" WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 "$SWAY_BIN" -d --config "$CFG" >"$LOG" 2>&1 &
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
  jump)
    SWAYPPLET_PEEK_OPEN="${SWPP_PEEK:-}" SWAYPPLET_PINS_OPEN="${SWPP_PINS_OPEN:-}" "$BIN" >/tmp/swpp-app.log 2>&1 &
    for _ in $(seq 1 200); do
      [ -e "$RUNTIME/swaypplet.pid" ] && break; sleep 0.1
    done
    sleep 1.5
    # Somewhere to have been, with something on it. A workspace sway leaves
    # empty is destroyed on the way out, so visiting bare workspace numbers
    # leaves a recency stack one entry long — which the jump list correctly
    # refuses to draw anything for (src/jump/gesture.rs). Each stop gets a
    # window so the workspace survives being left, which is also what makes
    # the detail column show something.
    #
    # The windows keep drawing (a coloured line every 50 ms), so the card's
    # pictures of workspaces nobody is looking at have something to show
    # moving: that is the claim a SWPP_SHOTS series checks. Workspace 24 gets
    # two windows, for a split in its picture. SWPP_JUMP_APP overrides the
    # client, for a shot of what a real one looks like.
    ticker='i=0; while :; do i=$((i+1)); printf "\033[4%dm %05d %s \033[0m\n" $((i%6+1)) $i "$(date +%T.%N)"; sleep 0.05; done'
    app="${SWPP_JUMP_APP:-alacritty -e sh -c '$ticker'}"
    for ws in 5 24 24 30 1; do
      swaymsg "workspace number $ws" >/dev/null 2>&1 || true
      swaymsg exec "timeout 120 $app" >/dev/null 2>&1 || true
      sleep 1.2
    done
    sleep 0.8
    # SWPP_PEEK=<workspace> opens the bar's peek at that workspace instead.
    # The session has no pointer, so the app opens it itself on start
    # (SWAYPPLET_PEEK_OPEN, set before launch above).
    if [ -n "${SWPP_PEEK:-}" ]; then
      sleep 1.2
    # SWPP_PIN=<n> pins workspace n the way the binding does (go there, run
    # `swaypplet pin`), then comes back, so the pin shows a hidden workspace.
    elif [ -n "${SWPP_PIN:-}" ]; then
      swaymsg "workspace number $SWPP_PIN" >/dev/null 2>&1 || true
      sleep 0.4
      "$BIN" pin >>/tmp/swpp-app.log 2>&1 || true
      # SWPP_PIN_STAY=1 stays on the pinned workspace, for the shot of what
      # pinning says: the OSD card, and the pin showing itself for a moment.
      if [ -z "${SWPP_PIN_STAY:-}" ]; then
        sleep 0.6
        swaymsg "workspace number 1" >/dev/null 2>&1 || true
        sleep 1.0
      fi
      # SWPP_PIN_VISIT=1 then visits the pinned workspace and comes back,
      # which must leave it pinned.
      if [ -n "${SWPP_PIN_VISIT:-}" ]; then
        swaymsg "workspace number $SWPP_PIN" >/dev/null 2>&1 || true
        sleep 1.0
        swaymsg "workspace number 1" >/dev/null 2>&1 || true
        sleep 1.5
      fi
      # SWPP_AFTER="cmd | cmd | ..." then runs sway commands, 1.2 s apart,
      # for a scenario the flags above do not cover.
      if [ -n "${SWPP_AFTER:-}" ]; then
        IFS='|' read -ra steps <<< "$SWPP_AFTER"
        for step in "${steps[@]}"; do
          swaymsg "$step" >/dev/null 2>&1 || true
          sleep 1.2
        done
      fi
      # SWPP_PIN_FOCUS=<output> then moves focus there (with SWPP_OUTPUTS=2),
      # for the pin following the screen you work on.
      if [ -n "${SWPP_PIN_FOCUS:-}" ]; then
        swaymsg "focus output $SWPP_PIN_FOCUS" >/dev/null 2>&1 || true
        sleep 2.0
        swaymsg -t get_outputs 2>/dev/null | grep -E '"name"|"focused"|"x"|"current_workspace"' > /tmp/swpp-outputs.txt || true
      fi
    else
      "$BIN" jump >>/tmp/swpp-app.log 2>&1 || true
      sleep 0.6
    fi
    ;;
  notifications)
    "$BIN" >/tmp/swpp-app.log 2>&1 &
    for _ in $(seq 1 200); do
      [ -e "$RUNTIME/swaypplet.pid" ] && break; sleep 0.1
    done
    sleep 1.5
    # The stack's own bugs are stacking bugs, so the set has to mix body
    # lengths: a one-line card next to a three-line one is what showed the
    # slot heights being wrong. Different -a per card because MAX_PER_APP
    # caps one sender at three, and oldest first so the newest ends up on top
    # where the critical card is easiest to read.
    # -t 60000 on the ones that would otherwise expire: the server's own
    # timeout scales with body length (timeout_for), so the shortest card —
    # the one that shows a slot height being wrong — is also the first to go,
    # and the capture retries below can outlast it.
    notify-send -t 60000 -a "Chat" "Ada Lovelace" "Short one." || true
    sleep 0.4
    notify-send -t 60000 -a "Backup" "Snapshot complete" \
      "Wrote 12 GB to /mnt/backup in 4 m 12 s, verified checksums, pruned three older snapshots to stay under the quota." || true
    sleep 0.4
    notify-send -t 60000 -a "Kalender" "Möte nu — Åsa, Öresund" "Startade 10:45." || true
    sleep 0.4
    notify-send -u critical -a "Disk" "Root filesystem 96% full" \
      "Only 3.1 GB left on /. Clear the nix store or the next rebuild fails." || true
    sleep 1.2
    ;;
  screenshot)
    "$BIN" >/tmp/swpp-app.log 2>&1 &
    for _ in $(seq 1 200); do
      [ -e "$RUNTIME/swaypplet.pid" ] && break; sleep 0.1
    done
    sleep 1.5
    # Put the panel on screen first: an empty nested desktop makes a dimmed
    # frozen capture indistinguishable from a black rectangle.
    if [ -n "${SWPP_SHOW_PANEL:-}" ]; then
      p="$(cat "$RUNTIME/swaypplet.pid" 2>/dev/null || true)"
      [ -n "$p" ] && kill -USR1 "$p" 2>/dev/null || true
      sleep 1.5
    fi
    "$BIN" screenshot region >/tmp/swpp-client.log 2>&1
    echo "client exit: $?" >> /tmp/swpp-client.log
    # The client returns as soon as the running instance takes the request;
    # the capture is a Wayland round trip after that, and the non-blank test
    # below would otherwise pass on the frame before the selector maps.
    sleep 3
    ;;
  osd)
    # The caps-lock card: it reads the LED and changes nothing, where a
    # volume or brightness key would move the real machine's level (the
    # audio service talks to the user's own sound server).
    "$BIN" >/tmp/swpp-app.log 2>&1 &
    for _ in $(seq 1 200); do
      [ -e "$RUNTIME/swaypplet.pid" ] && break; sleep 0.1
    done
    sleep 1.5
    "$BIN" osd --caps-lock >>/tmp/swpp-app.log 2>&1 || true
    ;;
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
# A layer surface is never in the tree, so this wait cannot see one and runs
# to its 6 s end. For the jump card that is fatal: its watchdog commits and
# closes it after 3 s, so every shot missed it.
case "$MODE" in jump|osd) mapped=layer ;; esac
for _ in $(seq 1 60); do
  [ -n "$mapped" ] && break
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

# Content that arrives after the first paint: a Wi-Fi scan is seconds of
# nmcli, a bluetooth enumeration is a bus round trip, and the loop below stops
# at the first non-blank frame, which is the one with the empty list on it.
#   SWPP_SETTLE=8 dev/render.sh --mode preview:network
[ -n "${SWPP_SETTLE:-}" ] && sleep "$SWPP_SETTLE"

# Capture once GTK has actually painted. The headless paint can lag the map by
# a variable margin, so re-grim until the PNG is clearly non-blank (a blank
# solid-color frame compresses to a tiny file) rather than guessing one delay.
captured=""
for _ in $(seq 1 10); do
  sleep 0.5
  "$GRIM_BIN" "${GRIM_OUTPUT[@]}" "$OUT" 2>/dev/null || true
  sz=$(stat -c '%s' "$OUT" 2>/dev/null || echo 0)
  if [ "$sz" -gt 6000 ]; then captured=1; break; fi
done
# A series, for a surface that is meant to move: SWPP_SHOTS more frames,
# SWPP_SHOT_GAP seconds apart, named after OUT (-1, -2, ...), and the number
# of pixels that differ between the first and the last. Zero means nothing
# on screen changed, which for the jump card means its pictures are stills.
if [ "${SWPP_SHOTS:-0}" -gt 0 ]; then
  last="$OUT"
  for n in $(seq 1 "$SWPP_SHOTS"); do
    sleep "${SWPP_SHOT_GAP:-0.4}"
    last="${OUT%.png}-$n.png"
    "$GRIM_BIN" "${GRIM_OUTPUT[@]}" "$last" 2>/dev/null || true
  done
  changed=$(compare -metric AE "$OUT" "$last" null: 2>&1 >/dev/null || true)
  echo "changed pixels, first to last shot: $changed"
fi

[ -z "$mapped" ] && { echo "WARNING: no swaypplet surface in tree"; echo "--- app log ---"; head -30 /tmp/swpp-app.log; }
[ -z "$captured" ] && echo "WARNING: capture stayed blank after retries"
echo "wrote $OUT (${W}x${H}, mode=$MODE, display=$WD)"
