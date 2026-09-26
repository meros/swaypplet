---
name: Android FrameTimeline
slug: android-frametimeline
domain: motion
kind: feature
platform: Android
vendor: Google
years: 2021–present
status: current
tags: [perfetto, jank-attribution, expected-vs-actual, surfaceflinger, tracing]
relevance: high
---

## What it is
Android 12 instrumentation, visible in Perfetto, that records for every frame an *expected* timeline (the deadline the scheduler predicted) and an *actual* one, for both the app and SurfaceFlinger.

## What was new
Jank is a mismatch between predicted and real present time, and it is attributed: AppDeadlineMissed, BufferStuffing (app queues frames faster than they present, raising latency), SurfaceFlingerCpu/GpuDeadlineMissed, DisplayHAL, PredictionError. Present type is classified early, on time or late. The blame lands on a layer of the stack, not on "the frame".

## What went wrong / limits
Requires Android 12 and a trace; it is a diagnosis tool, not a pass/fail gate by itself.

## Lessons for swaypplet
- Take: split the gate's "late" into client-late (swaypplet's frame work missed the deadline) and compositor-late (swayfx rendering with glass missed it). Only the patched swayfx can report the second; `wp_presentation` feedback gives the actual present time to the client.
- Take: watch for buffer stuffing: a client that commits faster than presentation adds a frame of latency without showing up as a dropped frame.

## Sources
- https://perfetto.dev/docs/data-sources/frametimeline — timelines, jank types, Android 12 [verified]
