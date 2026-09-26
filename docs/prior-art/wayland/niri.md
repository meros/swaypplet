---
name: niri
slug: niri
domain: wayland
kind: product
platform: Wayland
vendor: Ivan Molodetskikh (YaLTeR) and contributors
years: 2023–present
status: current
tags: [compositor, scrollable-tiling, smithay, rust, overview, blur]
relevance: high
---

## What it is
A Rust compositor on Smithay with scrollable tiling: each workspace is an infinite horizontal strip of columns, new windows never resize existing ones, and workspaces stack vertically and dynamically per monitor. It became the default target of the Quickshell shells (DMS, Noctalia) in 2025–26.

## What was new
- Opening a window never shrinks others; the viewport scrolls instead.
- Overview (25.05): zooms out to all workspaces, keyboard navigation still works, drag windows between workspaces, hot corner and 4-finger swipe.
- 25.11 added an alt-tab switcher with window previews.
- 26.04 implemented `ext-background-effect-v1` with two blur modes: normal blur (samples the frame behind) and "x-ray" blur, which blurs the wallpaper once and reuses it as a static image behind every surface. Shells and terminals get blur with no compositor config.
- Every animation is spring- or curve-driven and interruptible, and gestures track the finger 1:1.

## What went wrong / limits
- Blur was the most requested feature for about two years; users waited for a protocol rather than a niri-specific hack.
- Scrollable layout confuses apps that assume a fixed screen size; floating windows arrived late.

## Lessons for swaypplet
- Take x-ray blur as a cheap tier: a pre-blurred wallpaper texture behind bar and panel costs almost nothing per frame and never smears moving windows.
- Take the overview interaction: zoom out from the real frame, keep keyboard focus semantics.
- Interruptible animations matter more than their curves.

## Sources
- https://www.phoronix.com/news/Niri-25.05-Released — overview [verified via search summary]
- https://www.phoronix.com/news/Niri-26.04-Released — blur via ext-background-effect, x-ray mode [verified via search summary]
- https://lwn.net/Articles/1025866/ — tour of niri [verified: exists]
