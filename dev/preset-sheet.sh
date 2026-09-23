#!/usr/bin/env bash
# Contact sheet of the Glass tab's preset row, one notification card each.
#
# The row is judged by looking at it and by nothing else: every preset is a
# point in a shader's parameter space, and a diff of `preset.rs` says nothing
# about whether two buttons are worth pressing separately. This boots the
# nested headless sway dev/render.sh uses, replays one preset's material into
# it per pass (as the `~/.config/swaypplet/glass.json` override `apply_saved`
# reads at panel start), puts a card on screen, and lays the six side by side.
#
# The wallpaper matters and is the point: a material over a flat colour is
# invisible, and every one of these is a statement about what the desktop
# behind the card does to it.
#
# Usage:
#   dev/preset-sheet.sh [--bin PATH] [--out DIR] [--wallpaper FILE]
#
# The tunings come from the ignored test that writes them:
#   SWPP_PRESET_OUT=<dir> cargo test -- --ignored write_preset_tunings
#
# No `set -e`: this polls with `cond && break` loops, which trip the set -e +
# &&-list gotcha.
set -uo pipefail

if [ -z "${SWPP_DBUS:-}" ]; then
  exec env SWPP_DBUS=1 dbus-run-session -- "$0" "$@"
fi

BIN="${SWAYPPLET_BIN:-swaypplet}"
# The compositor to boot headless. Defaults to PATH, which on a NixOS host
# can be a wrapper build whose scenefx fails the liquid glass shader compile
# on the headless GLES2 path — pass SWAY_BIN=/…/swayfx-unwrapped/bin/sway.
SWAY_BIN="${SWAY_BIN:-sway}"
GRIM_BIN="${GRIM_BIN:-grim}"
OUT="/tmp/swaypplet-presets"
TUNINGS=""
WALLPAPER="${SWPP_WALLPAPER:-}"
RES="520x300"
while [ $# -gt 0 ]; do
  case "$1" in
    --bin) BIN="$2"; shift 2;;
    --out) OUT="$2"; shift 2;;
    --tunings) TUNINGS="$2"; shift 2;;
    --wallpaper) WALLPAPER="$2"; shift 2;;
    --res) RES="$2"; shift 2;;
    *) echo "unknown arg: $1" >&2; exit 2;;
  esac
done
[ -n "$TUNINGS" ] || { echo "--tunings DIR is required (see the header)" >&2; exit 2; }
mkdir -p "$OUT"

W="${RES%x*}"; H="${RES#*x}"

# The system config is the live one: mask_threshold and the per-class geometry
# are the contract between a card's alpha and whether the compositor sees it,
# and a preset sheet that invented them would be showing a material no session
# renders. Only `material` is what each pass replaces.
SYSTEM="${SWAYPPLET_GLASS_CONFIG:-/etc/swaypplet/glass.json}"
[ -r "$SYSTEM" ] || { echo "no system glass config at $SYSTEM" >&2; exit 1; }

shot_one() {
  local tuning="$1" name="$2"
  local work; work="$(mktemp -d /tmp/swpp-preset-XXXX)"
  mkdir -p "$work/run" "$work/config/swaypplet"
  chmod 700 "$work/run"
  cp "$tuning" "$work/config/swaypplet/glass.json"

  local cfg="$work/sway.conf" log="$work/sway.log"
  {
    printf 'output HEADLESS-1 resolution %sx%s position 0 0 scale 1\n' "$W" "$H"
    printf 'default_border none\nxwayland disable\n'
    [ -n "$WALLPAPER" ] && printf 'output HEADLESS-1 bg %s fill\n' "$WALLPAPER"
  } > "$cfg"

  XDG_RUNTIME_DIR="$work/run" \
  XDG_CONFIG_HOME="$work/config" \
  SWAYSOCK="$work/sway.sock" I3SOCK="$work/sway.sock" \
  SWAYPPLET_GLASS_CONFIG="$SYSTEM" \
    bash -c '
      WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 timeout 90 '"$SWAY_BIN"' -d --config "'"$cfg"'" >"'"$log"'" 2>&1 &
      sway_pid=$!
      trap "kill $sway_pid 2>/dev/null" EXIT
      for _ in $(seq 1 80); do swaymsg -t get_version >/dev/null 2>&1 && break; sleep 0.1; done
      wd=""
      for _ in $(seq 1 40); do
        wd="$(grep -oE "wayland display .[^'"'"']+." "'"$log"'" | head -1 | sed "s/.*.\(wayland-[0-9]*\)./\1/")"
        [ -n "$wd" ] && break; sleep 0.1
      done
      [ -n "$wd" ] || { echo "no nested display"; exit 1; }
      export WAYLAND_DISPLAY="$wd"
      timeout 60 "'"$BIN"'" >"'"$work"'/app.log" 2>&1 &
      for _ in $(seq 1 200); do [ -e "$XDG_RUNTIME_DIR/swaypplet.pid" ] && break; sleep 0.1; done
      sleep 2
      notify-send -t 60000 -a "'"$name"'" "Glass preset" "Möte nu — Åsa, Öresund. 12 GB in 4 m 12 s." || true
      sleep 2.5
      '"$GRIM_BIN"' -o HEADLESS-1 "'"$OUT"'/'"$name"'.png" || echo "grim failed for '"$name"'"
    '
  rm -rf "$work"
}

names=()
for tuning in "$TUNINGS"/*.json; do
  name="$(basename "$tuning" .json)"
  echo "── $name"
  shot_one "$tuning" "$name"
  names+=("$name")
done

# The sheet, labelled, because six cards of glass are not self-identifying.
args=()
for name in "${names[@]}"; do
  [ -f "$OUT/$name.png" ] && args+=(-label "$name" "$OUT/$name.png")
done
if [ "${#args[@]}" -gt 0 ] && command -v montage >/dev/null 2>&1; then
  montage "${args[@]}" -tile 2x -geometry +6+6 -background '#1d2021' -fill '#ebdbb2' \
    "$OUT/contact-sheet.png" && echo "wrote $OUT/contact-sheet.png"
else
  echo "wrote the individual frames to $OUT (montage not on PATH for the sheet)"
fi
