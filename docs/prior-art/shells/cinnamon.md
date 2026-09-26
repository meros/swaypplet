---
name: Cinnamon
slug: cinnamon
domain: shells
kind: product
platform: other
vendor: Linux Mint
years: 2011–present
status: current
tags: [traditional-desktop, fork, applets, spices, x11]
relevance: low
---

## What it is
Cinnamon is Linux Mint's desktop, forked from GNOME Shell 3 in 2011 to keep a traditional layout: a bottom panel with menu, window list, applets and tray. Extensions ("Spices") come as applets, desklets, extensions and themes. Wayland support has been experimental since 6.0 (2023).

## What was new
Reused GNOME Shell's compositor and JavaScript shell to rebuild a GNOME 2-style desktop, and treated user familiarity as the main requirement. Mint's XApps keep apps consistent across Cinnamon, MATE and Xfce.

## What went wrong / limits
- Inherits the monkey-patching extension model and its breakage.
- Wayland lags years behind; the stack was designed around X11.
- Little new interaction design; its value is continuity.

## Lessons for swaypplet
- Note: the 2011 GNOME Shell reaction shows that users punish removal of a working layout more than they reward novelty; swaypplet's own layout should change only with a reason written down (as ROADMAP does).

## Sources
- https://en.wikipedia.org/wiki/Cinnamon_(desktop_environment) — history, Spices, Wayland status [memory]
