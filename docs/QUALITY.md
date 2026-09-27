# Quality: reports, crashes and auto-fixes

**A key files a report, a failed service files a crash, and the Quality
tab lists every open issue with an Auto-fix button that has an agent on
this machine prepare a PR, and Merge & apply for the result.** Nothing
runs until someone presses the button. Everything goes through `gh` to
the public repository `meros/swaypplet`; `SWAYPPLET_DRY_RUN=1` (and
`dev/autofix/run.sh --dry-run`) prints every write instead of sending it.

```
swaypplet report ─→ card ─→ issue [report]
OnFailure= ─→ swaypplet crash-report ─→ issue [crash], or "seen again"
Settings → Quality: every open issue
    Auto-fix ─→ systemd-run swaypplet-autofix-<n>: dev/autofix/run.sh --issue n
               queued → base build → claude -p → checks → PR "Fixes #n"
    Stop ─→ systemctl --user stop swaypplet-autofix-<n>
    Merge & apply ─→ gh pr merge --squash ─→ nx      Close ─→ gh pr close
```

| Part | Where |
|---|---|
| The report card and its upload | `src/quality/report.rs` |
| Issue text, markers, fences | `src/quality/body.rs` |
| `gh`, the dry run, the picture upload | `src/quality/gh.rs` |
| Panic hook, crash signature, de-duplication, `crash-report` | `src/quality/crash.rs` |
| A fix's state from the local job and the PR; who must be asked about | `src/quality/status.rs` |
| The log tail a report attaches | `src/quality/logring.rs` |
| The tab, starting and stopping jobs, Merge & apply | `src/settings/quality_pane.rs` |
| The runner, its prompt, its checks, its pictures | `dev/autofix/` |

## Reports

`swaypplet report [region|window|screen|none]` asks the running panel for a
capture through the screenshot flow (Escape there means no picture), then
opens a card on `Namespace::Report`: a description, "Attach screenshot"
(on when there is one), "Attach recent log" (the last 80 lines this process
logged at info and above, home directory redacted), and a line in the
warning tone that says all of it is public. Send hides the card; a worker
encodes the PNG, uploads it, files the issue, and a notification carries
the link.

The issue body has the description, the picture, the revision
(`SWAYPPLET_REV`, from the flake; a cargo build says so), the mode and every output with its mode
and scale, and the log in a fence that no log line can close.

### Why pictures go to an orphan branch

The web UI's attachment upload has no public API, a gist mangles binaries,
and a release asset needs a release and serves a redirect. The contents
API is documented and takes the credential `gh` holds; a raw URL of a
public repository renders in an issue. The files go to the orphan branch
`report-assets` (created on first use, never merged, never in a clone of
main), and the URL names the commit, so a picture never changes under an
issue. The cost: the pictures are public for good, and deleting one needs a
force-push of that branch.

## Crashes

- **The panic hook** (every process) writes the message, the location and
  a forced backtrace to `$XDG_STATE_HOME/swaypplet/panics/<unix>-<pid>.txt`
  before the abort, keeping ten.
- **`swaypplet crash-report [--unit U] [--dry-run]`** reads the newest
  panic note of the last ten minutes, else `coredumpctl info` for the
  newest `swaypplet` dump (waiting up to 30 s for systemd-coredump), else
  systemd's "Main process exited" line, and the unit's last 80 journal
  lines.
- **The signature** is the panic's file and line plus its message with the
  numbers taken out, or the signal plus the first three swaypplet frames
  of the crashing thread by name (demangled, hash dropped). Its FNV-1a hash
  goes into the body as `<!-- crash-signature: … -->`. An open `crash`
  issue by the owner with that marker gets "Seen again at …, revision …"
  instead of a new issue.
- **Symbols.** Release builds keep their symbol table (stdenv strips
  debug sections only, and Cargo's release default does the same), so a
  frame has its function's name; a panic's file and line come from the
  panic itself. The binary gets a build ID (mold writes none by default),
  so systemd-coredump can tie a core to it. The installed binary is 27.7 MB
  (27.0 MB before this work). Line tables were measured and left out: 27.0 MB grows to 70.1 MB with swaypplet's own, 121.4 MB with
  every crate's.
- **Core dumps on this machine are truncated** (`coredumpctl list` says
  so for every swaypplet dump of 2026-09-27), so systemd-coredump unwinds
  one frame and names none. Such a dump is skipped for the signature and
  the journal's exit line is used. Raising `ProcessSizeMax` and
  `ExternalSizeMax` in coredump.conf is what makes a segfault's stack
  readable.

## Auto-fix

The button starts `dev/autofix/run.sh --issue <n>` (from
`~/git/personal/swaypplet`, or `SWAYPPLET_AUTOFIX_RUNNER`) as the transient
user unit `swaypplet-autofix-<n>`, so it survives the panel restarting.
Jobs queue on the runner's lock, one at a time. The runner keeps its state
in `~/.local/state/swaypplet-autofix/jobs/<n>/` (`phase`, `result`, `log`,
`check.txt`, `out/shots/`), which the tab reads every two seconds while it
is open: the phase, the last two log lines, Stop. The header of `run.sh`
lists the phases. Nothing is written to GitHub before the last one, so a
failed or stopped job leaves no trace there. Tunables: `AUTOFIX_STATE`,
`AUTOFIX_MINUTES` (75), `AUTOFIX_BUDGET_USD` (20), `AUTOFIX_MODEL` (opus),
`SWAYPPLET_SWAY` for the harness.

The agent renders a before/after pair (`dev/autofix/shot.sh`) only for a
visual change; the PR carries the pairs, uploaded to `report-assets`, and
the runner's own check.sh result.

### The agent's flags, and why

| Flag | Why |
|---|---|
| `-p`, prompt on stdin | Headless; the issue text never touches a command line. |
| `--restricted` | Ignores the user's settings, hooks and plugins, and confines the file tools to the worktree and `--add-dir`. |
| `--tools Bash,Read,Edit,Write,Glob,Grep` | The only tools that exist for it: no WebFetch, no WebSearch, no agents. |
| `--permission-mode dontAsk` + `--allowedTools` | Nothing prompts (nobody is there); anything not on the list is refused, not asked. The list is cargo, the two harness scripts, read-only git, `git add` and `git commit`. |
| `--add-dir $JOB/out` | Where it writes the summary and reads its pictures. |
| `--strict-mcp-config --mcp-config '{"mcpServers":{}}'` | No MCP server, so no tool beyond the list. |
| `--no-session-persistence` | No transcript of an untrusted prompt left in `~/.claude`. |
| `--max-budget-usd`, `timeout` | Bounded in money and in time. |
| `--output-format stream-json --verbose` | A full log per job in `$JOB/agent.jsonl`. |
| env without `GH_TOKEN`, `GITHUB_TOKEN`, `SSH_AUTH_SOCK` | It cannot push or call GitHub even through a test it writes. |

### Security

The repository is public: anyone can open an issue, and its text is the
agent's input. On an issue the owner did not file, the tab says so and
Auto-fix asks first ("Run on @author's text?", a second press within six
seconds); the runner refuses such an issue unless `--allow-author` names
that author. The issue goes into the prompt between two lines carrying a
random delimiter, and the prompt says it is data. The runner, not the
agent, pushes, after it refuses a diff that touches `dev/autofix/`,
`.github/`, `.githooks/`, build files or dependencies, or that contains
anything shaped like a credential (and gitleaks, when it is installed).
Nothing merges without Merge & apply.

Residual risk: the agent runs `cargo test`, which runs code it wrote, as
your user. That code can read what your user can read, although it cannot
reach the network through a tool, and the diff guard stands between it and
a push. A bubblewrap or `systemd-run -p InaccessiblePaths=` sandbox around
the agent step is the next step.

## The tab

Settings → Quality lists the open issues (30, newest first) with a fix
status (open, queued, fixing, fix ready, failed, stopped, merged, fix
closed), the author, the labels, the age and the PR. It asks GitHub only
while it is on screen: once when it maps, then every 60 s.

For a ready fix it shows the first before/after pair (from the job's
folder, or downloaded once into `$XDG_CACHE_HOME/swaypplet/quality/`),
the checks line, View PR, Close and Merge & apply. Close and Merge & apply
arm on the first press and act on a second. Merge & apply refuses, and
says which files, while the nixos checkout nx commits from (`NX_FLAKE`,
default `~/nixos`) has uncommitted changes: nx would commit them along
with the merge. Otherwise it runs
`gh pr merge --squash --delete-branch`, then the deploy as the transient
unit `swaypplet-deploy` with its output in
`$XDG_STATE_HOME/swaypplet/deploy.log`: a unit and not a child, because nx
switches home-manager, which restarts the panel; a reopened tab reads the
log and shows how it ended. `SWAYPPLET_DEPLOY` replaces `nx`.

There is no "Try locally": `nx dev swaypplet` rebuilds the system with
sudo, which asks for a password nothing in the panel can answer.

## Trying it without GitHub

```sh
SWAYPPLET_DRY_RUN=1 swaypplet crash-report
dev/autofix/run.sh --issue 42 --fixture issue.json     # to the prompt, printed
SWAYPPLET_QUALITY_FIXTURE=dir dev/render.sh --mode preview:settings.quality
SWPP_REPORT_TEXT='…' dev/render.sh --mode report       # the card, over a capture
```
