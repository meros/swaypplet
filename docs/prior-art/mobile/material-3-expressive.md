---
name: Material 3 Expressive
slug: material-3-expressive
domain: mobile
kind: pattern
platform: Android
vendor: Google
years: 2025–present
status: current
tags: [motion, springs, shape, emphasis, research, design-language]
relevance: medium
---

## What it is
The 2025 update to Material Design, shipped with Android 16 QPR1 and Wear OS 6. It brings shape morphing, bolder type emphasis, new components (button groups, a floating toolbar, loading indicators) and a motion system built on physics springs in place of duration-and-easing curves.

## What was new
- **Motion as springs**: spatial springs (which may overshoot) for position and size, and effects springs (no overshoot) for colour and opacity, each in a fast, default and slow tier. An interrupted animation keeps its velocity instead of restarting a curve.
- Backed by 46 studies with more than 18,000 participants. Google reports that participants found key UI elements up to four times faster on the expressive screens.
- Emphasis through size and shape before colour: a pressed button changes shape, and the important action is the bigger one.

## What went wrong / limits
- Critics called it playful and inconsistent next to productivity work; the loud shapes and bounce are a matter of taste.
- The "4× faster" figure is Google's own, from its own studies, and has no independent replication.
- A spring has no fixed duration, which makes frame budgets and filmstrip tests harder to state than with a fixed ms token.

## Lessons for swaypplet
- **Adapt the split between spatial and effects motion**: overshoot is allowed on position and never on colour or opacity. That fits on top of the existing `move`/`travel`/`page` tokens as a rule, not a new system.
- **Avoid springs as the token primitive**: swaypplet's tokens name durations with meanings, and `dev/frame-bench.sh --gate` needs a bounded duration. Keep durations and allow a spring-shaped easing inside them.
- "Size and weight before colour" (design-system principle 3) matches M3E's own research result.

## Sources
- https://design.google/library/expressive-material-design-google-research — 46 studies, 18,000+ participants, 4× finding [verified via search summary]
- https://m3.material.io/blog/building-with-m3-expressive — springs replace easing and duration [verified via search summary]
- https://m3.material.io/styles/motion/overview — spatial vs effects springs [memory]
