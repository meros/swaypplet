---
name: Noctalia shell
slug: noctalia
domain: wayland
kind: project
platform: Wayland
vendor: noctalia-dev
years: 2025–present
status: current
tags: [quickshell, niri, fork, minimal, lavender]
relevance: medium
---

## What it is
A Quickshell shell aimed at niri (also Hyprland, sway, KDE), with a quieter, pastel design than the Material shells. From v5 it depends on its own Quickshell fork, noctalia-qs, which adds toplevel management and window-manager interfaces.

## What was new
- Documents per-compositor settings (including KDE Plasma) so the same shell runs on several compositors.
- Forked the toolkit to get protocol features it needed, rather than waiting upstream.

## What went wrong / limits
- The toolkit fork means users of DMS and Noctalia need different Quickshell builds; fragmentation moved down a layer.

## Lessons for swaypplet
- Owning a fork is sometimes the right answer (swaypplet owns a swayfx patch), but name the upstream path out of it in writing.

## Sources
- https://docs.noctalia.dev/noctalia/compositor-settings/kde/ — KDE settings for v5+ [verified via search summary]
- https://deepwiki.com/noctalia-dev/noctalia-qs/4.4-toplevel-management-and-window-manager-interface — noctalia-qs [verified via search summary]
