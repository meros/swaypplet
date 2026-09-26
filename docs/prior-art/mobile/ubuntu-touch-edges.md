---
name: Ubuntu Touch edge gestures (Unity 8, Lomiri)
slug: ubuntu-touch-edges
domain: mobile
kind: product
platform: other
vendor: Canonical, later UBports
years: 2013–present
status: niche
tags: [gestures, edges, launcher, convergence, linux, qml]
relevance: medium
---

## What it is
Ubuntu Touch (Canonical, 2013; community-run by UBports since 2017) gave each screen edge one job. Left edge: the launcher strip. Right edge: a short swipe goes to the previous app, a long one opens the app spread. Top: indicators pulled down one at a time. Bottom: app-defined controls. The shell (Unity 8, now Lomiri) was also meant to become the desktop (**convergence**).

## What was new
- **Every edge is a named function**, and the distance of the swipe chooses between two related actions (previous app vs all apps).
- **Indicators pulled individually**: dragging down on the network icon opens that indicator's menu, and sliding sideways switches to the next one without lifting.
- One shell for phone and desktop, adapting to input.

## What went wrong / limits
- Convergence was never finished; Canonical dropped Unity 8 and the phone in April 2017, citing no return on the investment.
- Four edges with two speeds each is a lot to learn; some gestures clash with in-app swipes.

## Lessons for swaypplet
- **Take "open one indicator, slide to its neighbour"**: clicking a bar segment opens its panel section, and left/right moves to the next section without closing. That fits the panel as one surface of sections.
- **Avoid distance-coded gestures** as the only way to reach something (P8).

## Sources
- https://en.wikipedia.org/wiki/Ubuntu_Touch — edges, 2017 withdrawal, UBports [verified via search summary]
- https://github.com/buildwclaude/edge-launcher — edge gesture model described [verified via search summary]
