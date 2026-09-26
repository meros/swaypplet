---
name: Chrome compositor thread and compositor-only properties
slug: chrome-compositor-thread
domain: motion
kind: feature
platform: web
vendor: Google
years: 2012–present
status: current
tags: [compositor, layers, transform, opacity, off-main-thread, will-change]
relevance: high
---

## What it is
Chromium splits rendering into a main thread (style, layout, paint recording) and a compositor thread (cc) that owns a layer tree, sends tiles to raster threads and produces frames. CSS animations and transitions of `transform` and `opacity` (and, later, `filter` in some cases) run entirely on the compositor.

## What was new
A page whose main thread is stuck in JavaScript can still scroll and run transform/opacity animations at full rate, because the compositor only repositions already-rastered layers. "Animate only transform and opacity" became the most repeated web performance rule.

## What went wrong / limits
- Promoted layers cost GPU memory; `will-change` everywhere ("layer explosion") makes things slower.
- Animating anything that affects layout or paint (`width`, `box-shadow`, `top`) falls back to the main thread every frame.

## Lessons for swaypplet
- Adapt: GTK4 has no compositor thread, so every CSS transition is main-thread. The cheap properties are still the same ones: `opacity` and `transform` avoid relayout; `box-shadow`, `filter: blur()`, `min-height` and padding transitions force re-snapshot and re-raster. MOTION.md already moved two keyframes off box-shadow for this reason.
- Take: extend `dev/motion-census.sh` to flag transitions on layout- or shadow-affecting properties, the way it already flags off-ladder durations.

## Sources
- https://developer.chrome.com/blog/inside-browser-part3 — main vs compositor thread, layers, raster [verified]
- https://web.dev/articles/stick-to-compositor-only-properties-and-manage-layer-count — compositor-only properties, layer cost [memory]
