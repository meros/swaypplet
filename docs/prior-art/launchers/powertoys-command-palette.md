---
name: PowerToys Command Palette
slug: powertoys-command-palette
domain: launchers
kind: product
platform: Windows
vendor: Microsoft (open source)
years: 2025–present
status: current
tags: [out-of-process extensions, winget, home pins, dock, compact mode, fallback]
relevance: high
---

## What it is
The successor to PowerToys Run, opened with Win+Alt+Space. It launches apps, runs `>` commands, `??` web search, `$` settings pages, `=` or bare arithmetic, window switching, clipboard, services, registry, WinGet install and system metrics.

## What was new
- **Extensions out of process**: extensions are separate packaged apps talking over COM/WinRT, listed in a built-in Extension Gallery and installed through WinGet or the Store; a crash stays in the extension.
- **Nested pages**: a command can open a list, a form or detail page inside the palette.
- **Home page** with recent items and user **Pinned commands**; the same commands can be pinned to a screen-edge **Dock**.
- **Compact mode**: only the search box shows until results need room; Down or Tab expands.
- **Bookmarks with placeholders**: `{query}` in a URL is asked for inline, not in a dialog.
- Contextual system rows appear when relevant ("Update and restart" only while an update waits).

## What went wrong / limits
- Young; the extension surface and gallery are still settling, and PowerToys Run users lost plugins in the move.
- Being a PowerToys module, it is opt-in and not the OS default search.

## Lessons for swaypplet
- Take pins on the empty query: user-pinned rows above frecent ones, so habit and intent both show.
- Take context-conditional rows (e.g. "Reboot to apply update" only when `nx pending` is true).
- Take out-of-process extensions as the only acceptable plugin shape; elephant already is that boundary.

## Sources
- https://learn.microsoft.com/en-us/windows/powertoys/command-palette/overview — features, home pins, dock, compact mode, gallery [verified]
- https://learn.microsoft.com/en-us/windows/powertoys/command-palette/extensibility-overview — COM out-of-proc extensions [memory]
