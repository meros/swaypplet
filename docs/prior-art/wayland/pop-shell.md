---
name: Pop Shell
slug: pop-shell
domain: wayland
kind: product
platform: GNOME
vendor: System76
years: 2020–2027 (maintenance ends with Pop!_OS 22.04 EOL)
status: discontinued
tags: [gnome-extension, auto-tiling, launcher, keyboard]
relevance: low
---

## What it is
System76's GNOME Shell extension adding keyboard-driven auto-tiling (toggle per desktop), a launcher with search plugins, and focus navigation with vim keys. Replaced by COSMIC's compositor-level tiling; System76 no longer adapts it to new GNOME versions and support ends with 22.04 in April 2027.

## What was new
- Auto-tiling as a toggle on a normal desktop, aimed at non-tiling users; floating exceptions per app.
- A launcher with plugin backends (calculator, files, windows) that later became pop-launcher and COSMIC's launcher.

## What went wrong / limits
- The vendor abandoned it for its own desktop; community ports for GNOME 46+ exist on a branch with minimal QA.

## Lessons for swaypplet
- Take: a launcher backend as a separate service with plugins (pop-launcher) is reusable across frontends; swaypplet's elephant integration is the same idea.

## Sources
- https://github.com/pop-os/shell/discussions/1728 — "Pop Shell Future" [verified via search summary]
