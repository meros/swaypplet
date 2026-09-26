---
name: tvOS focus engine and parallax
slug: tvos-focus-engine
domain: mobile
kind: pattern
platform: other
vendor: Apple
years: 2015–present
status: current
tags: [focus, keyboard-navigation, parallax, 10-foot-ui, depth]
relevance: medium
---

## What it is
tvOS has no pointer: a **focus engine** moves focus between elements by direction, choosing the nearest focusable element in the swiped direction. The focused item lifts (scales up, gains a shadow), and **layered images** (up to five layers) tilt in parallax as the thumb moves slightly on the remote's touch surface, so focus feels physical.

## What was new
- **Directional focus as a system service**, not per-app code, with every screen defining a default focused element.
- **Focus shown as elevation and motion**, not only a ring: the focused tile scales and tilts.
- A focus preview: the tiny tilt under the resting thumb confirms which item is focused before a click.

## What went wrong / limits
- Directional guessing fails on irregular layouts; developers add invisible focus guides.
- The motion only makes sense at 10 feet; up close, parallax is decoration.

## Lessons for swaypplet
- **Take directional focus for keyboard-first surfaces**: the panel, launcher and Super+Tab grid should move focus by arrow keys geometrically, with a defined default on open (P8).
- **Adapt focus as lift**: a focused tile shown by a state overlay plus a small scale (not a colour change) fits principle 7, which makes states overlays of the content's own colour.

## Sources
- https://www.brightec.co.uk/blog/tvos-focus-engine — directional focus [verified via search summary]
- https://learn.microsoft.com/en-us/xamarin/ios/tvos/app-fundamentals/navigation-focus — layered images and parallax [verified via search summary]
