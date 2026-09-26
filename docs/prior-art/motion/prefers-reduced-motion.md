---
name: prefers-reduced-motion and Reduce Motion
slug: prefers-reduced-motion
domain: motion
kind: feature
platform: cross
vendor: Apple (iOS 7, WebKit), W3C
years: 2013–present
status: current
tags: [accessibility, reduced-motion, vestibular, media-query, dissolve]
relevance: high
---

## What it is
Apple added "Reduce Motion" to iOS 7 (2013) after complaints that the new parallax and zoom transitions made people sick. WebKit exposed it to the web as the `prefers-reduced-motion` media query in 2017, now supported in all browsers, and mirrored by Windows "Animation effects", GNOME `enable-animations` and Android "Remove animations".

## What was new
WebKit's post named the triggers: scaling and zooming, spinning, multi-speed motion such as parallax, and moving along a simulated depth axis. It asked for *different* motion, not none: replace a zoom with a dissolve, keep elements near text still, stop spinning backgrounds. Apple's own implementation swaps zoom transitions for cross-fades.

## What went wrong / limits
- Many implementations read it as "animations off", which removes state cues (where did the panel come from?).
- It is binary; there is no "less" setting, so users with mild sensitivity must choose all or nothing.

## Lessons for swaypplet
- Take: under reduced motion, map spatial motion (slide, scale, the lock card's rise) to a 150 ms opacity dissolve, and keep one-frame collapse only for things that carry no information. The current one-frame collapse is safe for callbacks but drops the arrival cue.
- Take: audit the glass for multi-speed parallax (refraction moving against the backdrop while a card slides), which is a named trigger.

## Sources
- https://webkit.org/blog/7551/responsive-design-for-motion/ — trigger categories, alternatives, 2017 [verified]
- https://developer.mozilla.org/en-US/docs/Web/CSS/@media/prefers-reduced-motion — support [memory]
