---
name: Wayfire
slug: wayfire
domain: wayland
kind: product
platform: Wayland
vendor: Ilia Bozhinov (ammen99) and contributors
years: 2018–present
status: niche
tags: [compositor, compiz, plugins, wobbly, expo, cube]
relevance: low
---

## What it is
A wlroots compositor modelled on Compiz: nearly everything (wobbly windows, cube, expo, scale, fire animations, blur, tiling) is a plugin, configured through an INI file or the WCM GUI. It ships its own wf-shell panel and dock. Raspberry Pi OS used it as its Wayland default from 2023 until moving to labwc in 2024 [memory].

## What was new
- Rendering plugins in a wlroots compositor years before Hyprland plugins, including an expo grid and a scale (all windows) overview.
- Live config reload per plugin option.

## What went wrong / limits
- Plugin API tied to internals, so third-party plugins rot across versions.
- Slow release cadence and a small team; Raspberry Pi left it for the lighter labwc, citing performance on low-end hardware [memory].

## Lessons for swaypplet
- Effects that users love (expo, scale) are overviews, not wobble; spend motion budget there.
- Avoid: an effects-first compositor lost its biggest downstream on performance.

## Sources
- https://github.com/WayfireWM/wayfire — plugins list [memory]
- https://www.raspberrypi.com/news/a-new-release-of-raspberry-pi-os/ — switch to labwc, Oct 2024 [memory]
