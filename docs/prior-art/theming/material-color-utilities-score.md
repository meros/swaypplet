---
name: material-color-utilities quantize, Score and harmonize
slug: material-color-utilities-score
domain: theming
kind: project
platform: cross
vendor: Google
years: 2021–present
status: current
tags: [quantization, wu, wsmeans, score, harmonize, wallpaper, extraction]
relevance: high
---

## What it is
The library behind Material You. It quantizes an image (Wu, then weighted-k-means "WSMeans" to at most 128 colours), then `Score` ranks the result for use as a theme source, and `Blend.harmonize` rotates a fixed colour toward the source.

## What was new
- Score weighs a colour's area proportion (hue-smoothed across neighbours ±15°) and its chroma distance from a target of 48, filters out colours with chroma < 5 or under 1 % of the image, and then picks candidates at least 15° (relaxing) apart.
- Harmonize moves a brand or status colour by at most 15° toward the source hue (half the distance), so red errors stay red while matching the theme.
- Pure functions, identical across languages, reproducible from a seed.

## What went wrong / limits
- Grey or near-monochrome wallpapers fall back to a default blue (`#4285F4`), surprising users who picked a grey image.
- Area-weighted ranking favours sky and background over the subject, so a small red boat rarely wins.
- Downsampling choices differ per platform, so the "same" wallpaper gives different seeds on Android and in third-party tools like matugen.

## Lessons for swaypplet
- Already taken: Score for the primary, and harmonize-style bounded rotation (12° for status, 15° for categorical) in §2.2.
- Keep the explicit "tint off" for a colourless wallpaper rather than MCU's silent default blue.
- Document the sampler's downscale size so a cache-miss re-sample is reproducible.

## Sources
- https://github.com/material-foundation/material-color-utilities/ — quantizer, score, blend modules [verified]
- https://deepwiki.com/material-foundation/material-color-utilities — module architecture [verified]
- Score constants (target chroma 48, cutoff 5, 1 % proportion, fallback #4285F4) from `score.ts` [memory]
