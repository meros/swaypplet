---
name: Flutter Impeller
slug: flutter-impeller
domain: motion
kind: project
platform: cross
vendor: Google (Flutter)
years: 2022–present
status: current
tags: [shader-compilation-jank, precompiled-shaders, first-frame, renderer]
relevance: high
---

## What it is
Flutter's replacement renderer for Skia. It precompiles a small fixed set of shaders and pipeline states at engine build time instead of generating and compiling shaders at runtime. Default on iOS (only option), Android API 29+ with GL fallback, and desktop since Flutter 3.47.

## What was new
It targeted one specific jank class: the first time an effect is drawn, Skia had to compile its shader, costing tens to hundreds of ms on that frame. Flutter's earlier fix (SkSL warm-up captured by running the app) was fragile; Impeller removed runtime compilation by design.

## What went wrong / limits
Years of migration with rendering differences and regressions; some effects were slower than Skia at first. Web still uses Skia.

## Lessons for swaypplet
- Take: the liquid-glass shader and every GTK/GSK pipeline should be compiled and warmed at compositor or app start, not on the first glass surface. The gate's "first frame up to 270 ms" is where this jank lives.
- Take: add a cold-start gate case (fresh compositor, first panel open) separate from warm rounds.

## Sources
- https://docs.flutter.dev/perf/impeller — rationale, precompilation, per-platform defaults [verified]
