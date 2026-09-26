---
name: JankStats
slug: android-jankstats
domain: motion
kind: project
platform: Android
vendor: Google
years: 2022–present
status: current
tags: [jank, frame-metrics, heuristics, state-annotation, production-telemetry]
relevance: high
---

## What it is
A Jetpack library that reports per-frame timing from real devices, built on `FrameMetrics` (API 24+) with a fallback for older releases. Each frame reports duration, whether it counts as jank, and the app states active at the time.

## What was new
- Jank is defined relative to the refresh rate: by default a frame that takes more than 2x the refresh interval (`jankHeuristicMultiplier`), so the same rule works at 60, 90 and 120 Hz.
- `PerformanceMetricsState` lets code tag frames with what was happening ("Scrolling", "Activity=Settings"), so a janky frame arrives with its cause attached.
- API 31+ adds `frameOverrunNanos`, how far past the deadline a frame landed.

## What went wrong / limits
The 2x default is lenient; a single missed vsync at 60 Hz (33 ms) is at the edge. It measures the app's UI and render threads, not SurfaceFlinger or the display.

## Lessons for swaypplet
- Take: tag each frame in `frame_stats.rs` with the active animation (`reveal:panel`, `slot:notif`), so frame-bench can report late frames per surface instead of per run.
- Take: express "late" as a multiple of the refresh interval, and report overrun in ms as a continuous number next to the count.

## Sources
- https://developer.android.com/topic/performance/jankstats — heuristics, state API, API levels [verified]
