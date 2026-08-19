#!/usr/bin/env bash
# Render each glass variant across several backgrounds, one row per variant.
#
# A card that looks good on one wallpaper tells you almost nothing: the hard
# cases are a bright backdrop (where a fixed dark tint is the only thing you
# can read) and fine text (where detail behind the glass is what makes text on
# it unreadable). So every variant is judged on all of them at once, which is
# what the row layout is for.
#
#   ./glass-zoo.sh <outdir> <bgdir> 'label|KEY=V,KEY=V' ...
#
# Keys are the `Tuning` fields in src/lock/glass_gl.rs, passed as
# SWAYPPLET_GLASS_*. Runs `--preview lock`, which never takes a session lock,
# inside a nested headless sway — see dev/README-glass.md for why nested.
set -euo pipefail

OUT=${1:?usage: glass-zoo.sh <outdir> <bgdir> <label|K=V,...>...}
BGDIR=${2:?need a directory of background images}
shift 2
HERE=$(cd "$(dirname "$0")" && pwd)
BIN=${SWAYPPLET_BIN:-$HERE/../target/debug/swaypplet}
GEOM=${GEOM:-1400x900}
CROP=${CROP:-520x260+440+400}
ZOOM=${ZOOM:-100%}

[ -x "$BIN" ] || { echo "no binary at $BIN; cargo build first" >&2; exit 1; }
mkdir -p "$OUT"
FONT=$(fc-match -f '%{file}' sans 2>/dev/null || true)
[ -n "$FONT" ] && [ -f "$FONT" ] && FONT_ARG=(-font "$FONT") || FONT_ARG=()

mapfile -t BGS < <(ls "$BGDIR"/*.png | sort)

shoot() { # shoot <outfile> <bg> <envs>
  local out=$1 bg=$2 envs=$3 conf
  conf=$(mktemp)
  cat >"$conf" <<EOF
output HEADLESS-1 mode $GEOM
default_border none
exec env SWAYPPLET_LOCK_GLASS=gl SWAYPPLET_LOCK_WALLPAPER="$bg"$envs \
    "$BIN" --preview lock >/dev/null 2>&1
exec_always --no-startup-id sh -c 'sleep 3; grim "$out"; swaymsg exit'
EOF
  WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_HEADLESS_OUTPUTS=1 \
    sway -c "$conf" >/dev/null 2>&1 &
  local pid=$!
  for _ in $(seq 1 20); do sleep 1; kill -0 "$pid" 2>/dev/null || break; done
  kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true
  rm -f "$conf"
}

rows=()
for spec in "$@"; do
  label=${spec%%|*}
  vars=${spec#*|}
  slug=$(printf '%s' "$label" | tr -c 'A-Za-z0-9' '-')

  envs=""
  IFS=',' read -ra kvs <<<"$vars"
  for kv in "${kvs[@]}"; do
    [ -n "$kv" ] && envs="$envs SWAYPPLET_GLASS_$kv"
  done

  tiles=()
  for bg in "${BGS[@]}"; do
    name=$(basename "$bg" .png)
    shot="$OUT/$slug-$name.png"
    shoot "$shot" "$bg" "$envs"
    if [ -f "$shot" ]; then
      magick "$shot" -crop "$CROP" +repage -resize "$ZOOM" "$OUT/t-$slug-$name.png"
      tiles+=("$OUT/t-$slug-$name.png")
    fi
  done
  [ ${#tiles[@]} -gt 0 ] || { echo "nothing rendered for $label" >&2; continue; }
  magick montage "${tiles[@]}" -tile "${#tiles[@]}x1" -geometry +3+3 \
    -background '#0d1017' "$OUT/row-$slug.png"
  magick "$OUT/row-$slug.png" -background '#0d1017' -gravity west \
    -splice 210x0 -fill '#e6ecf7' -pointsize 21 "${FONT_ARG[@]}" \
    -annotate +14+0 "$label" "$OUT/labelled-$slug.png"
  rows+=("$OUT/labelled-$slug.png")
done

[ ${#rows[@]} -gt 0 ] || { echo "nothing rendered" >&2; exit 1; }
magick "${rows[@]}" -append -background '#0d1017' "$OUT/zoo.png"
echo "$OUT/zoo.png"
