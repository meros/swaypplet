---
name: Mica and Acrylic materials
slug: windows-mica
domain: shells
kind: feature
platform: Windows
vendor: Microsoft
years: 2017–present
status: current
tags: [material, wallpaper-tint, performance, fluent, glass]
relevance: high
---

## What it is
Fluent Design's Acrylic (2017) is a live blur with noise and tint for transient surfaces such as flyouts. Mica (Windows 11, 2021) is an opaque material for long-lived windows that is tinted by the desktop wallpaper, sampled once, and shows the wallpaper region under the window. Mica Alt (2022) tints more strongly, for tabbed title bars. Focused windows show Mica; unfocused ones fall back to a flat colour.

## What was new
- A wallpaper-derived material with the cost of a static texture: no per-frame blur of what is behind.
- A rule for which material goes where: Acrylic for transient light-dismiss surfaces, Mica for the app base layer.
- The material doubles as a focus cue (active window tinted, inactive flat).

## What went wrong / limits
- Mica only shows wallpaper, never other windows, so it reads as a coloured background; the effect is subtle and many users do not notice it.
- Win32 apps got it late and inconsistently (22H2 onward).

## Lessons for swaypplet
- Take: the tier rule (transient surfaces vs long-lived) as a material choice; swaypplet's bar (always there) and popovers (transient) could differ in cost.
- Take: sample the wallpaper once and derive the tint from it; this is what swaypplet's wallpaper tint already does for colour.
- Adapt: focus cue by material, e.g. slightly more body on unfocused outputs' bars.

## Sources
- https://learn.microsoft.com/en-us/windows/apps/design/style/mica — sampled once, not a transparency effect, focus cue [verified]
- https://www.xda-developers.com/windows-11-mica-alt-effect/ — Mica Alt [verified]
- https://learn.microsoft.com/en-us/windows/apps/design/style/acrylic — Acrylic usage rules [memory]
