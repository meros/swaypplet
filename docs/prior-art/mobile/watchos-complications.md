---
name: watchOS complications
slug: watchos-complications
domain: mobile
kind: pattern
platform: other
vendor: Apple
years: 2015–present
status: current
tags: [glanceable, fixed-slots, families, budget, watch-face]
relevance: medium
---

## What it is
Small fixed slots on an Apple Watch face (circular, corner, rectangular, inline families) that show one datum from an app: the next event, the temperature, activity rings. Apps supply data for each family; the face owner chooses which app fills which slot.

## What was new
- **Fixed slots with families**: the face defines the shapes, and every app must render in them. Layout is never at the mercy of data.
- **Budgeted updates**: a complication has a daily update budget (around 50 background refreshes) and a timeline, so a glance costs no radio time.
- **Tinted mode**: faces can force every complication into one tint, so third-party data cannot break the colour scheme.

## What went wrong / limits
- Space is tiny; many complications are just a launcher icon.
- Update budgets make some data stale, and developers struggled with the timeline model until WidgetKit replaced ClockKit (watchOS 9).

## Lessons for swaypplet
- **Take fixed slots with typed shapes**: the bar's segments are complications. Defining a small set of families (icon, icon plus value, short text) keeps P4 and P5 enforceable.
- **Take forced tint**: the watch's tinted mode is the same guarantee as "status colour is only for status". A source never chooses its own colour.

## Sources
- https://developer.apple.com/documentation/clockkit — complication families, timelines (deprecated for WidgetKit in watchOS 9) [memory]
- https://developer.apple.com/documentation/widgetkit/creating-accessory-widgets-and-watch-complications — accessory families, tinted rendering [memory]
