#!/usr/bin/env bash
# Render the lock card under several glass settings and lay them out side by
# side, so a material can be chosen by looking rather than by rebuilding.
#
# Every field of `Tuning` (src/lock/glass_gl.rs) reads an environment variable
# named after it, so a variant is just a set of those:
#
#   ./lock-glass-sweep.sh out/ \
#       'crisp|FROST=0,REFLECT_LOD=0' \
#       'frosted|FROST=3.2,REFLECT_LOD=4'
#
# Each runs in its own nested headless sway, for the same reason the other
# harnesses here do: a session whose outputs are asleep sends no frame
# callbacks, so GTK's frame clock stalls and grim blocks. Nothing touches the
# real outputs, and `--preview lock` never takes a session lock.
set -euo pipefail

OUT=${1:?usage: lock-glass-sweep.sh <outdir> <label|K=V,K=V>...}
HERE=$(cd "$(dirname "$0")" && pwd)
BIN=${SWAYPPLET_BIN:-$HERE/../target/debug/swaypplet}
shift
# Defaults to a generated test card rather than a photo: refraction and
# reflection are both displacements, and a displacement is only visible on
# content that has structure to displace. A dark corner of a wallpaper hides
# every difference this script exists to show.
WALLPAPER=${WALLPAPER:-$HERE/glass-testcard.png}
GEOM=${GEOM:-1400x900}
# Region of the 1400x900 frame the card occupies, plus margin.
CROP=${CROP:-500x250+455+405}
ZOOM=${ZOOM:-150%}
COLS=${COLS:-3}

[ -x "$BIN" ] || { echo "no binary at $BIN; cargo build first" >&2; exit 1; }
mkdir -p "$OUT"

# ImageMagick draws nothing at all rather than complaining when it cannot
# resolve a font name, which is why earlier sheets came out unlabelled. Ask
# fontconfig for a real file and pass that.
FONT=$(fc-match -f '%{file}' sans 2>/dev/null || true)
[ -n "$FONT" ] && [ -f "$FONT" ] && FONT_ARG=(-font "$FONT") || FONT_ARG=()

tiles=()
for spec in "$@"; do
  label=${spec%%|*}
  vars=${spec#*|}
  slug=$(printf '%s' "$label" | tr -c 'A-Za-z0-9' '-')

  # Turn K=V,K=V into SWAYPPLET_GLASS_K=V ... for sway's exec line.
  envs=""
  IFS=',' read -ra kvs <<<"$vars"
  for kv in "${kvs[@]}"; do
    [ -n "$kv" ] && envs="$envs SWAYPPLET_GLASS_$kv"
  done

  conf=$(mktemp)
  cat >"$conf" <<EOF
output HEADLESS-1 mode $GEOM
default_border none
exec env SWAYPPLET_LOCK_GLASS=gl SWAYPPLET_LOCK_WALLPAPER="$WALLPAPER"$envs \
    "$BIN" --preview lock >"$OUT/$slug.log" 2>&1
exec_always --no-startup-id sh -c 'sleep 3; grim "OUTDIR/$slug.png"; swaymsg exit'
EOF
  sed -i "s|OUTDIR|$OUT|g" "$conf"

  WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_HEADLESS_OUTPUTS=1 \
    sway -c "$conf" >"$OUT/sway-$slug.log" 2>&1 &
  pid=$!
  for _ in $(seq 1 20); do
    sleep 1
    kill -0 "$pid" 2>/dev/null || break
  done
  kill "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  rm -f "$conf"

  if [ -f "$OUT/$slug.png" ]; then
    magick "$OUT/$slug.png" -crop "$CROP" +repage -resize "$ZOOM" \
      "$OUT/tile-$slug.png"
    # Labels go on at montage time via -label, which picks a font itself;
    # -annotate needs one named and silently draws nothing without it.
    tiles+=(-label "$label" "$OUT/tile-$slug.png")
  else
    echo "no frame for $label; see $OUT/sway-$slug.log" >&2
  fi
done

[ ${#tiles[@]} -gt 0 ] || { echo "nothing rendered" >&2; exit 1; }
magick montage "${tiles[@]}" -tile "${COLS}x" -geometry +6+6 \
  -background '#0d1017' -fill '#e6ecf7' -pointsize 18 "${FONT_ARG[@]}" \
  "$OUT/sheet.png"
echo "$OUT/sheet.png"
