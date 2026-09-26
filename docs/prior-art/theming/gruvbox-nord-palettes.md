---
name: Gruvbox, Nord and fixed community palettes
slug: gruvbox-nord-palettes
domain: theming
kind: project
platform: cross
vendor: Pavel Pertsev (gruvbox); Arctic Ice Studio (Nord); Ethan Schoonover (Solarized)
years: 2011–present
status: current
tags: [palette, retro, community, terminal, fixed-palette]
relevance: medium
---

## What it is
Hand-made palettes that spread from editors to whole desktops. Solarized (2011) defined 16 colours in CIELAB with symmetric light/dark use; gruvbox (2012) a warm retro set with bright, neutral and faded tones per hue; Nord (2016) four groups (Polar Night, Snow Storm, Frost, Aurora).

## What was new
- Solarized: designed in L*a*b* with fixed lightness relationships so one palette works in both modes by swapping the base tones.
- Gruvbox: three tones per hue (bright/neutral/faded) that map naturally onto text/fill/muted roles, plus hard/medium/soft contrast backgrounds.
- Nord: grouping by job (backgrounds, text, accents, status).

## What went wrong / limits
- Fixed palettes cannot adapt to wallpaper or contrast needs; light variants are often afterthoughts (gruvbox light's cream reads as yellowed).
- Solarized's low-contrast body text is a known criticism.

## Lessons for swaypplet
- swaypplet already inherits gruvbox's per-hue bright/neutral/faded tones for categorical text/fill, and deliberately cut its cream for light glass; that is the right use of a community palette: anchors, not a dictionary.
- Nord's job-grouping mirrors the semantic tier; a "nord" neutral preset is cheap.

## Sources
- https://github.com/morhetz/gruvbox — palette [memory]
- https://www.nordtheme.com/docs/colors-and-palettes — groups [memory]
- https://ethanschoonover.com/solarized/ — CIELAB design [memory]
