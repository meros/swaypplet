---
name: Low-latency direct-touch research
slug: low-latency-touch-research
domain: motion
kind: pattern
platform: other
vendor: Microsoft Research (Ng, Lepinski, Wigdor, Sanders, Dietz)
years: 2012–2014
status: research
tags: [latency, perception, dragging, touch, 1ms, jnd]
relevance: medium
---

## What it is
Microsoft Research built a 1 ms latency touch prototype (UIST 2012, "Designing for low-latency direct-touch input") and measured when people notice latency while dragging and tapping. A follow-up (CHI 2014, Jota et al.) separated tapping and dragging thresholds.

## What was new
Users could distinguish latency far below the 100 ms folk limit: for dragging, just-noticeable differences were in the low single-digit ms for some participants, and median thresholds were around 2–11 ms in the follow-up work, against 50–200 ms on commercial tablets of the time. Tapping tolerated much more (tens of ms).

## What went wrong / limits
Lab hardware; exact thresholds vary by study and task. Figures here are from memory and should be checked before quoting.

## Lessons for swaypplet
- Take: anything that follows the pointer or a finger (drag to reorder, a future touchpad gesture) must update in the same frame as the input event, with no animation smoothing; smoothing is for the release, not the drag.

## Sources
- https://www.microsoft.com/en-us/research/video/applied-sciences-group-high-performance-touch/ — 1 ms touch demo [memory]
- https://dl.acm.org/doi/10.1145/2380116.2380174 — Ng et al., UIST 2012 [memory]
