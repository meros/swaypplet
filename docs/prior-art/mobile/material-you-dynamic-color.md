---
name: Material You dynamic color (monet)
slug: material-you-dynamic-color
domain: mobile
kind: feature
platform: Android
vendor: Google
years: 2021–present
status: current
tags: [wallpaper, palette, theming, tonal-palette, hct, color-extraction]
relevance: high
---

## What it is
Android 12's system theming: a seed colour is extracted from the wallpaper (`ColorScheme#getSeedColors`, falling back to `0xFF1B6EF3`) and expanded into five tonal palettes (accent1–3, neutral1–2). Each palette has 13 tones (0, 10, 50, 100 … 1000), about 65 colours in all, exposed to apps as `R.color.system_*`.

## What was new
- **Hue and chroma come from the image; tone is fixed per role.** Contrast is designed into the tone steps, so any wallpaper yields a legible scheme. That is what separates it from pywal-style extraction.
- The HCT colour space and the `Score` ranking (area and chroma) are published in the open-source Material Color Utilities, so other platforms can reproduce the same result.
- **Styles as a second dial**: TONAL_SPOT (the default), VIBRANT, EXPRESSIVE, SPRITZ, RAINBOW and FRUIT_SALAD, all run on the same seed.
- The user can override the seed with one of up to four candidate colours in the wallpaper picker, or with a fixed "basic colour".

## What went wrong / limits
- The default Tonal Spot is muted pastel. Many wallpapers give a washed-out result, and users reached for the Vibrant style or a fixed colour.
- It shipped on Pixel first, and OEM implementations differed (Samsung's own "Color palette"), so apps could not count on one output.
- Third-party app adoption was slow. A themed system beside untinted apps looked patchy for years.

## Lessons for swaypplet
- **Already taken, and correctly**: swaypplet's tint uses Material's `Score` and keeps lightness per role (design-system §2.2). This entry confirms that choice.
- **Adapt the styles idea**: the tint's `off/accents/full` levels are a degree; Material adds a *character* axis (spritz to vibrant, chroma scaling). A single "chroma" knob would cover the muted-pastel complaint cheaply.
- **Show the candidates**: showing the 2–4 ranked wallpaper hues as swatches in the settings pane lets the user pick when `Score` picks the boat instead of the sky.

## Sources
- https://source.android.com/docs/core/display/dynamic-color — seed, 5×13 palettes, six styles, R.color access [verified]
- https://www.xda-developers.com/material-you-monet-theme-engine/ — monet engine background [verified via search summary]
- https://github.com/material-foundation/material-color-utilities — HCT, Score, open-source [memory]
