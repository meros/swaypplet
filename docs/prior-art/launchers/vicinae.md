---
name: Vicinae
slug: vicinae
domain: launchers
kind: project
platform: cross
vendor: vicinaehq
years: 2025–present
status: current
tags: [c++, qt, raycast compatible, extensions, dmenu, linux]
relevance: high
---

## What it is
A native C++23/Qt command palette for Linux (also macOS and Windows) that runs Raycast extensions. Built in: apps, files, clipboard history, snippets, browser tabs, emoji, calculator, window/workspace switcher, fonts, volume.

## What was new
- **Borrowing an ecosystem**: implements enough of Raycast's React API that many store extensions run unchanged, alongside its own store, script commands and dmenu mode.
- Native Qt UI for Raycast's declarative views, so extensions look native on Linux.

## What went wrong / limits
- Compatibility is partial: macOS-only APIs (AppleScript, Keychain) have no Linux equivalent.
- Inherits Raycast's weak sandbox and adds a Node runtime to a Linux desktop.

## Lessons for swaypplet
- Worth watching as the Linux answer to "which extension ecosystem"; if swaypplet ever needs extensions, speaking an existing API beats inventing one.
- Its built-in list is a good checklist for gaps: browser tabs, snippets, fonts.

## Sources
- https://github.com/vicinaehq/vicinae — language, features, Raycast compatibility [verified]
