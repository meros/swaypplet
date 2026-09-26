---
name: Fluent motion timing and easing
slug: fluent-motion-timing
domain: motion
kind: pattern
platform: Windows
vendor: Microsoft
years: 2017–present
status: current
tags: [easing, duration, connected-animation, tokens, WinUI]
relevance: medium
---

## What it is
Fluent Design's motion guidance for WinUI. It names three durations (`ControlFasterAnimationDuration` 83 ms, `ControlFastAnimationDuration` 167 ms, `ControlNormalAnimationDuration` 250 ms) and two extreme easings: fast-out-slow-in `cubic-bezier(0, 0, 0, 1)` for entering, slow-out-fast-in `cubic-bezier(1, 0, 1, 1)` for exiting.

## What was new
The durations are multiples of a 60 Hz frame (5, 10 and 15 frames), so every animation ends on a frame boundary at 60 Hz. The easings are deliberately extreme: an entering object "traveled from a long distance away" so it reads as fast even after a moment of unresponsiveness.

## What went wrong / limits
Frame-multiple durations lose their meaning at 120 or 144 Hz. The 250 ms ceiling is short for large surfaces; Windows 11 shell animations are longer than the control tokens.

## Lessons for swaypplet
- Adapt: the "hide latency with a fast-arriving curve" argument supports swaypplet's decelerate entrance: if a surface's first frame is late, a curve that starts at full speed hides it better than one that eases in.
- Avoid: a fully vertical accelerate curve like (1, 0, 1, 1); swaypplet's (0.3, 0, 1, 1) keeps a visible start.

## Sources
- https://learn.microsoft.com/en-us/windows/apps/design/motion/timing-and-easing — durations and easing curves [verified]
