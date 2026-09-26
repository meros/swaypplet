---
name: Hot corners
slug: hot-corners
domain: shells
kind: pattern
platform: cross
vendor: Apple, GNOME, Microsoft
years: 2003–present
status: current
tags: [pointer, fitts-law, corners, accidental-activation]
relevance: low
---

## What it is
Moving the pointer into a screen corner triggers an action: macOS assigns Mission Control, desktop, screen saver or Quick Note per corner (off by default except Quick Note since Monterey); GNOME Shell opens the Activities overview from the top-left; Windows 8 revealed Charms and the app switcher from corners.

## What was new
Corners are the four easiest pointer targets on a screen (two edges stop the pointer), so a flick without aiming triggers the action.

## What went wrong / limits
- Accidental activation, especially on multi-monitor setups where a corner is shared with the next display, and when reaching for a close button or a menu.
- No affordance: nothing on screen says the corner does anything (Windows 8's main failure).
- GNOME added pressure thresholds (a pointer barrier) to reduce false triggers and made the corner optional in 3.34 settings.

## Lessons for swaypplet
- Avoid for primary functions; swaypplet is keyboard-first and BAR_VISION P8 rules out invisible pointer triggers.
- If ever added, use pressure (distance past the edge over time), not mere contact.

## Sources
- https://support.apple.com/guide/mac-help/use-hot-corners-mchlp3000/mac — macOS hot corners [memory]
- https://en.wikipedia.org/wiki/GNOME_Shell — Activities hot corner and pressure barrier [memory]
