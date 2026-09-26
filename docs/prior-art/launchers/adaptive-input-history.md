---
name: Adaptive input history (query-to-choice learning)
slug: adaptive-input-history
domain: launchers
kind: pattern
platform: cross
vendor: Quicksilver, LaunchBar, Firefox, Chrome, Alfred, Spotlight
years: 2003–present
status: current
tags: [ranking, learning, abbreviations, mnemonics, moz_inputhistory, prefix]
relevance: high
---

## What it is
Remember which item the user picked for which typed string, and rank that item first when the same string (or a longer one starting with it) is typed again. Quicksilver called these mnemonics, LaunchBar abbreviation learning, Firefox keeps them in `moz_inputhistory`, Chrome in its shortcuts provider, and Spotlight's 2025 quick keys are the same idea.

## What was new
- Personal abbreviations without configuration: "ff" means Firefox because you chose it after "ff" twice.
- Firefox's adaptive autofill: a record matches if the typed text starts with the stored text; records under 4 characters are not autofilled until 4 are typed; the highest use count wins, above a threshold (default 1.0).

## What went wrong / limits
- Stale mappings persist after the item is gone (Firefox bug 1773025: a forgotten site still autofilled via input history).
- Needs decay like frecency, or early mistakes stick.

## Lessons for swaypplet
- **The single biggest ranking gap in swaypplet**: `frecency.rs` scores items, not (query, item) pairs. Add a small map `prefix → item → decayed count`, updated on launch with the query as typed, consulted before the per-item boost. S–M.
- Clear entries whose item no longer exists when elephant stops returning it.

## Sources
- https://docs.google.com/document/d/e/2PACX-1vRBLr_2dxus-aYhZRUkW9Q3B1K0uC-a0qQyE3kQDTU3pcNpDHb36-Pfo9fbETk89e7Jz4nkrqwRhi4j/pub — Firefox adaptive history autofill design [verified via search summary]
- https://bugzilla.mozilla.org/show_bug.cgi?id=395739 — awesomebar adaptive learning [verified via search summary]
- https://bugzilla.mozilla.org/show_bug.cgi?id=1773025 — stale records [verified via search summary]
