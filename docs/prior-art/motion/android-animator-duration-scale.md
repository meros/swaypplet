---
name: Android animator duration scale
slug: android-animator-duration-scale
domain: motion
kind: feature
platform: Android
vendor: Google
years: 2011–present
status: current
tags: [animation-scale, developer-options, accessibility, remove-animations, debugging]
relevance: high
---

## What it is
Three global multipliers in Developer options (window, transition, animator duration scale: off, 0.5x to 10x) and an accessibility "Remove animations" toggle that sets them to 0. Apps read the result through `ValueAnimator.areAnimatorsEnabled()` and a duration-scale listener.

## What was new
A single system-wide knob that stretches every animation, which makes motion bugs visible (a 10x slow-motion exit shows a mismatched curve instantly) and gives users a way out of motion without per-app settings.

## What went wrong / limits
- Scale 0 skips animations entirely, and apps that chain logic on animation end callbacks break or hang; Espresso tests are advised to turn animations off, which hides those bugs.
- Buried in developer options; power users set 0.5x for speed, which ships as an accessibility setting by accident.

## Lessons for swaypplet
- Take (partly in place): swaypplet collapses reduced motion to one frame, not zero, precisely to avoid the callback trap. Keep it, and add a user-visible slowdown factor (like niri's `slowdown`) for debugging.
- Take: a frame-bench mode at 10x scale that filmstrips each transition would turn curve mismatches like the old 89 %/0.51 exit into a visible diff.

## Sources
- https://developer.android.com/reference/android/animation/ValueAnimator — areAnimatorsEnabled, DurationScaleChangeListener [verified, partial]
- https://developer.android.com/studio/debug/dev-options — animation scale settings [memory]
