---
name: Compiz and Beryl
slug: compiz-effects
domain: motion
kind: product
platform: other
vendor: Novell, Beryl community, Canonical
years: 2006–2017
status: discontinued
tags: [x11, compositor, wobbly-windows, cube, plugins, effects]
relevance: medium
---

## What it is
The first widely used OpenGL compositing window manager on Linux (2006, on AIGLX/Xgl). Known for the desktop cube, wobbly windows, fire and burn effects; Beryl was a fork that merged back as Compiz Fusion. Ubuntu's Unity ran on it until 2017.

## What was new
It proved a Linux desktop could composite with GL in 2006, before Windows Vista shipped Aero. Wobbly windows was an actual spring-mass mesh simulation on the window, years before springs were mainstream in UI.

## What went wrong / limits
- Hundreds of plugins with conflicting settings; the showcase effects were novelty rather than information, and the project's reputation stuck to them.
- Stability and driver problems, single-developer maintenance late in life, and Unity's end took it with it. Mutter and KWin absorbed the useful parts.

## Lessons for swaypplet
- Avoid: motion as spectacle. Every animation should say where something came from or went, which MOTION.md's roles already enforce.
- Take: a physical model (spring mesh) for a deliberate, rare delight is fine when it is interruptible and off by default.

## Sources
- https://en.wikipedia.org/wiki/Compiz — history, Beryl merge, Unity [memory]
