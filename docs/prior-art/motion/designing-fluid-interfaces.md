---
name: Designing Fluid Interfaces (WWDC18)
slug: designing-fluid-interfaces
domain: motion
kind: pattern
platform: iOS
vendor: Apple
years: 2018
status: current
tags: [interruptible, velocity-handoff, momentum-projection, gesture, spring, UIViewPropertyAnimator]
relevance: high
---

## What it is
A WWDC18 talk by the iPhone X gesture team setting out the principles behind the home-indicator gestures, backed by `UIViewPropertyAnimator` (iOS 10), which made UIKit animations pausable, scrubbable and reversible.

## What was new
- Every animation is interruptible and redirectable at any frame; the user never waits for one to finish.
- Gesture velocity is handed to the animation, and the landing point is *projected*: `current + v² / (2·decel)` using the scroll-view deceleration rate, so a flick lands where momentum would carry it.
- Springs are specified as damping (100 % for tool-like, 80–90 % when momentum is involved) and "response" instead of duration.
- Intent is detected from acceleration, not timers, so a pause in the multitasking gesture is recognised as fast as physics allows.

## What went wrong / limits
Nothing notable in the principles. In practice `UIViewPropertyAnimator` state handling (paused, stopped, finished) is error-prone.

## Lessons for swaypplet
- Take: any swipe or drag surface (notification dismissal, peek, a future touchpad gesture) should project the landing point from release velocity instead of a distance threshold.
- Take: 100 % damping as the default; underdamped only when a gesture launched it.

## Sources
- https://developer.apple.com/videos/play/wwdc2018/803/ — interruptibility, projection formula, damping guidance [verified]
