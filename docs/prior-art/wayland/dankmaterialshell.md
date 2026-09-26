---
name: DankMaterialShell
slug: dankmaterialshell
domain: wayland
kind: project
platform: Wayland
vendor: AvengeMedia
years: 2025–present
status: current
tags: [quickshell, go, niri, all-in-one, material, plugins]
relevance: high
---

## What it is
A full desktop shell built with Quickshell for the UI and a Go backend for services, targeting niri, Hyprland, sway, labwc, MangoWC, Scroll and Miracle WM. It replaces waybar, swaylock, swayidle, mako, fuzzel and the polkit agent in one install, and has a plugin system (for example a wluma plugin).

## What was new
- Explicit "replace the stitched-together stack" framing; the same scope as swaypplet (bar, launcher, lock, idle, notifications, polkit).
- A compiled backend (Go) for the services, QML only for UI: the split HyprPanel lacked.
- Picks up `ext-background-effect` blur on niri automatically.

## What went wrong / limits
- Seven compositor backends is a large test matrix for a young project [memory].
- Material styling is generic; little of the design is its own.

## Lessons for swaypplet
- Closest prior art in scope. Compare feature by feature when planning; DMS's plugin API is what swaypplet deliberately does not have.
- The UI/backend split in two languages works; swaypplet gets the same split inside one Rust process.

## Sources
- https://github.com/AvengeMedia/DankMaterialShell — scope, stack, compositors [verified via search summary]
- https://github.com/zurajm/dankWluma — plugin example [verified via search summary]
