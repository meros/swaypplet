---
name: WCAG 2.3.3 Animation from Interactions and vestibular disorders
slug: wcag-animation-from-interactions
domain: motion
kind: pattern
platform: web
vendor: W3C WAI
years: 2018–present
status: current
tags: [accessibility, wcag, vestibular, motion-sickness, aaa]
relevance: medium
---

## What it is
WCAG 2.1 success criterion 2.3.3 (level AAA): motion animation triggered by interaction can be disabled unless it is essential. Related: 2.2.2 Pause, Stop, Hide (level A) for anything moving automatically for more than five seconds.

## What was new
It separates motion the user caused (scrolling, which they control) from motion added as decoration in response (parallax, zooms). It cites vestibular reactions including nausea, migraine and "potentially needing bed rest to recover", which makes motion an accessibility harm, not a taste question.

## What went wrong / limits
AAA, so rarely required; "essential" is subjective.

## Lessons for swaypplet
- Take: ambient loops (2 s breathing glow, 1.1 s ring spin) are the 2.2.2 case: auto-moving and indefinite. They need to stop under reduced motion, and should stop after a bound even without it.

## Sources
- https://www.w3.org/WAI/WCAG21/Understanding/animation-from-interactions.html — level, exceptions, vestibular impact [verified]
- https://www.w3.org/WAI/WCAG21/Understanding/pause-stop-hide.html — 5 s rule [memory]
