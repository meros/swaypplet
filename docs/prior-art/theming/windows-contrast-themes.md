---
name: Windows high contrast / contrast themes
slug: windows-contrast-themes
domain: theming
kind: feature
platform: Windows
vendor: Microsoft
years: 1995–present
status: current
tags: [accessibility, forced-colors, system-colors, high-contrast]
relevance: medium
---

## What it is
A system mode that replaces every app's colours with a small set of user-chosen system colours (Window, WindowText, Hotlight, GrayText, Highlight, HighlightText, ButtonFace, ButtonText). Windows 11 renamed them contrast themes: Aquatic, Desert, Dusk, Night sky, each editable. Browsers expose it as `forced-colors: active`.

## What was new
- Forced colours: the user's palette overrides the app's, not the other way round. Semantic system colours (`CanvasText`, `LinkText`) are the whole vocabulary.
- Users can edit each of the eight colours, so the mode serves photophobia, low vision and colour-blindness with one mechanism.

## What went wrong / limits
- Apps that paint with images or custom drawing break (icons vanish, focus rings disappear).
- Eight colours cannot express status (success/warning/danger), so status is lost unless paired with icons or text.

## Lessons for swaypplet
- The semantic tier (`--fg`, `--accent-bg`, `--border-strong`) is almost a forced-colours vocabulary already. A "forced" contrast input that maps it onto a user-picked 8-colour set would be small, since rules may use only semantic tokens.
- Principle 5 plus icons for status is what keeps meaning when colour is taken away; keep status glyphs mandatory.

## Sources
- https://learn.microsoft.com/en-us/windows/apps/design/accessibility/high-contrast-themes — system colours, theme names [verified via search summary]
- CSS `forced-colors` media query [memory]
