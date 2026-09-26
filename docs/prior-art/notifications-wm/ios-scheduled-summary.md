---
name: iOS scheduled notification summary
slug: ios-scheduled-summary
domain: notifications-wm
kind: feature
platform: iOS
vendor: Apple
years: 2021–present
status: current
tags: [batching, digest, schedule, deferral]
relevance: medium
---

## What it is
Opt-in in iOS 15: chosen apps stop alerting in real time and are delivered as a digest at times the user sets (for example 08:00 and 18:00). Time Sensitive and messages from people bypass it.

## What was new
Batching as a user-level policy per app, with the most relevant items ranked at the top of the digest (on-device ranking, before generative AI). It turns "notification" back into "mail", read at a moment the user chose.

## What went wrong / limits
Adoption is modest; users who most need it rarely configure it. Picking apps and times is up-front work. Some users missed time-bound items that apps did not mark Time Sensitive.

## Lessons for swaypplet
- Adapt: the catch-up card after a quiet period (roadmap item 3) is a digest triggered by context instead of by clock; reuse the same rendering for both if a scheduled digest is ever wanted.
- Avoid: a clock schedule as the first version; the context trigger needs no setup.

## Sources
- https://onesignal.com/blog/ios-notification-changes-updates-from-apples-wwdc-21/ — summary delivered at scheduled times. [verified via search summary]
- https://nirajpaul2.medium.com/big-changes-in-notification-ios-15-focus-mode-notification-summary-293f310574a8 — summary and Focus overview. [verified via search summary]
- Adoption and ranking details. [memory]
