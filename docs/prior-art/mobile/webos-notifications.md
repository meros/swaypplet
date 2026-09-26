---
name: webOS notification bar
slug: webos-notifications
domain: mobile
kind: feature
platform: other
vendor: Palm, HP
years: 2009–2012
status: discontinued
tags: [notifications, non-modal, ambient, stack, dismiss-gesture]
relevance: medium
---

## What it is
In webOS a new notification appeared as a thin banner that scrolled across the bottom of the screen, then shrank to a small icon in a notification strip. The app kept the whole screen. Tapping the strip opened a "dashboard" of stacked notifications, each swiped away sideways. It arrived when iOS still showed modal alert boxes (until iOS 5, 2011).

## What was new
- **Non-modal from day one**: a notification never took focus or blocked input.
- **Two stages**: a transient banner, then a persistent small mark, then detail on demand.
- Swipe to dismiss, later copied everywhere.

## What went wrong / limits
- The strip took screen height from apps on a 3.1-inch screen.
- Many stacked icons became unreadable; there was no grouping.

## Lessons for swaypplet
- **Take the three stages** (transient, then a small persistent mark, then detail on demand). This is BAR_VISION P5's two-layer contract, and webOS proves it on a smaller screen.
- It is also evidence against modal alerts: nothing in the shell should take keyboard focus unless asked.

## Sources
- https://en.wikipedia.org/wiki/WebOS — notification system [memory]
- https://en.wikipedia.org/wiki/IOS_5 — iOS modal alerts replaced by Notification Center in 2011 [memory]
