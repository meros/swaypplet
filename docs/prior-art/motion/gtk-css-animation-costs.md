---
name: GTK4 CSS transitions and tick callbacks
slug: gtk-css-animation-costs
domain: motion
kind: pattern
platform: GNOME
vendor: GNOME (GTK)
years: 2014–present
status: current
tags: [gtk4, css, transitions, frame-clock, tick-callback, box-shadow, snapshot]
relevance: high
---

## What it is
GTK animates CSS `transition` and `@keyframes` on the widget's `GdkFrameClock`; custom motion uses `gtk_widget_add_tick_callback`, and libadwaita wraps it in `AdwTimedAnimation` and `AdwSpringAnimation` (with velocity and damping ratio). All run on the main thread; each frame restyles, possibly relayouts, re-snapshots and re-renders.

## What was new
libadwaita's `AdwSpringAnimation` brought critically damped, velocity-aware springs to GTK (swipe navigation, carousels) and respects the global `gtk-enable-animations` setting.

## What went wrong / limits
- No off-main-thread path: a long idle callback stalls every animation in the process.
- Cost depends on the property: animated `box-shadow` re-rasterises a differently sized blur every frame; `filter: blur()` renders offscreen; transitions on `min-width`, padding or margin force layout of the whole subtree. Opacity and `transform` are cheapest because they only change the node at the top.
- The CSS engine has no `linear()` easing and no springs, so a Rust spring and a CSS transition cannot share a curve beyond cubic-bezier.

## Lessons for swaypplet
- Take (in place): swaypplet's box-shadow keyframes were already moved to border and fill. Make that a census rule.
- Take: libadwaita's `AdwSpringAnimation` is the reference for a GTK-native spring with an initial velocity, even if swaypplet keeps its own `anim.rs`.
- Take: honour `gtk-enable-animations` as well as swaypplet's own reduced-motion setting.

## Sources
- https://gnome.pages.gitlab.gnome.org/libadwaita/doc/main/class.SpringAnimation.html — spring animation API, damping ratio, initial velocity [memory]
- https://docs.gtk.org/gtk4/method.Widget.add_tick_callback.html — tick callbacks on the frame clock [memory]
- ../../MOTION.md — box-shadow keyframes rationale [verified]
