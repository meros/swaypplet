---
name: Material You dynamic color
slug: material-you-dynamic-color
domain: theming
kind: feature
platform: Android
vendor: Google
years: 2021–present
status: current
tags: [dynamic-color, wallpaper, tonal-palette, material-3, accent]
relevance: high
---

## What it is
Android 12's system theme derived from the wallpaper. The system extracts a source colour, builds five tonal palettes (primary, secondary, tertiary, neutral, neutral-variant) and maps roles such as `primary`, `onPrimary`, `surfaceContainer` to fixed tones in each palette, per mode. Apps opt in with `dynamicLightColorScheme` / `dynamicDarkColorScheme`.

## What was new
- The wallpaper chooses hue and chroma; the role chooses tone. Text/background pairs are separated by a fixed tone distance, so contrast holds for any source colour ("on" colours are paired algorithmically).
- One source feeds the whole OS: launcher, quick settings, notifications, and third-party apps through a published scheme.
- Scheme variants (TonalSpot default, Vibrant, Expressive, Fidelity, Content, Monochrome, Neutral, Rainbow, Fruit Salad) let the same seed produce quiet or loud themes.
- Users get 4 wallpaper-derived options plus fixed "basic colours" in the picker, so a bad extraction is recoverable.

## What went wrong / limits
- Neutral surfaces carry a visible tint of the seed, which some users read as muddy; Pixel added "Monochrome" and tone-down variants.
- Many apps keep brand colour and ignore dynamic colour, so the OS looks consistent only in Google's own apps.
- Tertiary colour is computed by hue rotation (+60° in TonalSpot), not taken from the image, so it can clash with the wallpaper.

## Lessons for swaypplet
- Take (already taken): hue from the image, tone from the role. swaypplet's §2.2 rule of "only hue moves" is the same contract.
- Adapt: offer the 3–4 candidate hues from the palette as picks instead of only primary, as Android's picker does; a wrong Score pick then costs one click.
- Avoid deriving a secondary by rotation; swaypplet's image-sourced secondary with a 45° floor is better grounded.

## Sources
- https://m3.material.io/styles/color/dynamic-color/overview — role/tone pairing, accessibility by construction [verified]
- https://developer.android.com/develop/ui/views/theming/dynamic-colors — app opt-in [verified]
- https://github.com/material-foundation/material-color-utilities/ — scheme variants, five palettes [verified]
- Android 12 wallpaper picker offering wallpaper and basic colours [memory]
