---
name: picom (compton) X11 compositor
slug: picom
domain: motion
kind: project
platform: other
vendor: compton / yshui and contributors
years: 2011–present
status: niche
tags: [x11, compositor, fading, dual-kawase, vsync, unredirect, damage]
relevance: low
---

## What it is
A standalone X11 compositor, forked from xcompmgr via compton, used with tiling window managers (i3, bspwm) for shadows, fading, transparency and blur. Backends: xrender, glx, egl.

## What was new
- Fading is step-based: `fade-in-step` 0.03 opacity every `fade-delta` 10 ms, so the fade's duration depends on the timer, not on a curve.
- Blur methods include box, Gaussian and a `dual_kawase` method (added in forks around 2019, then upstreamed), which is what made strong blur usable on X11 laptops.
- `use-damage`, `unredir-if-possible` for fullscreen, and an experimental backend rewrite; recent versions add a scripted animation system.

## What went wrong / limits
Step-and-timer fading is not vsync-aligned, so fades stutter at non-60 Hz. Unredirect toggling flickers. Long periods of experimental backends with different bugs.

## Lessons for swaypplet
- Avoid: timer-driven animation. Every step must come from the frame clock with a timestamp, never a `timeout_add`.

## Sources
- https://github.com/yshui/picom/blob/next/picom.sample.conf — fade step/delta, backends, use-damage, unredir [verified]
- https://github.com/yshui/picom — dual_kawase and animations history [memory]
