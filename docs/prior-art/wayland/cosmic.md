---
name: COSMIC (cosmic-comp, cosmic-panel, applets)
slug: cosmic
domain: wayland
kind: product
platform: Wayland
vendor: System76
years: 2021–present (Epoch 1 shipped Dec 2025)
status: current
tags: [desktop, rust, smithay, iced, autotiling, applets, protocols]
relevance: medium
---

## What it is
System76's Rust desktop: cosmic-comp (Smithay compositor) with per-workspace autotiling that can be toggled on and off, libcosmic (on iced) for every app, and cosmic-panel, which hosts each applet as its own Wayland client process. Epoch 1 shipped with Pop!_OS 24.04; point releases (1.1 to 1.4 by mid-2026) added tiling exceptions and a system monitor.

## What was new
- Tiling and floating in one desktop, switched per workspace with one toggle, rather than as an extension on top of a stacking shell.
- Applets as separate processes embedded by the panel: a crashing applet does not take the panel down.
- Its engineers drove several `ext-*` protocols upstream (workspace, image capture, foreign-toplevel-list) instead of keeping private ones [memory].

## What went wrong / limits
- Took four years from announcement to Epoch 1; Pop Shell users sat on an unmaintained GNOME extension meanwhile.
- iced's rendering and text stack gave early releases input latency and font issues [memory].

## Lessons for swaypplet
- Adapt: per-workspace tiling policy is a good fit for task workspaces (tile the task, float scratch).
- Avoid the applet-per-process cost for a single-owner shell; swaypplet's in-process services are simpler and fine.

## Sources
- https://github.com/pop-os/cosmic-epoch/releases/tag/epoch-1.1.0 — 1.1 compositor changes [verified via search summary]
- https://github.com/pop-os/cosmic-epoch — project [verified via search summary]
