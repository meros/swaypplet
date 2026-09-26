---
name: Pop and Rebound physics animation libraries
slug: facebook-pop-rebound
domain: motion
kind: project
platform: cross
vendor: Facebook
years: 2013–2017
status: discontinued
tags: [spring, physics, dynamic-animation, open-source, Paper]
relevance: medium
---

## What it is
Pop (iOS) and Rebound (Android, Java) were open-source spring physics engines Facebook built for the Paper app (2014). They animated any property with a spring or decay integrator driven by the display link.

## What was new
They brought "dynamic" animation, with no fixed duration, to mainstream app code years before UIKit and SwiftUI adopted springs. Retargeting a running spring kept its velocity for free because the integrator state *is* position plus velocity. Rebound's Origami-style tension/friction parameters matched Facebook's prototyping tool so designers' numbers transferred directly.

## What went wrong / limits
Both are archived. Integrating per frame on the main thread gave up Core Animation's off-thread evaluation, so a busy main thread stalled them. Platform springs (UIKit 2014, SwiftUI, Jetpack `SpringAnimation`) replaced them.

## Lessons for swaypplet
- Adapt: a spring in `anim.rs` is a few lines (semi-implicit Euler or the closed-form damped solution), keeps position and velocity as state, and gives retargeting for free.
- Avoid: fixed-step integration tied to frame count; step on the frame clock's timestamp so a dropped frame does not slow the motion.

## Sources
- https://github.com/facebookarchive/pop — archived repository [memory]
- https://github.com/facebookarchive/rebound — archived Java spring library [memory]
