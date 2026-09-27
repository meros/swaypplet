#!/usr/bin/env bash
# One harness picture for an auto-fix PR: the base build (before) or the
# fixed build (after) of one surface, in one mode, into $AUTOFIX_OUT/shots.
#
#   dev/autofix/shot.sh before|after NAME MODE [--theme dark|light] [--res WxH] [KEY=VALUE ...]
#
#   NAME   a short word for the surface, [a-z0-9-]; the before and the after
#          of one surface share it, which is how run.sh pairs them
#   MODE   a dev/render.sh --mode (panel, launcher, preview:settings.look, ...)
#   KEY=VALUE  harness variables (SWPP_* and SWAYPPLET_* only), e.g.
#          SWPP_BG=white SWAYPPLET_PREVIEW_LOCK_STATE=error
#
# The file is <before|after>-NAME-<theme>.png. Each render gets its own
# XDG_RUNTIME_DIR, so it can never touch the session's pid file or sockets,
# and render.sh's own dbus-run-session and headless sway do the rest.
set -uo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
[ $# -ge 3 ] || { sed -n '2,15p' "$0" >&2; exit 2; }
WHICH="$1"; NAME="$2"; MODE="$3"; shift 3

case "$WHICH" in
  before) BIN="${AUTOFIX_BASE_BIN:?AUTOFIX_BASE_BIN is not set}";;
  after) BIN="$ROOT/target/release/swaypplet";;
  *) echo "shot.sh: first argument is before or after" >&2; exit 2;;
esac
[[ "$NAME" =~ ^[a-z0-9-]{1,40}$ ]] || { echo "shot.sh: NAME must be [a-z0-9-]" >&2; exit 2; }
[[ "$MODE" =~ ^[a-z0-9:._-]{1,60}$ ]] || { echo "shot.sh: odd MODE" >&2; exit 2; }
[ -x "$BIN" ] || { echo "shot.sh: no binary at $BIN (cargo build --release first)" >&2; exit 1; }

THEME=dark; RES=1400x1000; ENVS=()
while [ $# -gt 0 ]; do
  case "$1" in
    --theme) THEME="$2"; shift 2;;
    --res) RES="$2"; shift 2;;
    SWPP_*=*|SWAYPPLET_*=*) ENVS+=("$1"); shift;;
    *) echo "shot.sh: not allowed: $1" >&2; exit 2;;
  esac
done
case "$THEME" in dark|light) ;; *) echo "shot.sh: theme is dark or light" >&2; exit 2;; esac
[[ "$RES" =~ ^[0-9]{3,4}x[0-9]{3,4}$ ]] || { echo "shot.sh: RES is WxH" >&2; exit 2; }

OUT="${AUTOFIX_OUT:?AUTOFIX_OUT is not set}/shots"
mkdir -p "$OUT"
RT="$(mktemp -d "${TMPDIR:-/tmp}/autofix-rt-XXXXXX")"
chmod 700 "$RT"
trap 'rm -rf "$RT"' EXIT

FILE="$OUT/$WHICH-$NAME-$THEME.png"
env XDG_RUNTIME_DIR="$RT" SWPP_THEME="$THEME" "${ENVS[@]}" \
  timeout 120 "$ROOT/dev/render.sh" --bin "$BIN" --mode "$MODE" --res "$RES" --out "$FILE"
status=$?
[ -s "$FILE" ] && echo "shot.sh: wrote $FILE" || echo "shot.sh: no picture (render.sh exit $status)"
exit "$status"
