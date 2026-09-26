---
name: RAIL performance model
slug: rail-performance-model
domain: motion
kind: pattern
platform: web
vendor: Google
years: 2015–present
status: current
tags: [budgets, response, animation, idle, 100ms, 10ms]
relevance: high
---

## What it is
A user-centred performance model from the Chrome team: Response, Animation, Idle, Load, each with a budget.

## What was new
Concrete budgets tied to perception: respond to input within 100 ms (process the event in 50 ms, leaving headroom), produce each animation frame in 10 ms of app work (the frame is 16 ms, the browser needs the rest), do idle work in chunks of at most 50 ms so input is never blocked longer.

## What went wrong / limits
The 16 ms figure assumes 60 Hz. Google later moved emphasis to Core Web Vitals (INP), and RAIL pages are no longer actively maintained.

## Lessons for swaypplet
- Take: split the frame into an app share and a compositor share. A work p95 gate of "10 ms at 60 Hz" (about 60 % of the interval) leaves swayfx room for the glass pass; say it as a fraction of the interval so it scales to 120 Hz.
- Take: the 50 ms idle chunk rule for anything swaypplet does on the main thread outside animation (icon loading, search indexing in the launcher).

## Sources
- https://web.dev/articles/rail — budgets for response, animation, idle, load [verified]
