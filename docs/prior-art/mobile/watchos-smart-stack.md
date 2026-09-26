---
name: watchOS Smart Stack
slug: watchos-smart-stack
domain: mobile
kind: feature
platform: other
vendor: Apple
years: 2023–present
status: current
tags: [widgets, relevance, glanceable, wrist, context]
relevance: medium
---

## What it is
watchOS 10 (2023) made the widget stack the main glance surface: turn the Digital Crown up from any watch face and a scrolling stack of half-screen widgets appears, ordered by relevance (time, calendar, location), with pinning. watchOS 11 added Live Activities to the stack and suggestions from signals such as rain arriving or being on a trip.

## What was new
- **One gesture from anywhere to "what matters now"**, without leaving the face.
- **Relevance from time and place**, with pinning as the user's override.
- A special widget holding three circular complications, so fixed personal slots sit inside a ranked list.

## What went wrong / limits
- It replaced the side-button Dock and swiping between faces, and long-time users complained about lost muscle memory.
- Relevance guesses are sometimes wrong, and the stack reorders between glances.

## Lessons for swaypplet
- **Adapt "pinned on top, ranked below"**: if the panel ever surfaces suggestions, fixed user rows come first and anything ranked comes after, never interleaved.
- **Avoid moving a core gesture's target**: changing where a keybind leads costs more than the new surface earns.

## Sources
- https://developer.apple.com/videos/play/wwdc2023/10029/ — Smart Stack widgets, relevance [verified via search summary]
- https://www.macrumors.com/2024/06/13/watchos-11-live-activities-suggested-widgets/ — Live Activities and suggestions in watchOS 11 [verified via search summary]
