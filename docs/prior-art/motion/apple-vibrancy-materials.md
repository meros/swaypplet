---
name: Apple materials and vibrancy
slug: apple-vibrancy-materials
domain: motion
kind: feature
platform: macOS
vendor: Apple
years: 2013–present
status: current
tags: [blur, vibrancy, materials, reduce-transparency, liquid-glass, legibility]
relevance: high
---

## What it is
iOS 7 (2013) and OS X Yosemite (2014) introduced system blur materials (`UIVisualEffectView`, `NSVisualEffectView`), with "vibrancy" that blends foreground content with the blurred backdrop so labels pick up colour. Materials come in named thicknesses (ultra-thin to thick, chrome). iOS/macOS 26 (2025) replaced them with Liquid Glass, which refracts and adapts to content.

## What was new
Materials are semantic, not parameters: an app asks for `.regularMaterial` and the system chooses blur radius, saturation and tint per appearance and accessibility setting. The effect is rendered by the system compositor, so apps cannot make it expensive by misuse. "Reduce Transparency" swaps every material for an opaque fill, and "Increase Contrast" darkens it.

## What went wrong / limits
- iOS 7's blur was disabled on older devices (iPhone 4) for cost, giving two different looks of the same OS.
- Liquid Glass launched with legibility complaints; iOS 26.1 added a Clear/Tinted toggle.
- Vibrancy's colour blending makes contrast hard to guarantee.

## Lessons for swaypplet
- Take (in place): named materials per namespace, rendered once in the compositor, is the swaypplet architecture.
- Take: "Reduce Transparency" as a user setting mapping every glass namespace to its solid twin, reused by battery mode.

## Sources
- https://developer.apple.com/documentation/uikit/uivisualeffectview — materials and vibrancy [memory]
- https://developer.apple.com/videos/play/wwdc2025/219/ — Liquid Glass adaptation (see research/adaptive-glass.md) [verified by sibling doc]
- https://www.macrumors.com/how-to/ios-26-1-reduce-liquid-glass-effects/ — Clear/Tinted toggle [verified by sibling doc]
