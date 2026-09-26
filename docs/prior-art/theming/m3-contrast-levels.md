---
name: Material 3 contrast levels and Android colour contrast setting
slug: m3-contrast-levels
domain: theming
kind: feature
platform: Android
vendor: Google
years: 2023–present
status: current
tags: [contrast, accessibility, dynamic-scheme, high-contrast, user-setting]
relevance: high
---

## What it is
`DynamicScheme` takes a `contrastLevel` from −1 to 1 (0 standard, 0.5 medium, 1 high). Each colour role declares a target contrast curve against its background, so raising the level moves tones apart instead of swapping in a separate theme. Android exposes it as a Standard / Medium / High colour contrast setting (Android 14 QPR / 15 era) that apps receive through dynamic colour.

## What was new
- Contrast is a continuous input to the generator, not a separate hand-drawn "high contrast" theme.
- Roles are declared as constraints (`ContrastCurve(3, 4.5, 7, 11)`), and the solver picks tones that meet them at each level.
- Dynamic and brand schemes both get the higher contrast for free.

## What went wrong / limits
- Only apps built on dynamic schemes follow; hard-coded colours do not.
- Constraints use WCAG 2 ratios, which overstate dark-mode contrast and understate light text on colour.

## Lessons for swaypplet
- Adapt: express each text token as a target Lc per contrast level and let the generator solve alpha/lightness, instead of hand-tuned 84 %/88 % tables. The APCA test already exists; make it the solver.
- A third "medium" step is cheap once contrast is solved rather than tabulated.

## Sources
- https://github.com/material-foundation/material-color-utilities/ — `DynamicScheme(contrastLevel)`, `ContrastCurve` [verified via repo listing; constants memory]
- https://m3.material.io/styles/color/overview — contrast levels in M3 [verified]
- Android colour contrast setting location and release [memory]
