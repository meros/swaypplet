---
name: AGS and Astal
slug: ags-astal
domain: wayland
kind: project
platform: Wayland
vendor: Aylur
years: 2023–present
status: current
tags: [gjs, typescript, jsx, vala, services, gtk4]
relevance: medium
---

## What it is
AGS v1 was a GJS (GNOME's JavaScript) shell framework. v2 (Nov 2024) moved the core into Astal, a set of Vala/C libraries (Hyprland, network, battery, MPRIS, notifications, tray, Wireplumber, and more) usable from any GObject-introspection language, and turned AGS into a scaffolding CLI for Astal plus Gnim (JSX for GJS).

## What was new
- Services as standalone, introspectable libraries, each with a CLI, so a Lua, Python or TS shell can reuse the same battery or MPRIS logic.
- JSX and reactive bindings for GTK widgets.

## What went wrong / limits
- The v1 to v2 rewrite broke every config; HyprPanel and end-4 had to port or leave. end-4 left for Quickshell.
- GJS is not a systems language: memory growth and GC pauses in long-running bars [memory]; HyprPanel's author cited it when leaving.

## Lessons for swaypplet
- Take: service code with no widget dependency, testable on its own. swaypplet's `services/` is already shaped this way; keep it so.
- Avoid framework rewrites that break every downstream at once.

## Sources
- https://aylur.github.io/astal/guide/introduction — Astal [verified via search summary]
- https://github.com/Aylur/ags/releases/tag/v2.0.0 — v2 release [verified via search summary]
