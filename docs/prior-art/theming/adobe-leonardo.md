---
name: Adobe Leonardo (contrast-based colour)
slug: adobe-leonardo
domain: theming
kind: project
platform: cross
vendor: Adobe (Nate Baldwin)
years: 2019–present
status: current
tags: [contrast-ratio, adaptive-color, spectrum, background-brightness, generator]
relevance: high
---

## What it is
An open-source generator (`@adobe/leonardo-contrast-colors`, leonardocolor.io) that produces colours by target contrast ratio against a background. Key colours define a lightness curve; the tool solves for the colour on that curve that hits each requested ratio. Powers adaptive colour in Adobe Spectrum.

## What was new
- Contrast is the input and colour the output: a theme is a list of ratios (e.g. 3, 4.5, 7) per role.
- Changing the background brightness (and "contrast" as a user parameter) regenerates the whole palette, so dark, darkest and high-contrast themes are one function evaluated at different arguments.
- Supports WCAG 2 and APCA as the contrast function.

## What went wrong / limits
- Solving per ratio gives uneven perceptual steps between roles.
- Spectrum still ships fixed palettes for engineering simplicity; the adaptive runtime was never the default.

## Lessons for swaypplet
- Closest prior art to what swaypplet could become: make the glass-aware APCA function the solver. Instead of fixed alphas for `--fg-muted` and `--fg-faint`, solve the lowest alpha that meets Lc 60/45 through the material per input set; the test then can never fail.
- Leonardo's "contrast" slider as a continuous user parameter is the upgrade path from standard/high.

## Sources
- https://github.com/adobe/leonardo — generator, target ratios [verified via search summary]
- https://medium.com/thinking-design/adaptive-color-in-spectrum-adobes-design-system-feeeec89a2c7 — adaptive colour in Spectrum [verified via search summary]
- APCA support in Leonardo [memory]
