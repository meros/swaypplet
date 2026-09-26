---
name: dwl
slug: dwl
domain: wayland
kind: product
platform: Wayland
vendor: djpohly, then the dwl community (Codeberg)
years: 2020–present
status: niche
tags: [dwm, suckless, patches, minimal, wlroots]
relevance: low
---

## What it is
dwm for Wayland: a single C file of a few thousand lines on wlroots, configured by editing `config.h` and recompiling. Features come as patches from a community wiki.

## What was new
- A compositor small enough to read in an afternoon; startup and memory are close to zero.
- dwm's tag model (a window can carry several tags) rather than workspaces.

## What went wrong / limits
- Patches conflict with each other and rot on every wlroots bump; users maintain private forks.
- Forks such as mango (MangoWC) grew out of patch piles to ship animations and blur [memory].

## Lessons for swaypplet
- Avoid patch-set distribution for the swayfx changes: keep them as a rebased branch with tests, not a pile of diffs.

## Sources
- https://codeberg.org/dwl/dwl — project and patches wiki [memory]
