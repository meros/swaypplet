---
name: HCT colour space (CAM16 hue/chroma + L* tone)
slug: hct-cam16
domain: theming
kind: pattern
platform: cross
vendor: Google (Material Design team)
years: 2021–present
status: current
tags: [colour-space, cam16, cielab, tone, contrast, gamut-mapping]
relevance: medium
---

## What it is
HCT combines CAM16's hue and chroma with CIELAB's L* as "tone" (0–100). It is the colour space under Material 3 dynamic colour and is implemented in material-color-utilities (Java, Kotlin, Dart, TS, C++, Swift, and Rust ports such as `hct-cam16`, `mcu-material-color`).

## What was new
- Tone is L*, which maps directly to relative luminance, so a tone difference predicts a WCAG contrast ratio regardless of hue (tone 40 vs 100 ≥ 4.5:1 in practice; a 50-tone gap ≈ 4.5:1).
- A gamut solver finds the in-sRGB colour with the requested hue and tone and the highest chroma it can keep, reducing chroma rather than shifting tone.
- CAM16 hue stays stable when chroma drops, unlike HSL.

## What went wrong / limits
- Expensive: CAM16 needs viewing conditions and iterative solving, which is why the utilities cache palettes.
- Tone ≈ contrast holds for WCAG 2's formula; it does not predict APCA Lc, which is polarity- and size-aware.
- Hue uniformity is good but not better than OKLCH for UI palettes in most comparisons; OKLCH has a closed-form inverse.

## Lessons for swaypplet
- Keep OKLCH: swaypplet's rule "chroma gives way, never lightness" is HCT's gamut policy expressed in a cheaper space.
- The "tone gap predicts contrast" idea is worth a unit test: assert that each semantic pair keeps a minimum L gap, as a fast pre-check before the APCA-through-glass test.

## Sources
- https://github.com/material-foundation/material-color-utilities/ — HCT definition and implementations [verified]
- https://github.com/edgarhsanchez/hct_cam16 — Rust port, gamut-mapping solver [verified]
- https://material.io/blog/science-of-color-design — tone/contrast rationale [memory]
