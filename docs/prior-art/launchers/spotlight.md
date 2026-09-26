---
name: Spotlight (macOS)
slug: spotlight
domain: launchers
kind: product
platform: macOS
vendor: Apple
years: 2005–present
status: current
tags: [system search, metadata index, top hit, actions, quick keys, clipboard history]
relevance: high
---

## What it is
The system-wide search of macOS, opened with Cmd+Space since Mac OS X 10.4 Tiger (2005). It queries a metadata index fed by per-format importers, and since 10.10 Yosemite (2014) floats as a centred bar with a preview pane and web "Suggestions". macOS 26 Tahoe (2025) turned it into a command surface.

## What was new
- **Top Hit**: one row, chosen from every category, sits above the grouped results and is what Enter opens.
- **Tahoe actions**: hundreds of system and App Intents actions (send a message, create an event, run a Shortcut) run from the bar, with parameters filled inline.
- **Quick keys**: Spotlight assigns short letter codes to actions you use, so "sm" becomes send-message.
- **Scoped sections** behind Cmd+1..4 (Applications, Files, Actions, Clipboard). Clipboard history keeps 8 hours by default; 26.1 added a retention choice and a Clear button.

## What went wrong / limits
- Spotlight Suggestions (2014) sent queries and location to Apple and Bing by default, which drew privacy criticism and a toggle.
- No third-party result providers beyond importers and App Intents; power users stayed on Alfred and Raycast for a decade.
- Index corruption and `mds` CPU spikes are a recurring support topic.

## Lessons for swaypplet
- Take the Top Hit: one best row across providers, visually set apart, Enter-bound.
- Adapt quick keys: learn the short string a user types for an item (see [adaptive-input-history](adaptive-input-history.md)).
- Adapt scoped sections as keyboard chips (Ctrl+1..n) on top of the existing prefixes, so scope is visible, not memorised.

## Sources
- https://9to5mac.com/2025/06/10/macos-26-spotlight-gets-actions-clipboard-manager-custom-shortcuts/ — actions, quick keys, clipboard [verified via search summary]
- https://9to5mac.com/2025/11/04/macos-tahoe-26-1-adds-new-tools-for-spotlights-clipboard-feature/ — 26.1 retention and clear [verified via search summary]
- https://en.wikipedia.org/wiki/Spotlight_(Apple) — 2005 origin, Yosemite redesign, Suggestions privacy [memory]
