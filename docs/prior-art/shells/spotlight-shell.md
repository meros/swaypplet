---
name: Spotlight as shell command line
slug: spotlight-shell
domain: shells
kind: feature
platform: macOS
vendor: Apple
years: 2005–present
status: current
tags: [search, launcher, actions, clipboard-history, keyboard]
relevance: high
---

## What it is
Spotlight (Tiger, 2005) is the system-wide search field on Cmd+Space. It became the main app launcher for keyboard users. In macOS 26 Tahoe it gained filtered views (Cmd+1 apps, Cmd+2 files, Cmd+3 actions, Cmd+4 clipboard history), app actions and user-defined "quick keys" (for example `st` for Start Timer).

## What was new
- One keyboard entry point for launch, search and commands, with the shell learning from usage.
- Tahoe: actions exposed by apps (App Intents) can be run with parameters without opening the app; clipboard history lives in the same launcher, with a preview column.

## What went wrong / limits
- For years third parties (Quicksilver, LaunchBar, Alfred, Raycast) did this better; Tahoe mostly catches up with them.
- Action discovery is poor: the Cmd+3 list is the only way to see what exists.
- Clipboard history is off in some contexts and has short retention.

## Lessons for swaypplet
- Take: prefixes or mode keys in one launcher (apps, files, actions, clipboard) rather than separate tools; fits the `launcher` branch.
- Take: user-defined abbreviations that pin a query to a result.
- Consistent with the roadmap: clipboard history without disk persistence.

## Sources
- https://9to5mac.com/2025/06/10/macos-26-spotlight-gets-actions-clipboard-manager-custom-shortcuts/ — Tahoe actions, clipboard, quick keys [verified]
- https://macmost.com/how-to-use-the-spotlight-clipboard-history-in-macos-tahoe.html — Cmd+4 clipboard view [verified]
