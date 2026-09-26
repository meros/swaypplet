---
name: BeOS and Haiku Deskbar
slug: haiku-deskbar
domain: shells
kind: product
platform: other
vendor: Be Inc., Haiku Project
years: 1996–present
status: niche
tags: [taskbar, compact, replicants, historic, corner]
relevance: medium
---

## What it is
The Deskbar is BeOS's (and Haiku's) combined start menu, tray and window list, docked in a screen corner by default as a compact vertical block: the "leaf" menu on top, a shelf of replicants and the clock, then running apps, each expandable to its windows. It can also stretch horizontally along the top or bottom edge. Windows have the yellow title "tab" rather than full-width title bars.

## What was new
- Small footprint by default: a corner, not an edge, with the list growing only as apps open.
- Replicants: archivable views that apps export and the Deskbar (or the desktop) hosts, e.g. a CPU graph or workspace pager, a 1990s version of out-of-process widgets.

## What went wrong / limits
- BeOS failed commercially (Be Inc. sold to Palm in 2001); the ideas survived only in Haiku, which reached R1 beta status only in 2018.
- Replicants ran in the host's process space via archived objects, so a bad replicant could crash the Deskbar.

## Lessons for swaypplet
- Take: a shell surface sized by its content (the corner block) rather than claiming a full edge is an option for secondary outputs.
- Avoid: hosting other programs' code in the shell's process (the replicant crash mode).

## Sources
- https://www.haiku-os.org/docs/userguide/en/deskbar.html — Deskbar layout and options [verified]
- https://www.haiku-os.org/docs/userguide/en/gui.html — replicants [verified]
