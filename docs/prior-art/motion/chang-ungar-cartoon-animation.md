---
name: "Animation: from cartoons to the user interface"
slug: chang-ungar-cartoon-animation
domain: motion
kind: pattern
platform: other
vendor: Bay-Wei Chang and David Ungar (Sun, Self project)
years: 1993
status: research
tags: [animation-principles, solidity, exaggeration, reinforcement, self]
relevance: medium
---

## What it is
A UIST 1993 paper applying Disney's animation principles to the Self programming environment's UI: objects move instead of teleporting, with slow-in/slow-out, anticipation, follow-through and motion blur.

## What was new
It framed UI motion around three goals: *solidity* (objects persist and never jump), *exaggeration* (make the change easy to see) and *reinforcement* (motion confirms what happened). Motion blur and "arcs" were used to keep perceived continuity at low frame rates.

## What went wrong / limits
Early-90s hardware limited it to a research system; exaggeration taken literally produces the showy motion later criticised in Compiz-era desktops.

## Lessons for swaypplet
- Take: "solidity" as a gate property: across an interruption (open during close) no surface should jump more than a few pixels between consecutive frames. That is measurable and catches restart-from-zero bugs.

## Sources
- https://dl.acm.org/doi/10.1145/168642.168647 — Chang and Ungar, UIST 1993 [memory]
