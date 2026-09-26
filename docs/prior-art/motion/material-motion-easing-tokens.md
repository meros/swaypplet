---
name: Material motion easing and duration tokens
slug: material-motion-easing-tokens
domain: motion
kind: pattern
platform: Android
vendor: Google
years: 2014–present
status: current
tags: [easing, duration, tokens, cubic-bezier, emphasized, design-system]
relevance: high
---

## What it is
Material Design (2014) and Material 3 (2021) define motion as named easing curves plus a duration ladder. M3's standard set is `standard` (0.2, 0, 0, 1), `standard-decelerate` (0, 0, 0, 1) and `standard-accelerate` (0.3, 0, 1, 1), plus "emphasized" variants; durations run short1–4 (50–200 ms), medium1–4 (250–400), long1–4 (450–600) and extra-long1–4 (700–1000).

## What was new
Motion became a token system on par with colour and type: a component references `motion.duration.medium2` and `motion.easing.emphasized`, not a literal. Enter decelerates, exit accelerates, and exits are shorter than entrances.

## What went wrong / limits
- Sixteen duration tokens is more choice than most teams use consistently; apps still drift into near-duplicates.
- Duration-based curves cannot preserve velocity on interruption, which is part of why M3 Expressive moved spatial motion to springs.

## Lessons for swaypplet
- Take (already taken): swaypplet's five tiers and three curves are a subset of this, which is the right size. Keep the census check.
- Adapt: when a spring is introduced, keep the curve tokens for opacity/colour and CSS, as M3 now does for effects.

## Sources
- https://github.com/androidx/androidx/blob/androidx-main/compose/material3/material3/src/commonMain/kotlin/androidx/compose/material3/tokens/MotionTokens.kt — duration token values 50–1000 ms [verified]
- https://m3.material.io/styles/motion/easing-and-duration/tokens-specs — easing curve values [memory]
