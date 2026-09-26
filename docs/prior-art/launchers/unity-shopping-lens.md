---
name: Unity Dash shopping lens
slug: unity-shopping-lens
domain: launchers
kind: feature
platform: other
vendor: Canonical
years: 2012–2014
status: failed
tags: [privacy, amazon, online results, trust, default-on]
relevance: medium
---

## What it is
Ubuntu 12.10 (2012) sent every Dash search, including searches for local files, to Canonical's servers and showed Amazon product results among them by default.

## What was new
Nothing to copy. It is the canonical case of a local launcher leaking queries.

## What went wrong / limits
- EFF and Richard Stallman called it spyware; users found product ads next to their files. Canonical added an opt-out, and in 14.04 (2014) online scopes were turned off by default.
- The trust damage outlived the feature and fed the move away from Unity.

## Lessons for swaypplet
- Avoid: no query leaves the machine unless the user picked a row that says so ("Search the web for ...").

## Sources
- https://www.eff.org/deeplinks/2012/10/privacy-ubuntu-1210-full-disclosure — EFF critique [memory]
- https://en.wikipedia.org/wiki/Unity_(user_interface) — scopes, 14.04 default [memory]
