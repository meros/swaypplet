---
name: CarPlay Dashboard
slug: carplay-dashboard
domain: mobile
kind: feature
platform: iOS
vendor: Apple
years: 2019–present
status: current
tags: [glanceable, split-view, driver-distraction, fixed-layout, priority]
relevance: medium
---

## What it is
iOS 13 (2019) added the Dashboard to CarPlay: a split screen with the map in the biggest pane and small cards for media, the next calendar event, the current call and Siri suggestions. iOS 13.4 opened the map pane to third-party apps. Android Auto answered with its own split screen in 2021 (version 6.1, wide screens only).

## What was new
- **Fixed regions by role**: the map always big, secondary cards always in the same place, so the driver learns where to look.
- **Built for a glance**: large targets, a few words, no scrolling. Car guidelines budget glances at about 2 seconds each (the NHTSA guidelines: 2 s per glance, 12 s total per task).
- Cards swap by context (a call replaces media while it is active) without changing the layout.

## What went wrong / limits
- Few choices: users cannot change which cards appear, and complained until later versions.
- Next-generation CarPlay (announced 2022), which takes over the whole instrument cluster, has rolled out very slowly: the first cars, from Aston Martin, arrived in 2025 as "CarPlay Ultra" [memory].

## Lessons for swaypplet
- **Take fixed regions with contextual content**: exactly P4 ("slots never move; relevance controls prominence inside them"). CarPlay shows that model works under the hardest attention limit.
- **Adapt the glance budget as a review test**: every ambient mark must be read in under about 2 seconds.

## Sources
- https://www.autoevolution.com/news/apples-carplay-dashboard-is-a-feature-android-auto-needs-too-155332.html — Dashboard cards, map pane [verified via search summary]
- https://appleinsider.com/articles/20/03/26/ios-134-introduces-carplay-dashboard-support-for-third-party-map-apps — third-party maps in 13.4 [verified via search summary]
- https://www.nhtsa.gov/sites/nhtsa.gov/files/distraction_npfg-02162012.pdf — 2 s / 12 s glance criteria [memory]
