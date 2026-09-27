#!/usr/bin/env bash
# The checks an auto-fix must pass: the tests, then the frame-time gate.
# The agent runs it while it works; run.sh runs it again on the result and
# puts this script's own output in the PR, so the PR does not rest on the
# agent's word.
#
#   dev/autofix/check.sh            both
#   dev/autofix/check.sh --tests    the tests only
#
# The gate boots a nested headless sway on its own XDG_RUNTIME_DIR, never
# the session's; SWAYPPLET_SWAY (or SWAY_BIN) names the swayfx to use.
# Two tries, as .githooks/pre-push does: one noisy run is not a regression.
set -uo pipefail
cd "$(dirname "$0")/../.." || exit 1

echo "== cargo test --release"
cargo test --release --quiet 2>&1 | tail -n 15
[ "${PIPESTATUS[0]}" -eq 0 ] || { echo "RESULT tests: FAIL"; exit 1; }
echo "RESULT tests: pass"
[ "${1:-}" = "--tests" ] && exit 0

echo "== cargo build --release"
cargo build --release --quiet 2>&1 | tail -n 15
[ "${PIPESTATUS[0]}" -eq 0 ] || { echo "RESULT build: FAIL"; exit 1; }

sway_bin="${SWAYPPLET_SWAY:-${SWAY_BIN:-$(command -v sway)}}"
RT="$(mktemp -d "${TMPDIR:-/tmp}/autofix-gate-XXXXXX")"
chmod 700 "$RT"
trap 'rm -rf "$RT"' EXIT
for try in 1 2; do
  echo "== dev/frame-bench.sh --gate (try $try)"
  if XDG_RUNTIME_DIR="$RT" timeout 300 dev/frame-bench.sh --bin target/release/swaypplet --sway "$sway_bin" --gate 2>/dev/null | tail -n 6; then
    echo "RESULT gate: pass"
    exit 0
  fi
done
echo "RESULT gate: FAIL"
exit 1
