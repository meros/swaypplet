---
name: macOS/iOS Reduce Transparency and Increase Contrast
slug: macos-reduce-transparency-increase-contrast
domain: theming
kind: feature
platform: macOS
vendor: Apple
years: 2013–present
status: current
tags: [accessibility, contrast, transparency, preference, fallback]
relevance: high
---

## What it is
Two accessibility switches. Reduce Transparency replaces blurred materials with opaque fills. Increase Contrast darkens borders, adds outlines to controls and raises text contrast; apps read it as `accessibilityDisplayShouldIncreaseContrast` and asset catalogs carry "High Contrast" colour variants.

## What was new
- Colour assets have four appearances (light, dark, light high-contrast, dark high-contrast) resolved by the system, so apps get the right value with no code.
- The web gets them as `prefers-reduced-transparency` and `prefers-contrast: more`.
- Materials know their own opaque fallback, so the switch changes material, not layout.

## What went wrong / limits
- Third-party apps that hard-code colours ignore both.
- Liquid Glass regressions (26.1–26.2) left transparency on in places with the switch enabled.

## Lessons for swaypplet
- swaypplet has `contrast: high` and honours `prefers-contrast: more`; add a reduce-transparency input that selects the `.no-glass` solid surface globally, so the fallback is a user choice and not only a missing-compositor path.
- Publish both to the XDG portal (`contrast`) so GTK apps follow the shell.

## Sources
- https://developer.apple.com/documentation/appkit/nsworkspace/accessibilitydisplayshouldincreasecontrast — API [memory]
- https://osxdaily.com/2025/10/10/stop-overlapping-text-legibility-issues-on-macos-tahoe-with-reduce-transparency/ — use against Liquid Glass [verified via search summary]
- High-contrast asset catalog appearances [memory]
