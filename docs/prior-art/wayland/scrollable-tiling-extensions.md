---
name: PaperWM and Karousel (scrollable tiling on GNOME and KDE)
slug: scrollable-tiling-extensions
domain: wayland
kind: pattern
platform: GNOME
vendor: PaperWM community; Peter Fajdiga (Karousel)
years: 2017–present
status: niche
tags: [scrollable-tiling, paper, extension, kwin-script]
relevance: low
---

## What it is
PaperWM (GNOME Shell extension) and Karousel (KWin script) place windows on a horizontal strip that scrolls, with per-monitor workspaces. PaperWM predates niri and inspired it.

## What was new
- The "paper" metaphor: new windows open to the right of the focused one, full height, width under user control; you scroll, never shrink.
- Works especially well on ultrawide screens.

## What went wrong / limits
- As extensions they inherit every GNOME release break; PaperWM changed maintainers and repositories more than once (jvns fork, paperwm org) [memory].
- A native compositor (niri) delivered the same idea with better animations and no host churn, and drew most of the users.

## Lessons for swaypplet
- An idea proven as an extension succeeds only when someone builds it natively; do not ship core behaviour as a layer on someone else's internals.

## Sources
- https://github.com/paperwm/PaperWM — PaperWM [verified via search summary]
- https://github.com/peterfajdiga/karousel/blob/master/README.md — Karousel [verified via search summary]
