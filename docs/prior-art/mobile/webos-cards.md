---
name: Palm webOS cards
slug: webos-cards
domain: mobile
kind: pattern
platform: other
vendor: Palm, later HP and LG
years: 2009–2012 (phones); LG TVs since 2014
status: discontinued
tags: [multitasking, cards, gestures, stacks, spatial-model, switcher]
relevance: high
---

## What it is
The multitasking model of Palm's webOS (Palm Pre, January 2009), designed under Matías Duarte. Each running app is a card; pressing the gesture area zooms the current app out into a row of cards you swipe through, tap to enter, or flick up to close. webOS 2.0 (2010) added **stacks**: cards spawned from one another (an email and the link it opened) grouped together.

## What was new
- **Running apps as physical objects**: zoom out, see them side by side, throw one away. Flick-up-to-close became the multitasking gesture on iOS (7) and Android (9, P).
- **The same object in both states**: the card in the switcher *is* the app, scaled down live, so there is no break between "app" and "switcher".
- **Stacks**: grouping by origin, not by app. A task's windows travel together.

## What went wrong / limits
- The UI was not what failed: the Pre's hardware was weak and the webOS apps (HTML/JS) were slow. HP bought Palm in 2010 for 1.2 billion dollars and dropped the TouchPad after 49 days in August 2011.
- Many live cards on 2009 hardware meant memory pressure, and a "too many cards" error.

## Lessons for swaypplet
- **Take "the switcher tile is the thing"**: roadmap item 1's workspace-tile-to-workspace morph is webOS's zoom in and out. The overview (item 4) should start from a real capture, which is exactly what made webOS feel continuous.
- **Adapt stacks**: grouping windows by task rather than app matches the task-workspace model (task colours on the bar).
- Flick to close in the switcher is cheap and well understood; offer it next to the keyboard close.

## Sources
- https://www.howtogeek.com/finally-tried-webos-inspired-android-still-might-be-better/ — cards, flick up to close, influence [verified via search summary]
- https://en.wikipedia.org/wiki/Mat%C3%ADas_Duarte — Duarte's role at Palm [verified via search summary]
- https://en.wikipedia.org/wiki/HP_TouchPad — discontinued after 49 days [memory]
