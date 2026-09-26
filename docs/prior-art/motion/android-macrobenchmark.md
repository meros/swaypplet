---
name: Jetpack Macrobenchmark
slug: android-macrobenchmark
domain: motion
kind: project
platform: Android
vendor: Google
years: 2021–present
status: current
tags: [benchmark, ci, frame-timing, percentiles, regression-gate]
relevance: high
---

## What it is
A Jetpack library that drives a real app build through scripted interactions (startup, scrolling, animation) and collects metrics from system traces. `FrameTimingMetric` reports `frameDurationCpuMs` and `frameOverrunMs` at P50, P90, P95 and P99.

## What was new
- Frame metrics come out of a repeatable scripted journey with controlled compilation state (`CompilationMode`), so two builds compare like for like.
- Overrun is reported as a signed distribution, not a pass/fail count, which shows how close to the edge frames are.
- Designed for CI, with JSON output and trace files per iteration.

## What went wrong / limits
Google discourages emulators; numbers are only representative on real devices, which is costly in CI. Low battery and thermal throttling add noise.

## Lessons for swaypplet
- Take: add P90/P99 of work and of overrun to frame-bench, not only p50/p95/max. Max is one outlier; P99 over six rounds is steadier.
- Adapt: the headless sway backend is swaypplet's emulator. Keep gating relative differences on it and run an occasional real-hardware pass on the laptop for absolutes.

## Sources
- https://developer.android.com/topic/performance/benchmarking/macrobenchmark-overview — FrameTimingMetric, CompilationMode, CI, emulator caveat [verified]
