---
name: SwiftUI springs as the default animation
slug: swiftui-spring-animations
domain: motion
kind: feature
platform: iOS
vendor: Apple
years: 2019–present
status: current
tags: [spring, bounce, duration, interruptible, velocity, retargeting]
relevance: high
---

## What it is
At WWDC23 ("Animate with springs") Apple made springs the default animation in SwiftUI: a bare `withAnimation` now uses a spring. Springs are specified by two designer-facing numbers, `duration` and `bounce`, instead of mass, stiffness and damping.

## What was new
- `bounce` runs from -1 to 1: 0 is a smooth critically damped curve, positive overshoots, negative is over-damped. Presets: `.smooth` (bounce 0), `.snappy` (about 0.15), `.bouncy` (about 0.3). Apple advises care above 0.4.
- "Duration" is the perceptual duration, not the settling time, so a spring can be swapped into a place that used to take a duration token.
- On retargeting, the spring keeps the velocity it had and heads for the new target. Gesture end hands its velocity to the animation the same way.

## What went wrong / limits
- A spring's tail is long; "done" callbacks fire later than the perceptual duration, which surprises code that chains on completion.
- Springs cannot be expressed in plain CSS cubic-bezier, so web and SwiftUI motion diverge unless sampled into `linear()`.

## Lessons for swaypplet
- Take: parameterise any spring in swaypplet as (duration, bounce) so it maps onto the existing 150/200/300/400/500 ms tokens; keep bounce 0 for everything but a deliberate emphasis.
- Take: `Reveal::animate` already continues from the current alpha and offset when a hide interrupts a show, so position is continuous; velocity is not (a decelerating entrance at speed becomes an accelerating exit from rest). Velocity-preserving retargeting closes that last discontinuity.
- Avoid: chaining state changes on spring completion; key them off the perceptual duration.

## Sources
- https://developer.apple.com/videos/play/wwdc2023/10158/ — duration/bounce, presets, velocity on retarget, springs as default [verified]
