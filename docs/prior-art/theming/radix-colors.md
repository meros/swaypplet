---
name: Radix Colors
slug: radix-colors
domain: theming
kind: project
platform: web
vendor: WorkOS (Radix)
years: 2021–present
status: current
tags: [12-step-scale, step-jobs, dark-mode, alpha-scales, custom-palette]
relevance: high
---

## What it is
A colour system of 12-step scales per hue, each in light and dark, with solid and alpha variants. Every step has a job: 1–2 app backgrounds, 3–5 component backgrounds (rest, hover, pressed/selected), 6–8 borders (subtle, interactive, hover/focus), 9–10 solid (and hover), 11–12 text (low and high contrast).

## What was new
- Steps defined by use, not lightness, and the job is the same in both modes, so a component written against steps works in dark and light.
- Alpha scales that look identical to solids over the page background, for use over unknown surfaces.
- Per-hue decision on white vs dark text on step 9 (yellow, lime, amber, mint, sky take dark text).
- Radix Themes (2023) added a custom palette generator from an accent, gray and background.

## What went wrong / limits
- Hand-tuned scales: adding a hue means hand work, which the custom generator only approximates.
- Contrast targets are informal (APCA-guided, not tested per pair).

## Lessons for swaypplet
- Already the model for swaypplet's primitives (§3.1) including the per-accent on-accent decision.
- Borrow the alpha-scale idea explicitly for the glass: swaypplet's `--fill-*` as `currentColor` mixes are the same answer for an unknown backdrop.

## Sources
- https://www.radix-ui.com/colors/docs/palette-composition/understanding-the-scale — step jobs [verified via search summary]
- https://www.radix-ui.com/themes/docs/theme/color — custom palettes [verified via search summary]
