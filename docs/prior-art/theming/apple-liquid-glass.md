---
name: Apple Liquid Glass
slug: apple-liquid-glass
domain: theming
kind: feature
platform: cross
vendor: Apple
years: 2025–present
status: current
tags: [glass, refraction, lensing, material, legibility, adaptive]
relevance: high
---

## What it is
The material introduced at WWDC 2025 across iOS, iPadOS, macOS (Tahoe 26), watchOS and visionOS. Controls and navigation float on a refracting, specular glass layer above content, in Regular (adaptive) and Clear (transparent, needs dimming) variants.

## What was new
- Lensing (refraction at the edge) instead of scattering blur, specular highlights that respond to geometry and device motion.
- Regular glass adapts per pixel: shadow opacity rises over text, small elements flip light/dark with the backdrop and their glyphs follow; large surfaces deliberately do not flip.
- Thickness scales with element size.

## What went wrong / limits
- Widespread legibility complaints at launch (notifications, Control Centre, menu bar over bright content).
- iOS/macOS 26.1 added a Clear/Tinted switch; Tinted raises opacity. Reduce Transparency was partly broken in macOS 26.1–26.2 and fixed in 26.3.
- Consistency complaints: in-app and system surfaces use the glass differently.

## Lessons for swaypplet
- swaypplet's glass with photochromic ceiling/lift plus a tested APCA floor is already stricter than Apple's shipped behaviour; keep the test as a release gate.
- Take the Tinted lesson: a single user-facing "opaque–clear" axis (swaypplet's Clarity) is what users actually wanted; keep it prominent.
- Make sure the Reduce Transparency equivalent (`.no-glass` fallback) is tested on every surface; Apple's regression shows how easily it rots.

## Sources
- https://developer.apple.com/videos/play/wwdc2025/219/ — lensing, adaptive shadow, flipping [verified in ../../research/liquid-glass.md]
- https://osxdaily.com/2026/02/13/reduce-transparency-works-again-in-macos-tahoe-26-3/ — 26.3 fix [verified via search summary]
- https://www.tomsguide.com/computing/macos/you-can-reduce-liquid-glass-transparency-on-macos-tahoe-heres-how — Tinted option [verified via search summary]
- https://en.wikipedia.org/wiki/Liquid_Glass — reception [verified via search summary]
