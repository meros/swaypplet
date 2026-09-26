---
name: Response time limits (0.1 / 1 / 10 s)
slug: response-time-limits
domain: motion
kind: pattern
platform: other
vendor: Miller; Card, Robertson, Mackinlay; Nielsen
years: 1968–1993
status: research
tags: [perception, latency, 100ms, instantaneous, feedback]
relevance: high
---

## What it is
Three limits from Miller (1968) and Card et al. (1991), popularised by Jakob Nielsen in 1993: 0.1 s feels instantaneous; 1 s keeps the flow of thought; 10 s is the limit of attention.

## What was new
It set the 100 ms bar for direct manipulation that RAIL, Chrome and most UI guidance still cite. Nielsen also noted interfaces can be too fast to follow, which is the argument for transitions at all.

## What went wrong / limits
100 ms is the threshold for *causality*, not for smoothness or for pointer-following: dragging or typing latency is noticeable far below it (see low-latency-touch-research). Treating 100 ms as "good enough" for continuous interaction is a known misuse.

## Lessons for swaypplet
- Take: key press to first changed frame under 100 ms for every surface (launcher, OSD, panel). The entrance animation can take 300 ms; the first frame cannot.
- Take: gate the first-frame time, which frame-bench already records, against 100 ms minus a refresh interval on real hardware.

## Sources
- https://www.nngroup.com/articles/response-times-3-important-limits/ — the three limits and their origin [verified]
