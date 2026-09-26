---
name: Core Animation and the render server
slug: core-animation-render-server
domain: motion
kind: feature
platform: macOS
vendor: Apple
years: 2007–present
status: current
tags: [compositor, render-server, implicit-animation, layer-tree, off-main-thread]
relevance: high
---

## What it is
Core Animation (macOS 10.5, iPhone OS 1) is a retained layer tree. The app commits layer property changes and animation descriptions in a transaction; a separate render server process (`backboardd` on iOS, WindowServer on macOS) interpolates and composites them every frame.

## What was new
Animations are *data*, not per-frame callbacks. Once `opacity` or `position` animation is committed, the app's main thread can block and the animation still runs at display rate, because the out-of-process server evaluates the curve. Implicit animations (changing a layer property animates it by default, 0.25 s) made motion the default rather than extra work.

## What went wrong / limits
- Anything the server cannot interpolate (layout, text, custom drawing) falls back to the app's main thread, and a hitch there still shows up as a skipped commit. Apple's hitch guidance splits "commit hitches" from "render hitches" for that reason.
- Offscreen passes (masks, shadows without a `shadowPath`, group opacity) are the classic cost; Instruments' "Color Offscreen-Rendered" exists because of them.

## Lessons for swaypplet
- Adapt: GTK4 has no out-of-process animation evaluator. swaypplet already gets the nearest equivalent for layer surfaces: `anim::Reveal` hands surface alpha and slide to the patched swayfx through the alpha modifier. But the per-frame values still come from swaypplet's tick, so a stalled main thread still stalls the motion. Committing (from, to, curve, start, duration) once and letting swayfx evaluate it would be the full Core Animation model.
- Take: separate "commit" cost (style, layout, snapshot) from "render" cost (compositor GPU) in the frame gate; frame-bench already measures the first, not the second.

## Sources
- https://developer.apple.com/documentation/quartzcore — Core Animation layer and transaction model [memory]
- https://developer.apple.com/videos/play/tech-talks/10855/ — "Explore UI animation hitches and the render loop", commit vs render hitches [memory]
