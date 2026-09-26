---
name: iOS home-screen widgets and Smart Stack
slug: ios-widgets-smart-stack
domain: mobile
kind: feature
platform: iOS
vendor: Apple
years: 2020–present
status: current
tags: [widgets, glanceable, relevance, timeline, stack]
relevance: medium
---

## What it is
iOS 14 (2020) moved widgets from the Today view onto the home screen in fixed sizes (small, medium, large). A **Smart Stack** holds several widgets in one slot and rotates to the one it judges relevant (time of day, habits, app-donated relevance). Widgets render from a **timeline** of snapshots the app provides in advance; they are not live views. iOS 17 made them interactive (buttons and toggles through App Intents).

## What was new
- **Timeline rendering**: the app hands the system a list of (date, view) entries, and the system swaps them without waking the app. Battery cost is bounded by design.
- **Relevance ranking** of widgets in a stack, with the user able to turn rotation off.
- Fixed size classes keep the grid tidy.

## What went wrong / limits
- Smart Stack rotation means a thing is not where you left it; many users turn "Smart Rotate" off.
- Timeline snapshots go stale; widgets are often minutes out of date.
- Interactivity was late (iOS 17) and limited to intents.

## Lessons for swaypplet
- **Take timeline rendering as a cadence rule**: a source hands over future states with timestamps (the clock, a countdown), and the shell swaps them on a boundary-aimed timer. That is P7 made into an API.
- **Avoid content that moves on its own**: rotating stacks break "slots never move" (P4). Relevance may raise *prominence* inside a fixed slot, never swap what the slot is.

## Sources
- https://appleinsider.com/articles/20/06/24/in-depth-with-widgets-app-library-more-on-ios-14-home-screen — widgets on home screen, Smart Stack [verified via search summary]
- https://developer.apple.com/documentation/widgetkit/keeping-a-widget-up-to-date — timelines, reload budget [memory]
