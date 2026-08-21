#!/usr/bin/env bash
# Record the live sway session as a jump fixture.
#
# The switcher this replaces could only be judged by opening it and looking,
# which is why "window sizes are weird" survived as long as it did. A fixture
# is the fix: three swaymsg calls freeze a real session into JSON that a test
# can replay with no compositor, no display and no windows of its own.
#
# Recorded rather than synthesised, because the shapes that break layout are
# the ones nobody thinks to invent - a 120-character session description, a
# workspace on the output that is not focused, a container tree nested four
# deep. Synthetic fixtures go beside these for the cases a real session does
# not happen to contain.
#
# Usage: dev/record-session.sh [--scrub] <name>
#
# --scrub replaces every window title with "<app_id>-<n>". Titles carry mail
# subjects, client names and chat fragments, and these files are committed.
set -uo pipefail

SCRUB=0
case "${1-}" in
  --scrub) SCRUB=1; shift ;;
esac
NAME="${1-}"
[ -z "$NAME" ] && { echo "usage: $0 [--scrub] <name>" >&2; exit 2; }

DIR="$(cd "$(dirname "$0")/.." && pwd)/tests/fixtures/sessions/$NAME"
mkdir -p "$DIR" || exit 1

swaymsg -t get_tree --raw       > "$DIR/tree.json"       || exit 1
swaymsg -t get_workspaces --raw > "$DIR/workspaces.json" || exit 1
swaymsg -t get_config --raw     > "$DIR/config.json"     || exit 1

if [ "$SCRUB" -eq 1 ]; then
  python3 - "$DIR" <<'PY'
import json, sys, pathlib
d = pathlib.Path(sys.argv[1])
n = [0]
def scrub(node):
    # Only leaves carry a user-visible title; a workspace's `name` is its
    # identity and every lookup in the fixture keys on it.
    if node.get("type") in ("con", "floating_con"):
        app = node.get("app_id") or (node.get("window_properties") or {}).get("class") or "win"
        if node.get("name"):
            n[0] += 1
            node["name"] = f"{app}-{n[0]}"
    for k in ("nodes", "floating_nodes"):
        for c in node.get(k, []):
            scrub(c)
tree = json.loads((d / "tree.json").read_text())
scrub(tree)
(d / "tree.json").write_text(json.dumps(tree, indent=1))
PY
fi

python3 -c "
import json, sys, pathlib
d = pathlib.Path('$DIR')
tree = json.loads((d/'tree.json').read_text())
ws   = json.loads((d/'workspaces.json').read_text())
outs = []
def walk(n):
    if n.get('type') == 'output' and n.get('name') != '__i3': outs.append(n)
    for k in ('nodes','floating_nodes'):
        for c in n.get(k,[]): walk(c)
walk(tree)
wins = [0]
def count(n):
    if n.get('type') in ('con','floating_con') and (n.get('app_id') or n.get('window_properties')):
        wins[0] += 1
    for k in ('nodes','floating_nodes'):
        for c in n.get(k,[]): count(c)
count(tree)
wins = wins[0]
(d/'meta.json').write_text(json.dumps({
    'note': '$NAME, recorded from a live session',
    'scrubbed': bool($SCRUB),
    'outputs': [{'name': o['name'],
                 'rect': o['rect'],
                 'scale': o.get('scale'),
                 'focus': o.get('focus', [])} for o in outs],
    'workspaces': len(ws),
    'windows': wins,
}, indent=1) + '\n')
print(f'{'$NAME'}: {len(ws)} workspaces, {len(outs)} output(s)')
"
echo "wrote $DIR"
