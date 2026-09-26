---
name: Fabric and Ignis (Python shell frameworks)
slug: python-shell-frameworks
domain: wayland
kind: project
platform: Wayland
vendor: Fabric-Development; linkfrg (Ignis)
years: 2023–present
status: niche
tags: [python, pygobject, gtk3, gtk4, services]
relevance: low
---

## What it is
Two Python frameworks for writing shells with PyGObject. Fabric uses GTK3 with a signal-driven API and Hyprland helpers; Ignis uses GTK4 with a large set of built-in services (audio, network, MPRIS, notifications, recorder) and supports any compositor with layer-shell.

## What was new
- Ignis's "batteries included" services mirrored Astal's idea in Python, making a full shell possible in a few hundred lines.

## What went wrong / limits
- Ignis states its API is still subject to change and is being rewritten in Rust on a branch; the Python base did not hold [verified].
- Neither supports GNOME, because GNOME has no layer-shell.
- Python startup and memory costs are paid by every shell process.

## Lessons for swaypplet
- Another project leaving a dynamic language for Rust (after HyprPanel to Wayle): the long-running shell wants a compiled language.

## Sources
- https://github.com/ignis-sh/ignis — Ignis, Rust rewrite, GNOME unsupported [verified via search summary]
- https://github.com/Fabric-Development/fabric/blob/main/README.md — Fabric [verified via search summary]
