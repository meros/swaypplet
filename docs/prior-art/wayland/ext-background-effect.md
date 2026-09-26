---
name: ext-background-effect-v1
slug: ext-background-effect
domain: wayland
kind: protocol
platform: Wayland
vendor: Xaver Hugl (KDE); wayland-protocols
years: 2024 proposed, merged 2025–26
status: current
tags: [protocol, blur, glass, effects, region]
relevance: high
---

## What it is
A staging protocol in which a client sets a surface-local blur region, double-buffered and applied on commit; the compositor blurs what is behind that region. Designed to be extended with other effects (contrast was named). Implemented by KWin 6.7, niri 26.04 and Mutter; Quickshell exposes it as `BackgroundEffect`; fcitx5, Zen browser and Flutter have requests open to use it.

## What was new
- Blur requested by the client for a region, rather than configured per namespace or per app id by the user. Terminals, panels and input-method popups get blur with no user config.
- Region-based: rounded cards on a transparent surface can blur exactly their shape.

## What went wrong / limits
- Took from January 2024 to merge; wlroots compositors (sway, swayfx) do not implement it yet [memory].
- The compositor decides how the blur looks; a client cannot ask for strength or tint, which is exactly what swaypplet's glass controls.

## Lessons for swaypplet
- Adapt: implement the protocol in the swayfx patch so swaypplet sets per-card blur regions instead of whole-surface namespaces. This removes the binary-frost limit on transparent popover gaps and lets GTK apps (and foot) request blur.
- Keep the material parameters in the Nix-generated table; the protocol only carries the region.

## Sources
- https://www.phoronix.com/news/Wayland-Background-Effect — merge, author, intent [verified via search summary]
- https://wayland.app/protocols/ext-background-effect-v1 — protocol [verified via search summary]
- https://gitlab.gnome.org/GNOME/mutter/-/merge_requests/5071 — Mutter support [verified via search summary]
