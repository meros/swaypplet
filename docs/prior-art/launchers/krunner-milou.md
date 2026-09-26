---
name: KRunner and Milou
slug: krunner-milou
domain: launchers
kind: product
platform: KDE
vendor: KDE
years: 2008–present
status: current
tags: [runners, threaded match, relevance, dbus runners, informational match]
relevance: high
---

## What it is
KDE's run-command bar (Alt+Space / Alt+F2) since KDE 4 (2008). Results come from "runners": C++ plugins or, since Plasma 5, separate processes over D-Bus (`org.kde.krunner1`: Match, Actions, Run). Milou is the Plasma widget that shows the same runners inside Kickoff and other menus.

## What was new
- **Threaded matching**: each runner's `match()` runs off the UI thread; results stream in and are merged.
- **Match types plus relevance**: ExactMatch, PossibleMatch, InformationalMatch, each with a 0.0–1.0 relevance. An InformationalMatch (e.g. a calculator result) is copied to the clipboard and put back into the query on selection.
- **RunnerSyntax**: each runner advertises example queries (`:q:` placeholders), which the UI can show as help.
- D-Bus runners let any app add results without linking into Plasma.

## What went wrong / limits
- Relevance values are self-reported by each runner, so one over-eager runner can dominate; KDE has had to tune per-runner weights and add history-based boosts.
- In-process C++ runners can still crash or stall the shell.

## Lessons for swaypplet
- Take InformationalMatch semantics for the `=` row: Enter copies, and also puts the result in the entry so it can be chained (`42*2`).
- Take advertised syntax: each source shows an example in the empty or `?` state, which teaches `=`, `>`, `:`.
- Avoid trusting provider-reported scores across providers; normalise per provider before merging.

## Sources
- https://develop.kde.org/docs/plasma/krunner/ — AbstractRunner, match threads, match types, RunnerSyntax [verified]
- https://invent.kde.org/frameworks/krunner — D-Bus runner interface [memory]
