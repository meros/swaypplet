---
name: KWin effects and global animation speed
slug: kwin-animation-speed
domain: motion
kind: feature
platform: KDE
vendor: KDE
years: 2008–present
status: current
tags: [effects, animation-speed, slider, scripted-effects, instant]
relevance: medium
---

## What it is
KWin's compositing effects (fade, slide, magic lamp, wobbly windows, overview) all read one global "Animation speed" slider in System Settings, from "Very slow" to "Instant". Effects can also be written in JavaScript (scripted effects) or QML.

## What was new
A single user-facing multiplier that every effect honours, with "Instant" as a first-class end of the scale rather than a separate accessibility toggle. Scripted effects let the community add animations without C++.

## What went wrong / limits
- Effect quality varies widely because every effect picks its own curves; there is no shared token set.
- "Instant" disables motion entirely rather than substituting a dissolve, which is the opposite of what vestibular guidance recommends for state changes that need a cue.

## Lessons for swaypplet
- Take: a global speed multiplier over the whole ladder is cheap once durations go through `anim::duration`, as they do.
- Adapt: at the reduced-motion end, swap spatial motion for a short cross-fade instead of skipping it, so arrival is still signalled.

## Sources
- https://userbase.kde.org/Desktop_Effects_Performance — effects and animation speed [memory]
- https://develop.kde.org/docs/plasma/kwin/ — scripted effects [memory]
