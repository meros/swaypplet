---
name: labwc
slug: labwc
domain: wayland
kind: product
platform: Wayland
vendor: Johan Malm and contributors
years: 2020–present
status: current
tags: [compositor, stacking, openbox, wlroots, protocols]
relevance: low
---

## What it is
A stacking wlroots compositor inspired by Openbox, reading Openbox-style `rc.xml`, menus and themes. It does no panel itself and relies on protocols (layer-shell, foreign-toplevel, ext-workspace, output management) so any Wayland panel works. Raspberry Pi OS's default since late 2024 [memory].

## What was new
- An "integration" page that states which protocol each external tool needs, making the compositor a protocol hub rather than a platform.
- Moved quickly to `ext-workspace-v1`, prompting Waybar to port its workspace module from the wlr draft to ext.

## What went wrong / limits
- No effects; the look is 2005 unless a panel provides it.

## Lessons for swaypplet
- Write down, per surface, which protocol it relies on and what happens when it is missing; labwc's integration page is a good format for swaypplet's docs.

## Sources
- https://labwc.github.io/integration.html — protocol integration list [verified via search summary]
- https://github.com/Alexays/Waybar/pull/4016 — ext-workspace port for labwc [verified via search summary]
