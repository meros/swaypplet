---
name: Liquid Glass (iOS 26 and siblings)
slug: liquid-glass-ios26
domain: mobile
kind: feature
platform: iOS
vendor: Apple
years: 2025–present
status: current
tags: [glass, material, translucency, lensing, legibility, accessibility, design-language]
relevance: high
---

## What it is
Apple's cross-platform material, announced at WWDC on 9 June 2025 and shipped in iOS 26, iPadOS 26, macOS Tahoe, watchOS 26, tvOS 26 and visionOS 26. Controls, tab bars and sheets are drawn as a refracting, reflecting glass layer that floats above content, reshapes as it moves and adapts its tint to light and dark.

## What was new
- Glass as a functional layer: navigation and controls float over content instead of sitting in opaque bars, and content scrolls underneath them.
- Optical rather than blur-only: lensing (refraction at the edge), specular highlights that react to device motion, and an adaptive tint that moves with what is behind it.
- One material on every Apple platform, so a control looks the same on a watch and a Mac.

## What went wrong / limits
- Reception was mixed to negative on legibility, above all over busy backdrops and in sunlight. Apple raised the opacity of bars during the betas.
- iOS 26.1 added a Clear/Tinted choice (Settings > Display & Brightness > Liquid Glass). Tinted is more opaque and has more contrast. Reduce Transparency still works as a hard fallback.
- According to Wikipedia, at WWDC 2026 iOS 27 lowered the default transparency and added a slider between clear and tinted. Apple shipped the maximum-effect version first and backed off afterwards.
- Glass on glass (controls on sheets on content) stacks the problem. Apple's own guidance is to keep glass to the navigation layer.

## Lessons for swaypplet
- **Take the lesson, not the look.** Contrast has to be a hard floor that the material gives way to. swaypplet's rule "one glass layer per surface; nothing on it is glass" and the §5 contrast tests are the exact defence Apple added later.
- **Ship a tinted/opaque dial from day one**, bound to `contrast: high` and `prefers-contrast`, rather than as an accessibility afterthought.
- The busyness term in `research/adaptive-glass.md` is the right answer to Apple's legibility problem over a busy backdrop. Liquid Glass is evidence that the problem is real.

## Sources
- https://en.wikipedia.org/wiki/Liquid_Glass — dates, platforms, criticism, iOS 27 slider [verified]
- https://www.macrumors.com/how-to/ios-reduce-transparency-liquid-glass-effect/ — Clear/Tinted in 26.1, Reduce Transparency [verified via search summary]
- https://developer.apple.com/videos/play/wwdc2025/219/ — "Meet Liquid Glass" session, lensing and layer guidance [memory]
