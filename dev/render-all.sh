#!/usr/bin/env bash
# Every surface dev/render.sh can draw, in both modes over both backdrops,
# into one directory; and a perceptual diff of two such directories.
#
# The gate for a change that should not move a pixel (a stylesheet split, a
# refactor of the components): capture before, capture after, compare.
#
# Usage:
#   dev/render-all.sh capture DIR [--bin PATH] [--jobs N] [--only REGEX]
#   dev/render-all.sh compare BEFORE AFTER [--out DIR] [--fuzz 2%] [--threshold N]
#   dev/render-all.sh list
#
# capture: one PNG per surface × {dark, light} × {black, wallpaper}, named
#   <surface>--<mode>--<backdrop>.png. The wallpaper is SWPP_WALLPAPER_IMG
#   (default: the first image in ~/Pictures/wallpapers). Each render gets its
#   own XDG_RUNTIME_DIR (under DIR/.rt), so jobs run in parallel (--jobs,
#   default 3) without sharing a pid file or a Wayland socket, and none of
#   them can find the live session's.
# compare: `compare -metric AE -fuzz 2%` per pair. A pair whose count of
#   differing pixels is over --threshold (default 150) is CHANGED, and gets a
#   side-by-side sheet (before | after | difference) in --out (default
#   AFTER/compare). Prints one line per pair and a summary; exits 1 when
#   anything changed or went missing.
#
# Surfaces with live content (the bar's clock, the jump card's ticking
# terminals, a notification's age) differ run to run by a few hundred
# pixels. Capture the same build twice to see the floor before reading a
# comparison, and raise --threshold or read the sheets rather than trusting
# a count near it.
#
# SWAY_BIN, GRIM_BIN and the other render.sh variables pass through.
set -uo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"

# ImageMagick 7 (`magick`), from PATH or nixpkgs.
im() {
  if command -v magick >/dev/null 2>&1; then
    magick "$@"
  else
    if [ -z "${IM_STORE:-}" ]; then
      IM_STORE="$(nix build nixpkgs#imagemagick --print-out-paths --no-link 2>/dev/null | tail -1)"
      export IM_STORE
    fi
    "$IM_STORE/bin/magick" "$@"
  fi
}

# name|mode|resolution|extra environment (space-separated K=V)
SURFACES='
panel|panel|1200x1600|
launcher|launcher|1280x800|
keybinds|keybinds|1600x1000|
jump|jump|1280x800|
osd|osd|1280x800|
notifications|notifications|700x900|
screenshot|screenshot|1200x800|
lock|preview:lock|1280x800|
lock-error|preview:lock|1280x800|SWAYPPLET_PREVIEW_LOCK_STATE=error
lock-face|preview:lock|1280x800|SWAYPPLET_PREVIEW_LOCK_STATE=face
polkit|preview:polkit|1280x800|
polkit-error|preview:polkit|1280x800|SWAYPPLET_PREVIEW_POLKIT_STATE=fp,prompt,error
tiles|preview:tiles|1200x1000|
audio|preview:audio|1200x1000|
brightness|preview:brightness|1200x1000|
network|preview:network|1200x1000|
bluetooth|preview:bluetooth|1200x1000|
display|preview:display|1200x1000|
media|preview:media|1200x1000|
notification-card|preview:notifications|1200x1000|
clipboard|preview:clipboard|1200x1000|SWPP_SEED_CLIPBOARD=1
power|preview:power|1200x1000|
selector|preview:selector|1200x1000|
annotate|preview:annotate|1400x1000|
settings-look|preview:settings.look|1400x1400|
settings-idle|preview:settings.idle|1400x1400|
settings-bar|preview:settings.bar|1400x1400|
settings-alerts|preview:settings.alerts|1400x1400|
settings-launcher|preview:settings.launcher|1400x1400|
settings-glass|preview:settings.glass|1400x1400|
'

wallpaper() {
  # The capture's own copy, scaled to the largest shot, so swaybg paints it
  # in a fraction of the time a 4k original takes.
  if [ -n "${RENDER_ALL_WALLPAPER:-}" ]; then
    printf '%s\n' "$RENDER_ALL_WALLPAPER"
    return
  fi
  if [ -n "${SWPP_WALLPAPER_IMG:-}" ]; then
    readlink -f "$SWPP_WALLPAPER_IMG"
    return
  fi
  find -L "$HOME/Pictures/wallpapers" -maxdepth 1 -type f \
    \( -name '*.jpg' -o -name '*.png' -o -name '*.jpeg' \) 2>/dev/null | sort | head -1
}

# One render: $1 = "name|mode|res|env", $2 = mode (dark|light),
# $3 = backdrop (black|wallpaper), $4 = out dir, $5 = binary.
render_one() {
  local spec="$1" theme="$2" backdrop="$3" dir="$4" bin="$5"
  local name mode res extra
  IFS='|' read -r name mode res extra <<< "$spec"
  local out="$dir/$name--$theme--$backdrop.png"
  local bg wp=""
  if [ "$backdrop" = wallpaper ]; then
    bg="$(wallpaper)"; wp="$bg"
  else
    bg=black
  fi
  local try rt log
  for try in 1 2 3; do
    rt="$(mktemp -d "$dir/.rt/XXXXXX")"
    log="$dir/.log/$name--$theme--$backdrop.log"
    # shellcheck disable=SC2086
    env XDG_RUNTIME_DIR="$rt" SWPP_THEME="$theme" SWPP_BG="$bg" SWPP_WALLPAPER="$wp" \
      $extra timeout 150 "$HERE/render.sh" --bin "$bin" --out "$out" --mode "$mode" --res "$res" \
      > "$log" 2>&1
    rm -rf "$rt" 2>/dev/null
    # swaybg can lose the race with the capture under load, which leaves
    # a black desktop in a wallpaper shot: a different picture, not a
    # different build. A wallpaper shot that came out dark is taken again.
    if [ "$backdrop" = wallpaper ] && [ -s "$out" ] &&
       [ "$(im "$out" -format '%[fx:mean > 0.12 ? 1 : 0]' info: 2>/dev/null)" != 1 ]; then
      continue
    fi
    if [ -s "$out" ] && ! grep -q 'capture stayed blank' "$log"; then
      echo "ok    $name--$theme--$backdrop"
      return 0
    fi
  done
  # render.sh calls a PNG under 6 kB blank, and a small card on black can be
  # that small and still be all there; a wallpaper shot can stay dark. Keep
  # the last try, and say so.
  if [ -s "$out" ]; then
    echo "check $name--$theme--$backdrop (blank or dark after 3 tries)"
    return 0
  fi
  echo "FAIL  $name--$theme--$backdrop (see $log)"
  return 1
}
export -f render_one wallpaper im
export HERE

capture() {
  local dir="" bin="${SWAYPPLET_BIN:-swaypplet}" jobs=3 only="."
  while [ $# -gt 0 ]; do
    case "$1" in
      --bin) bin="$2"; shift 2;; --jobs) jobs="$2"; shift 2;;
      --only) only="$2"; shift 2;;
      -*) echo "unknown arg: $1" >&2; exit 2;;
      *) dir="$1"; shift;;
    esac
  done
  [ -n "$dir" ] || { echo "capture: no directory" >&2; exit 2; }
  mkdir -p "$dir/.log" "$dir/.rt"
  # sway puts its IPC socket in XDG_RUNTIME_DIR (dir/.rt/XXXXXX here), and a
  # Unix socket path must fit in 108 bytes: a deep DIR makes every render die
  # with "Socket path won't fit into ipc_sockaddr->sun_path" and no picture.
  # The socket name itself is about 30 bytes, so the directory gets 70.
  if [ "$(printf %s "$(cd "$dir" && pwd)/.rt/XXXXXX" | wc -c)" -gt 70 ]; then
    echo "render-all: $dir is too deep for sway's socket path; use a shorter DIR (e.g. /tmp/qa)" >&2
    exit 2
  fi
  local src
  src="$(wallpaper)"
  if [ -n "$src" ]; then
    im "$src" -resize '1600x1600^' "$dir/.wallpaper.jpg"
    RENDER_ALL_WALLPAPER="$(readlink -f "$dir/.wallpaper.jpg")"
    export RENDER_ALL_WALLPAPER
  else
    echo "no wallpaper found: the wallpaper renders fall back to black" >&2
  fi
  local specs
  specs="$(printf '%s\n' "$SURFACES" | grep -v '^$' | grep -E "^($only)" )"
  for theme in dark light; do
    for backdrop in black wallpaper; do
      printf '%s\n' "$specs" | while IFS= read -r s; do
        printf '%s\t%s\t%s\n' "$s" "$theme" "$backdrop"
      done
    done
  done | xargs -P "$jobs" -d '\n' -I{} bash -c \
    'IFS=$'"'"'\t'"'"' read -r s t b <<< "$1"; render_one "$s" "$t" "$b" "$2" "$3"' _ {} "$dir" "$bin"
  echo "captured into $dir"
}

compare_dirs() {
  local a="" b="" out="" fuzz="2%" threshold=150
  while [ $# -gt 0 ]; do
    case "$1" in
      --out) out="$2"; shift 2;; --fuzz) fuzz="$2"; shift 2;;
      --threshold) threshold="$2"; shift 2;;
      -*) echo "unknown arg: $1" >&2; exit 2;;
      *) if [ -z "$a" ]; then a="$1"; else b="$1"; fi; shift;;
    esac
  done
  [ -n "$a" ] && [ -n "$b" ] || { echo "compare: BEFORE AFTER" >&2; exit 2; }
  out="${out:-$b/compare}"
  mkdir -p "$out"
  local same=0 changed=0 missing=0 f name n diff
  for f in "$a"/*.png; do
    name="$(basename "$f")"
    if [ ! -f "$b/$name" ]; then
      echo "MISSING  $name"; missing=$((missing + 1)); continue
    fi
    diff="$out/.${name%.png}-diff.png"
    n="$(im compare -metric AE -fuzz "$fuzz" "$f" "$b/$name" "$diff" 2>&1 >/dev/null | awk '{print $1}')"
    case "$n" in
      ''|*[!0-9.e+]*) n=size ;;
    esac
    if [ "$n" = size ] || [ "${n%.*}" -gt "$threshold" ] 2>/dev/null; then
      changed=$((changed + 1))
      printf 'CHANGED  %-44s %s px\n' "$name" "$n"
      if [ "$n" = size ]; then
        im "$f" "$b/$name" -background '#202020' -gravity north +append "$out/$name"
      else
        im "$f" "$b/$name" "$diff" -background '#202020' -gravity north -splice 8x0 +append "$out/$name"
      fi
    else
      same=$((same + 1))
      printf 'same     %-44s %s px\n' "$name" "$n"
    fi
    rm -f "$diff"
  done
  for f in "$b"/*.png; do
    [ -f "$a/$(basename "$f")" ] || { echo "NEW      $(basename "$f")"; }
  done
  echo "same $same, changed $changed, missing $missing (fuzz $fuzz, threshold $threshold px); sheets in $out"
  [ "$changed" -eq 0 ] && [ "$missing" -eq 0 ]
}

case "${1:-}" in
  capture) shift; capture "$@";;
  compare) shift; compare_dirs "$@";;
  list) printf '%s\n' "$SURFACES" | grep -v '^$' | cut -d'|' -f1;;
  *) sed -n '2,31p' "$0" | sed 's/^# \{0,1\}//'; exit 2;;
esac
