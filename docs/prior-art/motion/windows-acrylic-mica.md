---
name: Windows Acrylic and Mica
slug: windows-acrylic-mica
domain: motion
kind: feature
platform: Windows
vendor: Microsoft
years: 2017–present
status: current
tags: [blur, acrylic, mica, noise, luminosity, fallback, power]
relevance: high
---

## What it is
Acrylic (Fluent, 2017) is a live blur of what is behind a surface, plus exclusion blend, tint, luminosity and a noise texture. Mica (Windows 11, 2021) tints with a blurred copy of the desktop wallpaper only.

## What was new
- Mica's cost trick: it "only samples the desktop wallpaper once", so a window with Mica costs almost nothing per frame and still looks personal.
- Acrylic's noise layer (about 2 % grain) hides banding in a dark blur.
- Both have defined solid fallbacks used automatically with Battery Saver, "Transparency effects" off, low-end hardware, and (for Acrylic) inactive windows.

## What went wrong / limits
- Acrylic on Windows 10 made window dragging lag on many machines; Microsoft limited in-app acrylic and moved system surfaces to Mica.
- Mica reflects the wallpaper, not what is actually behind, which confuses users who expect see-through.

## Lessons for swaypplet
- Take: a Mica-like tier for large, static namespaces: sample the wallpaper blur once and composite it, no live backdrop pass.
- Take: define inactive and battery fallbacks up front, as Microsoft did after the Acrylic lag.

## Sources
- https://learn.microsoft.com/en-us/windows/apps/design/style/acrylic — recipe, fallback conditions [verified via search summary and research/adaptive-glass.md]
- https://learn.microsoft.com/en-us/windows/apps/design/style/mica — samples wallpaper once, fallback [verified by sibling doc]
