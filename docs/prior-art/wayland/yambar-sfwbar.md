---
name: yambar and sfwbar
slug: yambar-sfwbar
domain: wayland
kind: product
platform: Wayland
vendor: Daniel Eklöf (yambar); Lev Babiev (sfwbar)
years: 2019–present
status: niche
tags: [bar, lightweight, c, taskbar, floating]
relevance: low
---

## What it is
yambar is a C bar by the author of foot, configured in YAML, rendering with pixman and no toolkit. sfwbar ("S* Floating Window Bar") is a C/GTK3 taskbar and panel aimed at users who float windows in sway, with a pager, taskbar and a scanner language that parses files and command output into variables.

## What was new
- yambar: a bar with single-digit MB memory and near-zero idle CPU, by avoiding a toolkit and redrawing only on change.
- sfwbar: a Windows-style taskbar for a tiling compositor, built on foreign-toplevel.

## What went wrong / limits
- yambar's minimal look needs heavy config for anything modern; development slowed [memory].
- sfwbar's config language is its own dialect.

## Lessons for swaypplet
- yambar is the idle-cost benchmark: swaypplet's bar at rest should aim for zero wakeups, which P7 already demands; measure against it.

## Sources
- https://codeberg.org/dnkl/yambar — yambar [memory]
- https://github.com/LBCrion/sfwbar — sfwbar [memory]
