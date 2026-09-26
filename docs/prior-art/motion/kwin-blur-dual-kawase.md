---
name: KWin blur with dual Kawase
slug: kwin-blur-dual-kawase
domain: motion
kind: feature
platform: KDE
vendor: KDE
years: 2018–present
status: current
tags: [blur, dual-kawase, downsample, gpu-cost, noise, contrast]
relevance: high
---

## What it is
In January 2018 (KWin 5.13) Alex Nemeth replaced KWin's Gaussian-style blur with the dual Kawase method (D9848): downsample the backdrop through several half-resolution passes with a small Kawase kernel, then upsample back.

## What was new
Blur cost became nearly independent of radius: strength is chosen by the number of down/up iterations and offset, not by a wider kernel. The patch claimed "virtually infinite blur with very very little performance cost" and measured faster than the old blur at maximum strength. Strength levels were tabulated from min/max offsets per iteration instead of magic numbers. A later noise option masks banding.

## What went wrong / limits
- The blurred region must be expanded by the kernel's reach, and any change underneath forces the whole region to be re-blurred; damage tracking helps less.
- The result is not a true Gaussian; at some strengths it shows block or diamond artefacts that need noise to hide.

## Lessons for swaypplet
- Take: swaypplet's glass already keeps two frost levels (`tex_frost`, `tex_frost_wide`); producing them with dual Kawase down/up passes rather than wide Gaussian taps is the known cheap route.
- Take: frame-bench should run with the real glass settings, not `blur_passes 1, blur_radius 5`, or the compositor side of the gate measures a cheaper material than the one shipped.

## Sources
- https://phabricator.kde.org/D9848 — dual Kawase patch, performance claim, strength levels [verified]
