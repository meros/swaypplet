---
name: Qt Quick scene graph and Animators
slug: qt-quick-scene-graph
domain: motion
kind: feature
platform: KDE
vendor: The Qt Company
years: 2012–present
status: current
tags: [scene-graph, render-thread, animator, batching, vsync, rhi]
relevance: medium
---

## What it is
Qt Quick 2 renders QML through a retained scene graph. In the threaded render loop a render thread draws while the GUI thread prepares the next frame, with a short synchronisation phase (`updatePaintNode`) between them. Qt 6 renders through RHI over GL, Vulkan, Metal or D3D.

## What was new
- `Animator` types (`OpacityAnimator`, `XAnimator`, `ScaleAnimator`) run on the render thread, so they keep going while the GUI thread is busy, like Core Animation or Chrome's compositor.
- Animation is driven by vsync in the threaded loop, falling back to a timer when vsync throttling fails.
- Batching merges draw calls by material (one call for all backgrounds, one for icons, one for text).

## What went wrong / limits
The basic single-threaded loop, used on some drivers, gives up both benefits. Plasma Shell stutters historically came from GUI-thread JavaScript in QML, which Animators cannot help.

## Lessons for swaypplet
- Adapt: GTK has no Animator equivalent. swaypplet's compositor-side alpha and slide are the closest thing; an Animator-style contract (commit the whole animation, not per-frame values) is what would make them immune to main-thread stalls.

## Sources
- https://doc.qt.io/qt-6/qtquick-visualcanvas-scenegraph.html — threaded loop, sync phase, Animators, batching [verified]
