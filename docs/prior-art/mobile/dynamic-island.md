---
name: Dynamic Island
slug: dynamic-island
domain: mobile
kind: feature
platform: iOS
vendor: Apple
years: 2022–present
status: current
tags: [status, morph, shared-element, ongoing-activity, cutout, pill]
relevance: high
---

## What it is
Introduced with the iPhone 14 Pro in September 2022. The OLED panel draws black around two sensor cutouts to make one pill, and that pill grows, splits and shrinks to carry system alerts (Face ID, AirPods, silent switch) and ongoing activities (a timer, a call, navigation, music).

## What was new
- **A hardware flaw turned into a status surface**: the area was dead pixels anyway.
- **Three sizes of one element**: *compact* (leading and trailing halves around the cutout), *minimal* (a detached circle when two activities compete) and *expanded* (on a long press). The system chooses; the app supplies all three.
- **Shared-element motion**: an app that goes to the background (a call, audio) visibly shrinks *into* the island, so the user sees where the thing went. The morph uses spring motion with no hard cut.
- Tap opens the app; a long press expands in place.

## What went wrong / limits
- Minimal view shows only two activities; a third is dropped.
- Much of the island's content is decorative: the idle music waveform animates continuously for a state that needs no action.
- Third-party activities often fill the compact view with low-value marketing (sports scores, delivery upsells).

## Lessons for swaypplet
- **Take "shrink into where it lives"**: roadmap item 1 (shared-element motion) is this exact idea. A dismissed popup should travel to the notification centre, and a background media session into its bar segment.
- **Adapt the three-sizes contract**: a bar segment designed as compact, minimal and expanded variants, with the shell (not the source) deciding which, fits P4's fixed slots and the one centre decision slot.
- **Avoid the idle waveform**: continuous decoration violates P1/P2. Take the island's onset transition, not its looping content.

## Sources
- https://9to5mac.com/2022/09/08/iphone-14-pro-how-dynamic-island-works/ — two cutouts drawn as one pill [verified via search summary]
- https://www.bgr.com/tech/interview-iphone-14-pro-dynamic-island/ — Apple design interview [verified via search summary]
- https://canopas.com/integrating-live-activity-and-dynamic-island-in-i-os-a-complete-guide — compact/minimal/expanded [verified via search summary]
