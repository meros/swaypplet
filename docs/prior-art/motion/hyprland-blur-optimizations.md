---
name: Hyprland blur, xray and new_optimizations
slug: hyprland-blur-optimizations
domain: motion
kind: feature
platform: Wayland
vendor: Hyprland
years: 2022–present
status: current
tags: [blur, kawase, xray, cache, vfr, damage, glass]
relevance: high
---

## What it is
Hyprland's window and layer blur is Kawase-based with `size` and `passes`, plus `noise`, `contrast`, `brightness` and `vibrancy` modulation. Newer builds add blur variants, including glass-like ones with refraction.

## What was new
- `new_optimizations` (default on) caches the blurred wallpaper-plus-tiled-layers once and reuses it behind floating windows, instead of re-blurring per window per frame.
- `xray` goes further: floating windows blur only the wallpaper layer, ignoring tiled windows, trading correctness for a large cost cut.
- `vfr` (default on) renders only when damage exists; the docs call it "heavily recommended" to save resources, and ask users to disable it for accurate perf overlay numbers.
- `blur.popups` and `blur.special` are off by default and marked expensive.

## What went wrong / limits
xray's output is visibly wrong when a window overlaps another; users accept it for speed. Cache invalidation on any change under the blur means a video behind a blurred bar still re-blurs every frame.

## Lessons for swaypplet
- Take: an xray-like mode for static namespaces (bar, dock): blur only the wallpaper plus a cached frost, rebuilt on wallpaper change. Worth an experiment on battery.
- Take: an idle gate. With VFR, an idle shell should produce zero compositor frames; frame-bench can assert that nothing redraws when nothing moves.

## Sources
- https://github.com/hyprwm/hyprland-wiki/blob/main/content/configuring/core/config-options.md — blur options, xray, new_optimizations, vfr, vrr [verified]
