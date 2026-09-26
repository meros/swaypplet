---
name: Apple materials and vibrancy (NSVisualEffectView)
slug: apple-vibrancy-materials
domain: theming
kind: feature
platform: macOS
vendor: Apple
years: 2014–present
status: current
tags: [materials, blur, vibrancy, translucency, semantic-materials]
relevance: high
---

## What it is
OS X Yosemite (2014) introduced `NSVisualEffectView`: a view that blurs what is behind it (behind-window or within-window blending) and renders its content with "vibrancy", a blend that pulls colour from the backdrop into text and glyphs. iOS has the equivalent `UIVisualEffectView` with `ultraThin` to `thick` materials.

## What was new
- Materials are named by purpose (`sidebar`, `menu`, `popover`, `hudWindow`, `titlebar`, `headerView`), not by blur radius; each adapts to light/dark appearance.
- Vibrancy treats foreground content as part of the material: labels are blended (plus-darker / plus-lighter), so grey text keeps contrast over any backdrop colour.
- Separate "thickness" levels on iOS let the designer trade legibility for context per surface.

## What went wrong / limits
- Vibrant text can drop below readable contrast over busy or mid-tone backdrops; Reduce Transparency exists partly to rescue it.
- Blending modes are private; third parties (Electron, Qt) approximate them poorly.
- Early Yosemite translucency was criticised as costing performance on older Macs.

## Lessons for swaypplet
- Take: name materials by role. swaypplet already has "thin" and "card" glass namespaces; a role name per namespace (bar, card, menu, OSD) documents why their values differ.
- Adapt: vibrancy is Apple's answer to swaypplet's principle 7 ("states are overlays of the content's own colour"); a compositor-side vibrancy for secondary text is a possible next step but costs the APCA guarantee, so measure first.

## Sources
- https://mackuba.eu/2018/07/04/dark-side-mac-1/ — materials and appearances [verified via search summary]
- https://oskargroth.com/blog/reverse-engineering-nsvisualeffectview — internals, blend modes [verified via search summary]
- https://asciiwwdc.com/2014/sessions/220 — WWDC 2014 introduction [verified via search summary]
