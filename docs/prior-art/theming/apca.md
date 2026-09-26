---
name: APCA (Accessible Perceptual Contrast Algorithm)
slug: apca
domain: theming
kind: pattern
platform: cross
vendor: Myndex Research (Andrew Somers)
years: 2019–present
status: research
tags: [contrast, accessibility, wcag3, lc, polarity, font-size]
relevance: high
---

## What it is
A contrast model reporting lightness contrast Lc (roughly −108 to 106) between text and background, polarity-aware (dark-on-light and light-on-dark differ) and paired with font-size/weight lookup tables (e.g. Lc 75 for body text, 60 for content text, 45 for large/bold, 30 for non-text). Proposed for WCAG 3.

## What was new
- Fixes WCAG 2's known faults: WCAG 2 overstates contrast of dark colour pairs (so dark mode "passes" while illegible) and understates white text on mid-tone colours (orange buttons).
- Ties the threshold to size and weight rather than one ratio.

## What went wrong / limits
- Removed from the WCAG 3 working draft in July 2023 for lack of working-group consensus; the WCAG 3 contrast method is undecided as of 2026. It is not a legal standard, and abandoning WCAG 2 for it carries compliance risk.
- Licensing and governance of the reference implementation were contested.

## Lessons for swaypplet
- swaypplet is not bound by web compliance, so APCA is the right model for a dark-mode shell. Keep it, but also compute WCAG 2 ratios in the test report so a claim of "accessible" can be stated in both terms.
- The size tables justify swaypplet's `--accent` "15 px bold and up" rule; encode the size in the test rather than in prose.

## Sources
- http://adrianroselli.com/2026/04/wcag3-contrast-as-of-april-2026.html — status as of April 2026 [verified via search summary]
- https://yatil.net/blog/wcag-3-is-not-ready-yet — WCAG 3 status [verified via search summary]
- https://github.com/Myndex/SAPC-APCA — algorithm and Lc tables [memory]
