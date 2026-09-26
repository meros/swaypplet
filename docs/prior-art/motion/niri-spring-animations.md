---
name: niri springs and animation config
slug: niri-spring-animations
domain: motion
kind: feature
platform: Wayland
vendor: niri (Ivan Molodetskikh)
years: 2023–present
status: current
tags: [spring, gesture, slowdown, custom-shader, interruptible, scrollable-tiling]
relevance: high
---

## What it is
niri is a scrollable-tiling Wayland compositor built on Smithay (Rust). Every animation is either an easing (`duration-ms`, `curve`) or a spring (`damping-ratio`, `stiffness`, `epsilon`), and there is a global `slowdown` factor and a global `off`.

## What was new
- Defaults mix the two by role: workspace switch, view movement, window movement and resize are critically damped springs (damping 1.0, stiffness 800–1000); window open is 150 ms `ease-out-expo`; close 150 ms `ease-out-quad`.
- Springs are chosen where a touchpad gesture can hand over velocity; the docs note even damping 1.0 can overshoot when "launched" by a fast swipe.
- `epsilon` controls when the spring is declared settled; a too-large value makes the end "jump".
- Open/close animations can run a user GLSL shader, with fallback to the last good shader on compile failure.

## What went wrong / limits
Overdamped springs (above 1.0) have numerical stability issues and are not recommended. Shaders carry no compatibility guarantee.

## Lessons for swaypplet
- Take: the role split. Springs for things that move under a finger or get retargeted; curves for open/close. This is the smallest adoption of springs that swaypplet could make.
- Take: the settle `epsilon` bug class. Any spring in `anim.rs` needs a settle test: last frame's jump below a pixel.
- Take: `slowdown` as a debug knob.

## Sources
- https://github.com/YaLTeR/niri/blob/main/docs/wiki/Configuration:-Animations.md — defaults, spring parameters, warnings, custom shaders [verified]
