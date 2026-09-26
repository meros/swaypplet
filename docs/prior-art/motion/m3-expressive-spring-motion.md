---
name: M3 Expressive motion physics system
slug: m3-expressive-spring-motion
domain: motion
kind: feature
platform: Android
vendor: Google
years: 2025–present
status: current
tags: [spring, motion-scheme, spatial, effects, damping, stiffness, tokens]
relevance: high
---

## What it is
Material 3 Expressive (2025) replaced easing-plus-duration with spring tokens in a `MotionScheme`. Each scheme has spatial springs (position, size, shape) and effects springs (colour, opacity), each at fast, default and slow.

## What was new
Two families with different rules, verified in androidx source:

| | spatial damping / stiffness | effects damping / stiffness |
|---|---|---|
| standard fast / default / slow | 0.9 / 1400, 700, 300 | 1.0 / 3800, 1600, 800 |
| expressive fast / default / slow | 0.6, 0.8, 0.8 / 800, 380, 200 | 1.0 / 3800, 1600, 800 |

Effects springs are always critically damped, because an opacity or colour that overshoots reads as a flicker. Spatial springs may overshoot. Designers choose "standard" or "expressive" per product, not per animation.

## What went wrong / limits
- Stiffness numbers do not map to milliseconds, so designers lose the direct "300 ms" handle; tooling has to show settle times.
- Overshoot in the expressive scheme (damping 0.6) is visible and polarising; Google restricts it to small, playful components.

## Lessons for swaypplet
- Take: split any spring adoption into spatial (may bounce, rarely) and effects (never bounce, damping 1.0). This matches swaypplet's rule that fade and slide share a clock only if both are non-overshooting.
- Take: one scheme switch for the whole shell, not per-surface tuning.

## Sources
- https://github.com/androidx/androidx/blob/androidx-main/compose/material3/material3/src/commonMain/kotlin/androidx/compose/material3/tokens/StandardMotionTokens.kt — standard spring values [verified]
- https://github.com/androidx/androidx/blob/androidx-main/compose/material3/material3/src/commonMain/kotlin/androidx/compose/material3/tokens/ExpressiveMotionTokens.kt — expressive spring values [verified]
- https://m3.material.io/blog/m3-expressive-motion-theming — spatial vs effects rationale [memory]
