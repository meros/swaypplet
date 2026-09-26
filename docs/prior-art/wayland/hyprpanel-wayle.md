---
name: HyprPanel and Wayle
slug: hyprpanel-wayle
domain: wayland
kind: product
platform: Wayland
vendor: Jas Singh (Jas-SinghFSU)
years: 2024–present (HyprPanel archived 27 Apr 2026)
status: discontinued
tags: [ags, burnout, rewrite, rust, gtk4, relm4]
relevance: high
---

## What it is
HyprPanel was a feature-rich Hyprland bar and panel on AGS (GJS, GTK3) with dashboards, settings GUI and themes. Its author archived it on 27 April 2026 once Wayle, a compositor-agnostic rewrite in Rust with GTK4 and Relm4, reached feature parity.

## What was new
- A GUI settings dialog for a riced bar, so users stopped editing config files; most AGS shells had none.
- Wayle: bar, notifications, OSD, wallpaper and device controls in one Rust binary with Hyprland, niri and Mango workspace backends.

## What went wrong / limits
- The author named the reasons: configurability limits, installation pain, too many dependencies, breaking changes in the dependencies (AGS v1 to v2, Hyprland), and GJS not being a systems language.
- A mature, popular project was ended by its stack, not by lack of users.

## Lessons for swaypplet
- Validates swaypplet's own stack choice (Rust, GTK4, few runtime deps).
- Avoid dependency on a fast-moving framework you do not control; pin and own the seams (zbus, wayland-client) as swaypplet does.

## Sources
- https://github.com/Jas-SinghFSU/HyprPanel — archived, successor Wayle [verified via search summary]
- https://github.com/wayle-rs/wayle — Wayle modules and motivation [verified via search summary]
- https://bbs.archlinux.org/viewtopic.php?id=312821 — Wayle announcement [verified via search summary]
