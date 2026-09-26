---
name: Tailwind CSS v4 OKLCH palette
slug: tailwind-v4-oklch
domain: theming
kind: feature
platform: web
vendor: Tailwind Labs
years: 2025–present
status: current
tags: [oklch, p3, palette, css-variables, wide-gamut]
relevance: medium
---

## What it is
Tailwind v4 (January 2025) redefined its default palette (22 hues × 11 shades, 50–950) in `oklch()` and exposes every token as a CSS custom property through `@theme`.

## What was new
- Brought OKLCH to the most widely used CSS framework; wide-gamut P3 colours where displays allow, sRGB fallback by the browser.
- Theme tokens as real CSS variables, so runtime theming is overriding variables.
- `color-mix(in oklab, …)` for opacity modifiers.

## What went wrong / limits
- Shades are numbered, not jobbed; contrast between shade numbers varies by hue (yellow-500 vs blue-500).
- P3 colours clip unpredictably in screenshots and on sRGB displays.

## Lessons for swaypplet
- GTK does not do wide gamut; keep sRGB output but OKLCH generation.
- Confirms `color-mix` over opacity is the portable state-overlay primitive swaypplet uses.

## Sources
- https://tailwindcss.com/blog/tailwindcss-v4 — OKLCH palette, CSS-first theme [verified via search summary]
- https://tailwindcss.com/docs/colors — palette structure [verified via search summary]
