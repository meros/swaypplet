---
name: hyprlock and swaylock-effects
slug: lockers
domain: wayland
kind: product
platform: Wayland
vendor: hyprwm; mortie (swaylock-effects), later jirutka fork
years: 2020–present
status: current
tags: [lock-screen, blur, fade-in, grace-period, widgets]
relevance: medium
---

## What it is
swaylock-effects is a swaylock fork that adds a blurred screenshot background, vignette, fade-in, a clock in the indicator and a grace period in which any input unlocks without a password. hyprlock is a GPU-rendered locker for ext-session-lock with configurable widgets (labels, images, input field, shapes), blurred screencopy background and fade animations into and out of the lock.

## What was new
- Grace period: locking starts visibly but the first seconds are forgiving, so an accidental idle lock costs nothing.
- Fade-in from the live desktop (a screencopy) instead of a cut to black; hyprlock also fades out on unlock.
- hyprlock labels can run commands on an interval, making the lock a small dashboard.

## What went wrong / limits
- swaylock-effects went unmaintained and was forked; distributions carry different versions [memory].
- Screenshot backgrounds leak what was on screen into the lock surface (and into memory) by design; blur softens but does not hide large text.
- hyprlock widgets polling commands contradict a still lock screen.

## Lessons for swaypplet
- Take the grace period if not present: a few seconds after an idle lock in which motion unlocks.
- swaypplet's cross-fade already follows the fade-in idea; keep the blur heavy enough that text is unreadable.

## Sources
- https://github.com/hyprwm/hyprlock — features [verified via search summary]
- https://wiki.hypr.land/Hypr-Ecosystem/hyprlock/ — fade, blur, background [verified via search summary]
- https://github.com/mortie/swaylock-effects — options [memory]
