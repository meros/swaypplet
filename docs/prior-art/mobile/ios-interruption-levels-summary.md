---
name: Interruption levels and Notification Summary
slug: ios-interruption-levels-summary
domain: mobile
kind: feature
platform: iOS
vendor: Apple
years: 2021–present
status: current
tags: [notifications, priority, batching, digest, time-sensitive]
relevance: high
---

## What it is
Also in iOS 15: every notification carries one of four interruption levels, *passive* (no sound, no screen wake), *active* (the default), *time-sensitive* (may break through Focus and the summary if the user allows) and *critical* (bypasses the mute switch; needs an Apple entitlement). **Scheduled Summary** holds notifications from chosen apps and delivers them as one digest at set times of day.

## What was new
- **The sender declares urgency, and the user caps it**: an app says time-sensitive, and the user can revoke that per app.
- **Batching as a first-class delivery mode**: a digest at 08:00 and 18:00 in place of a trickle, ordered by relevance on the device.
- Passive as a real level: it is delivered to the list but never interrupts.

## What went wrong / limits
- Apps over-claim "time-sensitive" for marketing. iOS asks the user after a while whether to keep allowing it, which moves the policing to the user.
- The summary is used by few people; many never find it.
- iOS 18.1's AI-written summaries of notifications produced wrong headlines (the BBC complaint, December 2024), and Apple paused news summaries in iOS 18.3.

## Lessons for swaypplet
- **Take the four levels**: map the freedesktop urgency (low, normal, critical) plus one "time-sensitive" tier onto popup, list-only and breakthrough, and let the user demote per app.
- **Take the digest for item 3**: "7 while you were presenting" is a summary triggered by an event instead of a clock, which is the better trigger.
- **Avoid rewriting content**: a summary should count and group, never paraphrase.

## Sources
- https://documentation.onesignal.com/docs/en/ios-focus-modes-and-interruption-levels — four levels and their behaviour [verified via search summary]
- https://www.bbc.com/news/articles/cq5ggew08eyo — Apple pausing AI notification summaries after errors [memory]
