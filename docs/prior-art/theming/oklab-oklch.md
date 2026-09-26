---
name: Oklab and OKLCH
slug: oklab-oklch
domain: theming
kind: pattern
platform: cross
vendor: Björn Ottosson
years: 2020–present
status: current
tags: [colour-space, perceptual, css-color-4, gamut, interpolation]
relevance: high
---

## What it is
A perceptual colour space published in December 2020 as a blog post, fitted to perception data with a simple closed-form transform from linear sRGB. OKLCH is its polar form (lightness, chroma, hue). Both are in CSS Color 4 (`oklab()`, `oklch()`) and supported in all major browsers.

## What was new
- Hue stays stable as lightness and chroma change (fixes CIELAB's blue-to-purple shift), with a cheap, invertible transform.
- Adopted fast: Photoshop gradients, Unity, Godot, CSS, Tailwind v4, and palette tools.
- Ottosson's companion posts on gamut mapping (preserve lightness, reduce chroma) and Okhsl/Okhsv pickers.

## What went wrong / limits
- Lightness L is not exactly L* or APCA lightness, so equal L steps do not give equal contrast.
- Gamut cusp is irregular: max chroma at a given L varies strongly by hue (yellow vs blue), so a fixed-chroma scale clips.

## Lessons for swaypplet
- swaypplet's generator is in OKLCH and chroma yields to gamut; this is Ottosson's recommended mapping. Keep it.
- Consider Okhsl for any future colour picker in settings, since it keeps lightness perceptual in a familiar HSL shape.

## Sources
- https://bottosson.github.io/posts/oklab/ — the original post [verified via search summary]
- https://en.wikipedia.org/wiki/Oklab_color_space — dates, CSS adoption [verified via search summary]
- https://www.smashingmagazine.com/2024/10/interview-bjorn-ottosson-creator-oklab-color-space/ — adoption list [verified via search summary]
