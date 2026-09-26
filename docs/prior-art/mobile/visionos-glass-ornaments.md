---
name: visionOS glass material and ornaments
slug: visionos-glass-ornaments
domain: mobile
kind: feature
platform: other
vendor: Apple
years: 2023–present
status: current
tags: [glass, vibrancy, spatial, ornaments, hierarchy, legibility]
relevance: high
---

## What it is
visionOS draws every app window on a system glass material that adapts to the real room behind it. An **ornament** is a toolbar or tab bar attached to a window's edge, floating slightly in front of it on its own plane, and it keeps its position relative to the window as the window moves.

## What was new
- **Vibrancy as the hierarchy system.** Text and symbols on glass come in primary, secondary and tertiary vibrancy levels. These are not fixed colours: they are blended against whatever is behind the glass, so the hierarchy holds over any backdrop.
- Glass that corrects for its backdrop: in a bright room it darkens the scene behind it to keep contrast, and Apple does not offer a light-mode variant for windows.
- Ornaments move controls off the content surface. The window holds content, and actions sit on a separate layer at its edge.

## What went wrong / limits
- The whole approach depends on eye tracking and head-fixed depth cues. A flat desktop has neither, so ornaments turn into ordinary attached toolbars.
- Vision Pro sales stayed small, so the patterns are known mostly from the guidelines, not from wide use.

## Lessons for swaypplet
- **Adapt vibrancy levels to the three text levels.** `--fg`, `--fg-muted` and `--fg-faint` are already three tiers; visionOS shows they can be defined relative to the material, not as absolute colours, and still hold.
- **Take "no light glass".** visionOS avoids light glass because contrast is easier to guarantee on a darkened backdrop. That is worth weighing when light mode sits over a bright wallpaper.
- Ornaments show a clean split between content and controls, the same line as swaypplet's "glass is the surface; nothing on it is glass".

## Sources
- https://www.createwithswift.com/ensuring-interface-legibility-and-contrast-in-visionos/ — glass adapts to backdrop, vibrancy levels [verified via search summary]
- https://www.createwithswift.com/creating-ornaments-in-visionos/ — ornament definition [verified via search summary]
- https://developer.apple.com/design/human-interface-guidelines/materials — visionOS glass, no light/dark distinction [memory]
