---
name: Regolith Desktop
slug: regolith
domain: wayland
kind: product
platform: other
vendor: Ken Gilmer and the Regolith team
years: 2018–present
status: niche
tags: [i3, sway, ubuntu, gnome-flashback, curated, looks]
relevance: low
---

## What it is
A curated i3 desktop for Ubuntu and Debian that keeps GNOME's settings daemon, control center and session management underneath, with its own launcher (ilia), bar (i3xrocks, later i3status-rs) and "looks" (theme packs). Regolith 3.0 added a sway session [memory].

## What was new
- Tiling WM with GNOME's plumbing (network, displays, power) so users get a tiling desktop without assembling daemons.
- Settings as Xresources keys shared across components, with theme packs as packages.

## What went wrong / limits
- Tied to Ubuntu LTS cycles and GNOME's internal session components, which change underneath it.
- Small team; the Wayland session arrived years after sway was mature.

## Lessons for swaypplet
- One settings key space shared across every surface is the right idea (swaypplet's settings store already is); Regolith shows it survives only if one component owns the schema.

## Sources
- https://regolith-desktop.com/ — project [memory]
