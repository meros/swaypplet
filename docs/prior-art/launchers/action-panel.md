---
name: Secondary actions on a result (action panel)
slug: action-panel
domain: launchers
kind: pattern
platform: cross
vendor: Quicksilver, LaunchBar, Alfred, Raycast, KRunner
years: 2003–present
status: current
tags: [actions, modifiers, cmd-k, tab, right arrow, verb-object]
relevance: high
---

## What it is
Each result has more than one thing you can do with it. Quicksilver's second pane, LaunchBar's Right arrow, Alfred's modifier keys (Cmd/Alt/Ctrl+Enter), KRunner's action buttons and Raycast's Cmd+K panel are variants.

## What was new
- Raycast made it uniform: Enter is action 1, Cmd+Enter action 2, Cmd+K opens a searchable list of all actions, each with its shortcut shown.
- Actions attach by result kind (file: open, reveal, copy path, move to trash; app: launch, new window, quit, show windows).

## What went wrong / limits
- Modifier-only actions are invisible; users do not find them unless the UI shows the hint.

## Lessons for swaypplet
- Generalise Tab-for-windows into an action list per row kind: app (launch, new window, windows ▸, move to workspace), window (focus, close, float), file (open, reveal, copy path), clipboard (paste, delete). Show the second action's key on the selected row. M.
- elephant already returns `actions` per row; the UI only shows the default one today.

## Sources
- https://developers.raycast.com/api-reference/user-interface/action-panel — Raycast action panel [memory]
- https://www.alfredapp.com/help/features/universal-actions/ — Alfred actions [verified via search summary]
- https://develop.kde.org/docs/plasma/krunner/ — KRunner actions [verified]
