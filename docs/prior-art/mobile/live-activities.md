---
name: Live Activities
slug: live-activities
domain: mobile
kind: feature
platform: iOS
vendor: Apple
years: 2022–present
status: current
tags: [ongoing-activity, lock-screen, push-updates, budget, stand-down]
relevance: high
---

## What it is
Since iOS 16.1, an ActivityKit API that puts a live, updating card for one ongoing event (a delivery, a ride, a game, a timer) on the lock screen and in the Dynamic Island. The app updates it locally or by push notifications that run against a per-hour budget.

## What was new
- **An ongoing state as its own object**, separate from notifications: it has a start, updates and an end, instead of a stream of pings.
- **A lifetime built into the API**: an activity may run for 8 hours before the system ends it. It then stays on the lock screen for up to 4 more hours (12 in all), or until the user dismisses it.
- **Update budget**: push updates are rate-limited per hour, with priority headers, so an app cannot turn the card into a ticker.
- Since iOS 18 the same object shows up on the Apple Watch Smart Stack and in CarPlay.

## What went wrong / limits
- Apps have to opt in per event. Many do not, so coverage is patchy.
- The budget and the 4 KB payload limit make frequent real-time data (sports) laggy.
- Some apps misuse it for promotion, and the user cannot filter by kind.

## Lessons for swaypplet
- **Take the lifetime rule**: every ongoing indicator gets a hard maximum and a defined end. This is BAR_VISION P10 ("every signal ships with its stand-down") written into an API. The Claude task board's "waiting" state could carry the same kind of cap.
- **Take the update budget**: a producer that exceeds N updates per minute gets coalesced. That is P7's cadence budget, applied to input.
- **Adapt the lock-screen row**: roadmap item 3's lock summary could show one ongoing activity (a timer, a running build) as well.

## Sources
- https://canopas.com/integrating-live-activity-and-dynamic-island-in-i-os-a-complete-guide — 8 h + 4 h lifetime, presentations [verified via search summary]
- https://developer.apple.com/documentation/activitykit — push budget, 4 KB payload [memory]
