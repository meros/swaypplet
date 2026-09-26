---
name: PowerToys FancyZones
slug: fancyzones
domain: notifications-wm
kind: project
platform: Windows
vendor: Microsoft (PowerToys)
years: 2019–present
status: current
tags: [zones, custom-layouts, shift-drag, tiling]
relevance: low
---

## What it is
A PowerToys utility for user-drawn zone layouts, grid or free canvas, per monitor. Holding Shift while dragging a window shows the zones; dropping snaps the window into one.

## What was new
Custom layouts with a modifier to avoid accidental snapping, and layouts per monitor, for ultrawide screens where half/half is useless.

## What went wrong / limits
Pointer-driven; layouts do not adapt to window content. A power-user tool on top of a system that does not tile.

## Lessons for swaypplet
- Note: a modifier-gated drop target is the right pattern if swaypplet ever adds drag-to-workspace in the overview: nothing snaps without the modifier.

## Sources
- https://learn.microsoft.com/en-us/windows/powertoys/fancyzones — utility docs, Shift drag, editor. [verified via search summary]
- https://www.howtogeek.com/windows-11-powertoys-fancyzones/ — usage. [verified via search summary]
