---
name: Compiz (desktop cube, wobbly windows)
slug: compiz-cube-wobbly
domain: notifications-wm
kind: project
platform: other
vendor: Novell (David Reveman), Beryl community, Compiz Fusion
years: 2006–2016
status: discontinued
tags: [compositor, effects, plugins, cube, wobbly, linux]
relevance: medium
---

## What it is
An OpenGL compositing window manager for X11 (with Xgl, then AIGLX) famous for the rotating desktop cube for workspaces, wobbly windows, fire and water effects. Forked into Beryl in 2006, re-merged in 2007 as Compiz Fusion; Ubuntu's Unity ran on it until Unity's end.

## What was new
It proved Linux could composite before Vista shipped, and its Scale plugin (an Exposé clone) and zoom were real productivity features. A plugin architecture let the community add hundreds of effects.

## What went wrong / limits
Effects became the product: settings managers with hundreds of toggles, instability, driver bugs. The cube conveyed workspace adjacency but cost time on every switch. Maintenance fell away with Unity, and Wayland compositors replaced it.

## Lessons for swaypplet
- Take: wobbly windows lives on because it gives drag feedback; the effects that survive map to a user action.
- Avoid: a settings surface of effect toggles; one designed motion vocabulary (MOTION.md) instead.
- Note: swayfx is in Compiz's lineage; keep its effects (blur, corners, shadows) subordinate to legibility.

## Sources
- https://en.wikipedia.org/wiki/Compiz — cube, wobbly, merger, plugin architecture. [verified via search summary]
- https://linux.slashdot.org/story/07/04/02/1815214/a-look-at-the-compiz-and-beryl-merger — 2007 merger. [verified via search summary]
- Unity use, decline. [memory]
