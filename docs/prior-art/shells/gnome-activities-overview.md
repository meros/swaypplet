---
name: GNOME Activities overview
slug: gnome-activities-overview
domain: shells
kind: feature
platform: GNOME
vendor: GNOME Project
years: 2011–present
status: current
tags: [overview, workspaces, search, gestures, spatial]
relevance: high
---

## What it is
GNOME Shell's central surface, opened with Super, the Activities button or a hot corner: all windows of the current workspace spread out, workspaces beside them, the dash (favourites and running apps) and a search field that starts on typing. GNOME 40 (2021) turned workspaces horizontal, with the overview between the desktop and the app grid on a vertical axis.

## What was new
- One key opens launcher, window picker and workspace switcher at once, and typing immediately searches.
- GNOME 40's spatial model: workspaces left-right, zoom levels up-down (desktop, overview, app grid), mapped one-to-one onto three- and four-finger touchpad gestures. User testing found it easier to understand than the vertical layout.

## What went wrong / limits
- The 2011 switch from GNOME 2's panel to a modal overview drove many users to forks (Cinnamon, MATE) and to Unity.
- Every task goes through a full-screen mode change; for frequent app switching it is heavier than a taskbar.
- Multi-monitor: workspaces only on the primary display by default.

## Lessons for swaypplet
- Take: a single spatial map where every gesture and key moves along a stated axis; for swaypplet, Super+Tab strip and overview should share one geometry.
- Take: type-to-search from the overview, so the overview and launcher are one surface.
- Avoid: making the overview the only way to switch; sway keybinds stay primary.

## Sources
- https://blogs.gnome.org/shell-dev/2020/12/18/gnome-shell-ux-plans-for-gnome-40/ — horizontal rationale, testing [verified]
- https://en.wikipedia.org/wiki/GNOME_Shell — history and reception [memory]
