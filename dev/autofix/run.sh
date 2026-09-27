#!/usr/bin/env bash
# Auto-fix one issue: a worktree, one agent, one PR. Nothing starts this but
# a person: the Auto-fix button in Settings → Quality runs it as a transient
# user unit (`swaypplet-autofix-<n>`), and it is safe to run by hand.
#
#   dev/autofix/run.sh --issue N [--allow-author LOGIN] [--dry-run] [--agent] [--fixture FILE]
#
#   --issue N         the issue to fix (required)
#   --allow-author L  run on an issue filed by L, who is not the repository
#                     owner; the tab passes it only after asking (see SECURITY)
#   --dry-run         print the push, the uploads and the PR instead of doing
#                     them; stops before the agent unless --agent is given
#   --agent           with --dry-run: run the agent anyway (a real, paid run)
#   --fixture FILE    the issue as `gh issue view --json` gives it, from FILE
#                     instead of GitHub; implies --dry-run
#
# The job's files, which the tab reads while it runs, live in
# $STATE/jobs/<n>/ (STATE defaults to ~/.local/state/swaypplet-autofix):
#   pid         this script's pid
#   phase       one line: the phase it is in ("3/5 agent working")
#   result      written once, at the end: "ready <PR URL>", "failed <why>",
#               "stopped" or "dry-run"; absent while it runs
#   log         this script's output
#   check.txt   check.sh's output on the result (tests, frame gate)
#   out/        the agent's summary.md and shots/{before,after}-*.png
#   agent.jsonl the agent's stream
#
# Phases:
#   1. queued, until the lock is free: one job at a time, the rest wait
#   2. a worktree on `autofix/<n>` from origin/main, in
#      ../.swaypplet-autofix/<n> beside the clone; the base binary built
#   3. `claude -p` with prompt.md, bounded in time and money
#   4. the result checked here, not taken on the agent's word: a commit
#      exists, it touches none of the protected paths, it carries no
#      credential, and check.sh (tests, frame gate) passes
#   5. the pictures (if the agent rendered a before/after pair; it does so
#      only for a visual change) uploaded to the asset branch, the branch
#      pushed, the PR opened ("Fixes #n")
# Nothing is written to GitHub before phase 5, so a failed or stopped job
# leaves no trace there.
#
# SECURITY. The repository is public: anyone can file an issue, and the
# issue's text is the agent's input. So:
#   - an issue by someone other than the owner runs only with
#     --allow-author naming that author, which the tab passes after it has
#     shown the author and asked;
#   - the issue text goes into the prompt as a block between random
#     delimiters that the prompt calls data, and never onto a command line;
#   - the agent runs with `--restricted` (file tools confined to the
#     worktree and the job's out/, no user settings or hooks),
#     `--permission-mode dontAsk` with a short allow list (no network tool,
#     no gh, no push), no MCP servers, and without the GitHub token or the
#     SSH agent in its environment;
#   - this script, not the agent, pushes, after refusing a diff that
#     touches dev/autofix/, CI, hooks, build files or dependencies, or that
#     contains anything shaped like a credential;
#   - nothing merges without the owner pressing Merge & apply.
set -uo pipefail

# Its own process group, so Stop takes the build or the agent down with it
# (a unit has its own anyway; a dry run from the tab is a plain child).
if [ -z "${AUTOFIX_SETSID:-}" ]; then
  AUTOFIX_SETSID=1 exec setsid --wait "$0" "$@"
fi

HERE="$(cd "$(dirname "$0")" && pwd)"
CLONE="$(git -C "$HERE" rev-parse --show-toplevel)"
REPO="${AUTOFIX_REPO:-meros/swaypplet}"
OWNER="${REPO%%/*}"
STATE="${AUTOFIX_STATE:-${XDG_STATE_HOME:-$HOME/.local/state}/swaypplet-autofix}"
MINUTES="${AUTOFIX_MINUTES:-75}"
BUDGET="${AUTOFIX_BUDGET_USD:-20}"
MODEL="${AUTOFIX_MODEL:-opus}"
ASSETS="report-assets"

NUMBER=""; ALLOW=""; DRY=""; AGENT_IN_DRY=""; FIXTURE=""
while [ $# -gt 0 ]; do
  case "$1" in
    --issue) NUMBER="$2"; shift 2;;
    --allow-author) ALLOW="$2"; shift 2;;
    --dry-run) DRY=1; shift;;
    --agent) AGENT_IN_DRY=1; shift;;
    --fixture) FIXTURE="$2"; DRY=1; shift 2;;
    *) sed -n '2,16p' "$0" >&2; exit 2;;
  esac
done
[[ "$NUMBER" =~ ^[0-9]+$ ]] || { sed -n '2,16p' "$0" >&2; exit 2; }

JOB="$STATE/jobs/$NUMBER"
OUT="$JOB/out"
# Beside the clone, not in it or under STATE: ~/git/personal/.swaypplet-autofix/<n>
# never shows in the clone's `git status`.
WT="${AUTOFIX_WORKTREES:-$(dirname "$CLONE")/.swaypplet-autofix}/$NUMBER"
BRANCH="autofix/$NUMBER"
rm -rf "$JOB"
mkdir -p "$OUT/shots"
echo $$ >"$JOB/pid" # for Stop in a dry run, which is not a unit
# Everything from here on is also the tab's log tail.
exec > >(tee -a "$JOB/log") 2>&1

log() { printf '%s %s\n' "$(date +%T)" "$*"; }
phase() { printf '%s\n' "$1" >"$JOB/phase"; log "── $1"; }
DONE=""
finish() { # result line, exit code
  printf '%s\n' "$1" >"$JOB/result"
  DONE=1
  git -C "$CLONE" worktree remove --force "$WT" 2>/dev/null
  # The branch stays only when it was pushed for a PR.
  [ "${1%% *}" = ready ] || git -C "$CLONE" branch -D "$BRANCH" >/dev/null 2>&1
  exit "$2"
}
fail() { # reason [file whose tail explains it]
  log "FAILED: $1"
  [ -n "${2:-}" ] && [ -s "$2" ] && tail -n 30 "$2"
  finish "failed $1" 1
}
# Stop sends TERM: take the build or the agent down (our process group),
# say so, clean up. Long steps run through `waiting`, because bash runs a
# trap only between commands, and `wait` is one it can interrupt.
on_term() {
  trap '' TERM INT
  kill -TERM -- -$$ 2>/dev/null # tee too: straight to the file from here
  printf '%s stopped\n' "$(date +%T)" >>"$JOB/log"
  finish "stopped" 143
}
# `<&0`: a background command gets /dev/null for stdin unless it says.
waiting() { "$@" <&0 & wait $!; }
trap on_term TERM INT
trap '[ -n "$DONE" ] || printf "failed the runner exited unexpectedly\n" >"$JOB/result"' EXIT

# ── GitHub writes, with the dry run in one place ────────────────────────
gh_write() {
  if [ -n "$DRY" ]; then
    {
      printf 'DRY RUN: gh'; printf ' %q' "$@"; printf '\n'
      if [[ " $* " == *" --body-file - "* ]] || [[ " $* " == *" --input - "* ]]; then
        echo "--- stdin ---"; head -c 6000; echo; echo "--- end ---"
      fi
    } >&2
    return 0
  fi
  gh "$@"
}

# ── 1. The issue, and who wrote it ──────────────────────────────────────
phase "1/5 queued"
if [ -n "$FIXTURE" ]; then
  ISSUE="$(cat "$FIXTURE")"
else
  ISSUE="$(gh issue view "$NUMBER" --repo "$REPO" --json number,title,state,labels,author,body)" \
    || fail "could not read issue #$NUMBER"
fi
[ "$(jq -r .state <<<"$ISSUE")" = "OPEN" ] || fail "#$NUMBER is not open"
AUTHOR="$(jq -r '.author.login' <<<"$ISSUE")"
if [ "$AUTHOR" != "$OWNER" ] && [ "$AUTHOR" != "$ALLOW" ]; then
  fail "#$NUMBER was filed by $AUTHOR, not $OWNER; run it with --allow-author $AUTHOR only after reading it"
fi
TITLE="$(jq -r '.title' <<<"$ISSUE")"
BODY="$(jq -r '.body // ""' <<<"$ISSUE")"
KIND=issue
jq -e '.labels | any(.name == "crash")' <<<"$ISSUE" >/dev/null && KIND=crash
jq -e '.labels | any(.name == "report")' <<<"$ISSUE" >/dev/null && KIND=report
log "#$NUMBER ($KIND, by $AUTHOR): $TITLE"

mkdir -p "$STATE"
exec 9>"$STATE/lock"
flock 9 # one job at a time; a second one waits here, "queued"

# ── 2. Worktree and base build ──────────────────────────────────────────
phase "2/5 building the base"
git -C "$CLONE" fetch --quiet origin main || fail "git fetch failed"
git -C "$CLONE" worktree remove --force "$WT" 2>/dev/null
git -C "$CLONE" branch -D "$BRANCH" >/dev/null 2>&1
mkdir -p "$(dirname "$WT")"
git -C "$CLONE" worktree add --quiet -b "$BRANCH" "$WT" origin/main || fail "could not make the worktree"
BASE="$(git -C "$WT" rev-parse HEAD)"
in_dev() { waiting env -C "$WT" nix develop --quiet -c "$@"; }
in_dev cargo build --release --quiet >"$JOB/base-build.log" 2>&1 || fail "the base does not build" "$JOB/base-build.log"
cp "$WT/target/release/swaypplet" "$JOB/base-swaypplet"

# ── 3. The agent ────────────────────────────────────────────────────────
DELIM="=====$(head -c 12 /dev/urandom | od -An -tx1 | tr -d ' \n')====="
PROMPT="$(cat "$HERE/prompt.md")"
PROMPT="${PROMPT//\{\{NUMBER\}\}/$NUMBER}"
PROMPT="${PROMPT//\{\{KIND\}\}/$KIND}"
PROMPT="${PROMPT//\{\{AUTHOR\}\}/$AUTHOR}"
PROMPT="${PROMPT//\{\{DELIM\}\}/$DELIM}"
PROMPT="${PROMPT//\{\{BASE_BIN\}\}/$JOB/base-swaypplet}"
PROMPT="${PROMPT//\{\{OUT\}\}/$OUT}"
PROMPT="${PROMPT//\{\{MINUTES\}\}/$MINUTES}"
# Title and body last, so text in them is never taken for a placeholder,
# and with any copy of the delimiter taken out.
PROMPT="${PROMPT//\{\{TITLE\}\}/${TITLE//$DELIM/}}"
PROMPT="${PROMPT//\{\{BODY\}\}/${BODY//$DELIM/}}"
printf '%s\n' "$PROMPT" >"$JOB/prompt.md"

ALLOWED=(
  Read Edit Write Glob Grep
  "Bash(cargo build:*)" "Bash(cargo test:*)" "Bash(cargo check:*)"
  "Bash(cargo fmt:*)" "Bash(cargo clippy:*)"
  "Bash(dev/autofix/shot.sh:*)" "Bash(dev/autofix/check.sh:*)"
  "Bash(git status:*)" "Bash(git diff:*)" "Bash(git log:*)" "Bash(git show:*)"
  "Bash(git add:*)" "Bash(git commit:*)"
)
CLAUDE=(
  claude -p
  --model "$MODEL"
  --restricted
  --tools "Bash,Read,Edit,Write,Glob,Grep"
  --permission-mode dontAsk
  --allowedTools "${ALLOWED[@]}"
  --add-dir "$OUT"
  --strict-mcp-config --mcp-config '{"mcpServers":{}}'
  --no-session-persistence
  --max-budget-usd "$BUDGET"
  --output-format stream-json --verbose
)

if [ -n "$DRY" ] && [ -z "$AGENT_IN_DRY" ]; then
  echo "DRY RUN: would run in $WT, under nix develop, for at most $MINUTES min:"
  printf ' %q' "${CLAUDE[@]}"; printf ' < %s\n' "$JOB/prompt.md"
  echo "--- prompt ($JOB/prompt.md) ---"
  cat "$JOB/prompt.md"
  echo "--- end ---"
  echo "DRY RUN: then check.sh, the diff guard, uploads, git push origin $BRANCH, gh pr create"
  finish "dry-run" 0
fi

phase "3/5 agent working"
log "at most $MINUTES min and \$$BUDGET"
waiting env -C "$WT" -u GH_TOKEN -u GITHUB_TOKEN -u GH_ENTERPRISE_TOKEN -u SSH_AUTH_SOCK \
  AUTOFIX_OUT="$OUT" AUTOFIX_BASE_BIN="$JOB/base-swaypplet" \
  nix develop --quiet -c timeout --kill-after=60 "$((MINUTES * 60))" \
  "${CLAUDE[@]}" <"$JOB/prompt.md" >"$JOB/agent.jsonl" 2>"$JOB/agent.err"
AGENT_EXIT=$?
jq -r 'select(.type == "result") | .result // empty' "$JOB/agent.jsonl" >"$JOB/agent-result.txt" 2>/dev/null
[ "$AGENT_EXIT" -eq 124 ] && fail "the agent ran out of its $MINUTES minutes" "$JOB/agent-result.txt"

# ── 4. Check the result here ────────────────────────────────────────────
phase "4/5 checking the result"
COMMITS="$(git -C "$WT" rev-list --count "$BASE..HEAD")"
if [ "$COMMITS" -eq 0 ]; then
  cat "$OUT/summary.md" "$JOB/agent-result.txt" 2>/dev/null >"$JOB/why.txt"
  fail "the agent made no commit" "$JOB/why.txt"
fi
[ -z "$(git -C "$WT" status --porcelain --untracked-files=no)" ] || git -C "$WT" stash --quiet
PROTECTED='^(dev/autofix/|\.github/|\.githooks/|build\.rs$|flake\.nix$|flake\.lock$|package\.nix$|Cargo\.toml$|Cargo\.lock$|\.gitignore$|\.gitattributes$)'
touched="$(git -C "$WT" diff --name-only "$BASE..HEAD" | grep -E "$PROTECTED" || true)"
[ -z "$touched" ] || { printf '%s\n' "$touched" >"$JOB/protected.txt"; fail "the change touches protected paths" "$JOB/protected.txt"; }
SECRET='gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{20,}|sk-ant-[A-Za-z0-9_-]{20,}|-----BEGIN [A-Z ]*PRIVATE KEY-----|AKIA[0-9A-Z]{16}'
git -C "$WT" diff "$BASE..HEAD" | grep -qE "$SECRET" && fail "the change contains something shaped like a credential; nothing was pushed"
if command -v gitleaks >/dev/null; then
  gitleaks git --no-banner --log-opts="$BASE..HEAD" "$WT" >"$JOB/gitleaks.txt" 2>&1 \
    || fail "gitleaks found a secret in the change; nothing was pushed" "$JOB/gitleaks.txt"
fi
in_dev dev/autofix/check.sh >"$JOB/check.txt" 2>&1 || fail "check.sh failed on the agent's result" "$JOB/check.txt"

# ── 5. Pictures, push, PR ───────────────────────────────────────────────
phase "5/5 opening the PR"
ensure_assets_branch() {
  gh api "repos/$REPO/branches/$ASSETS" >/dev/null 2>&1 && return 0
  [ -n "$DRY" ] && { echo "DRY RUN: would create the orphan branch $ASSETS"; return 0; }
  local tree commit
  tree="$(jq -n '{tree: [{path: "README.md", mode: "100644", type: "blob", content: "Pictures for swaypplet reports and auto-fix PRs.\n"}]}' |
    gh api --method POST "repos/$REPO/git/trees" --input - --jq .sha)" || return 1
  commit="$(jq -n --arg t "$tree" '{message: "report assets", tree: $t, parents: []}' |
    gh api --method POST "repos/$REPO/git/commits" --input - --jq .sha)" || return 1
  jq -n --arg s "$commit" --arg r "refs/heads/$ASSETS" '{ref: $r, sha: $s}' |
    gh api --method POST "repos/$REPO/git/refs" --input - >/dev/null
}
upload() { # local file, repo path → raw URL
  if [ -n "$DRY" ]; then
    echo "DRY RUN: would upload $1 to $ASSETS:$2" >&2
    echo "https://raw.githubusercontent.com/$REPO/DRY-RUN/$2"
    return 0
  fi
  local sha
  sha="$(jq -n --arg m "add $2" --rawfile c <(base64 -w0 "$1") --arg b "$ASSETS" '{message: $m, content: $c, branch: $b}' |
    gh api --method PUT "repos/$REPO/contents/$2" --input - --jq .commit.sha)" || return 1
  echo "https://raw.githubusercontent.com/$REPO/$sha/$2"
}

PAIRS=""; MARK=""
stamp="$(date -u +%Y%m%d-%H%M%S)"
for before in "$OUT"/shots/before-*.png; do
  [ -e "$before" ] || continue
  rest="${before##*/before-}"
  after="$OUT/shots/after-$rest"
  [ -e "$after" ] || continue
  [ -n "$MARK" ] || ensure_assets_branch || fail "could not create the $ASSETS branch"
  b_path="reports/$NUMBER-$stamp-before-$rest"
  a_path="reports/$NUMBER-$stamp-after-$rest"
  b_url="$(upload "$before" "$b_path")" || fail "could not upload $before"
  a_url="$(upload "$after" "$a_path")" || fail "could not upload $after"
  PAIRS+="| \`${rest%.png}\` | ![before]($b_url) | ![after]($a_url) |"$'\n'
  MARK+=" before=$b_path after=$a_path"
done

SUBJECT="$(git -C "$WT" log -1 --format=%s)"
SUMMARY="$(cat "$OUT/summary.md" 2>/dev/null || echo "_The agent left no summary._")"
{
  printf '**Fixes #%s.** %s\n\n%s\n\n' "$NUMBER" "$SUBJECT" "$SUMMARY"
  if [ -n "$PAIRS" ]; then
    printf '| surface | before (%s) | after |\n|---|---|---|\n%s\n' "${BASE:0:12}" "$PAIRS"
  else
    printf '_No before/after pictures: the agent judged the change not visual._\n\n'
  fi
  printf '### Checks, run by the runner on this branch\n\n```text\n%s\n```\n\n' "$(grep -E '^RESULT' "$JOB/check.txt")"
  printf 'Prepared by `dev/autofix/run.sh` (claude -p, model %s) on `%s`. Review before Merge & apply.\n' "$MODEL" "$(hostname)"
  [ -n "$MARK" ] && printf '\n<!-- autofix-assets%s -->\n' "$MARK"
} >"$JOB/pr.md"

if [ -n "$DRY" ]; then
  echo "DRY RUN: git -C $WT push --force-with-lease origin $BRANCH"
  gh_write pr create --repo "$REPO" --base main --head "$BRANCH" --title "$SUBJECT" --body-file - <"$JOB/pr.md"
  finish "dry-run" 0
fi
git -C "$WT" push --quiet --force-with-lease origin "$BRANCH" || fail "git push failed"
PR_URL="$(gh pr create --repo "$REPO" --base main --head "$BRANCH" --title "$SUBJECT" --body-file - <"$JOB/pr.md" | tail -n 1)" \
  || fail "gh pr create failed"
log "ready: $PR_URL"
finish "ready $PR_URL" 0
