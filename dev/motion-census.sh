#!/usr/bin/env bash
# Every duration and easing in the stylesheet, with counts.
#
# GTK4 CSS has no custom properties, so the motion scale (docs/MOTION.md)
# cannot be enforced by the language — a stray 370ms is valid CSS and silently
# joins the vocabulary. This prints what is actually in there, so a drift shows
# up as a line that should not exist.
#
# What is expected:
#   durations  150 200 300 400 500 ms, plus delays (50, 100) and the ambient
#              loops, which are written in seconds and are deliberately not on
#              the ladder — a pulse's period answers to a heartbeat, not to a
#              transition scale.
#   easings    the three cubic-beziers of the standard set; `ease` as the
#              sanctioned shorthand at the 150ms micro tier, where it is
#              indistinguishable from the standard curve; ease-in-out and
#              linear for loops and spinners; one bespoke shake curve.
set -uo pipefail
CSS="${1:-$(dirname "$0")/../data/style.css}"

echo "── durations ──"
grep -oE '[0-9]+(\.[0-9]+)?m?s' "$CSS" | sort | uniq -c | sort -rn

echo
echo "── easings ──"
grep -oE 'cubic-bezier\([^)]*\)|ease-in-out|ease-out|ease-in|linear|[^-a-z]ease[,;]' "$CSS" \
  | sed 's/^[^a-z(]//' | sort | uniq -c | sort -rn

echo
echo "── off-ladder durations (should print nothing) ──"
# The first time in a transition/animation is the duration; a second is a
# delay and is allowed to be a stagger value.
off=$(grep -oE '(transition|animation):[^;]*' "$CSS" \
  | grep -oE '(^|[ ,])[0-9]+ms' | tr -d ' ,' | sort -u \
  | grep -vE '^(150|200|300|400|500|50|100)ms$' || true)
[ -z "$off" ] && echo "  (none)" || { echo "$off"; exit 1; }
