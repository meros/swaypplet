---
name: Mac OS X Aqua
slug: mac-os-x-aqua
domain: shells
kind: product
platform: macOS
vendor: Apple
years: 2000–2012
status: discontinued
tags: [historic, visual-language, skeuomorphism, translucency, pinstripes]
relevance: medium
---

## What it is
The visual language of Mac OS X from its 2000 preview and 10.0 (2001): gel "lickable" buttons, pulsing default buttons, pinstriped and translucent menus, drop shadows on every window, and the genie minimize. Through 10.2–10.7 it gained brushed metal, then unified grey toolbars, and was flattened in Yosemite (2014).

## What was new
Compositing (Quartz) let every window have a soft shadow and menus be translucent, so depth became a system-wide cue. The pulsing default button signalled the primary action.

## What went wrong / limits
- Translucent menus over text reduced legibility; Apple reduced menu transparency in 10.5 after complaints, then again in later releases.
- The pulsing default button was a continuous animation for a static fact; removed in Yosemite.
- Two competing window materials (Aqua and brushed metal) for years, used inconsistently even by Apple.

## Lessons for swaypplet
- Avoid: continuous motion for a static state (the pulsing button). Matches BAR_VISION P2.
- Avoid: two materials for the same role; one glass per surface.
- Note: the transparency-then-retreat cycle (Aqua 2001, Yosemite 2014, Liquid Glass 2025) repeats every decade.

## Sources
- https://en.wikipedia.org/wiki/Aqua_(user_interface) — history, brushed metal, Yosemite [memory]
