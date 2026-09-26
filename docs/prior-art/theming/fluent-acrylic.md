---
name: Fluent Acrylic
slug: fluent-acrylic
domain: theming
kind: feature
platform: Windows
vendor: Microsoft
years: 2017–present
status: current
tags: [material, blur, noise, luminosity, transient-surfaces]
relevance: high
---

## What it is
Fluent's frosted-glass material (Windows 10 Fall Creators Update, 2017). A recipe of backdrop blur, a luminosity blend, a colour tint layer and a noise texture. Since Windows 11 it is reserved for transient, light-dismiss surfaces (flyouts, menus).

## What was new
- A luminosity layer evens out the backdrop's lightness before tint, so text on acrylic varies less over busy content.
- Noise (grain) hides banding and makes the surface read as a material.
- Explicit placement rule: only transient surfaces get the live material; persistent ones get Mica.

## What went wrong / limits
- Windows 10 applied acrylic widely and it was GPU/battery costly; it disables on battery saver.
- `TintLuminosityOpacity` is derived from the tint, not the content, so busy backdrops still bleed through.

## Lessons for swaypplet
- swaypplet's glass already has grain and a photochromic term that plays the luminosity layer's role.
- Adapt the placement rule: live glass for transient surfaces (launcher, notifications, OSD), a cheaper Mica-like tint for long-lived ones, if GPU cost ever matters.

## Sources
- https://learn.microsoft.com/en-us/windows/apps/design/style/acrylic — recipe, placement rules [verified]
- https://en.wikipedia.org/wiki/Fluent_Design_System — history [verified via search summary]
