---
name: BlackBerry 10 Peek and Flow
slug: bb10-peek
domain: mobile
kind: pattern
platform: other
vendor: BlackBerry
years: 2013–2017
status: failed
tags: [gesture, peek, preview, cancelable, flow, no-buttons]
relevance: high
---

## What it is
BB10 had no home button. A short swipe up from the bottom bezel shrank the app a little to show notification indicators; continuing to the right slid the app aside to reveal the Hub; lifting the finger back where it started returned to the app untouched. BlackBerry called the model **Flow**: move through the OS instead of opening and closing apps.

## What was new
- **Peek**: look without leaving. The gesture is reversible until release, so checking messages costs no context.
- **The gesture is the navigation**: continuous, one-handed, with no mode.
- A deliberate first-run tutorial that taught the gestures.

## What went wrong / limits
- It needed a mandatory tutorial and was still hard to learn at first, reviewers said.
- The platform failed for ecosystem reasons (see [bb10-hub](bb10-hub.md)), so Peek went with it.
- Invisible gestures have no affordance; users who skip the tutorial never find them.

## Lessons for swaypplet
- **Take peek-while-held**: holding a key shows the notification centre or task board over the current window, and releasing closes it with no change to focus. That is a keyboard version of Peek, and it respects P8 (no hover).
- **Avoid gesture-only affordances**: every peek needs a visible, clickable equivalent.

## Sources
- https://crackberry.com/glance-and-peek-two-key-gestures-delivering-flow-experience-blackberry-10 — Peek and Flow [verified via search summary]
- https://www.digitaltrends.com/phones/blackberry-10-review/ — tutorial, learning curve [verified via search summary]
