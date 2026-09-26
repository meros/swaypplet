---
name: Android Choreographer and RenderThread
slug: android-choreographer-renderthread
domain: motion
kind: feature
platform: Android
vendor: Google
years: 2012–present
status: current
tags: [vsync, frame-clock, render-thread, project-butter, display-list]
relevance: high
---

## What it is
Project Butter (Android 4.1, 2012) introduced `Choreographer`, which aligns input, animation and traversal to vsync, plus triple buffering. Android 5.0 (2014) added `RenderThread`: the UI thread records display lists and a separate thread issues the GL/Vulkan commands.

## What was new
- One vsync-driven frame clock with ordered phases: input, animation, traversal (measure, layout, draw), commit. Animations read one frame timestamp so everything moving in a frame agrees.
- `RenderThread` lets property animations of `alpha`, `translation` and circular reveal run as `RenderNodeAnimator`s on the render thread while the UI thread is blocked.

## What went wrong / limits
Most jank still comes from the UI thread (inflation, layout, GC) exceeding its share of the frame. Triple buffering smoothed throughput but added latency, which later drove FrameTimeline's "buffer stuffing" category.

## Lessons for swaypplet
- Take: GTK4's `GdkFrameClock` has the same phase ordering. Every swaypplet animation should read the frame clock's frame time rather than wall time, so two surfaces animated in one frame agree. `SlideBin::slide_to` currently stamps its start with `glib::monotonic_time()`; worth checking it against the frame time.
- Adapt: the render-thread idea maps to the compositor. swaypplet already offloads layer-surface alpha and slide to swayfx; the remaining step is letting swayfx evaluate the curve itself.

## Sources
- https://developer.android.com/reference/android/view/Choreographer — vsync-driven frame callback [memory]
- https://source.android.com/docs/core/graphics — BufferQueue, triple buffering, SurfaceFlinger [memory]
