---
name: COSMIC desktop
slug: cosmic-desktop
domain: shells
kind: product
platform: Wayland
vendor: System76
years: 2021–present
status: current
tags: [rust, iced, tiling, wayland, panel, applets, theming]
relevance: high
---

## What it is
A desktop environment written from scratch in Rust by System76, with its own compositor (cosmic-comp, on Smithay), the iced-based toolkit libcosmic, panel, dock, launcher and settings. Epoch 1 shipped on 11 December 2025 with Pop!_OS 24.04 LTS; 1.0.x point releases followed through 2026.

## What was new
- Auto-tiling toggled per workspace from the panel or the overview, with floating windows allowed in tiled workspaces; stacks and a window-swapping mode.
- Panel applets are separate processes using a layer-shell protocol, so a crashing applet does not take down the panel.
- Theme editor with accent, frosted glass toggle and custom colours, exported to GTK apps.

## What went wrong / limits
- Building a toolkit, compositor and apps at once took four years; early releases lacked accessibility and some protocols.
- iced lacked mature text input and accessibility at the start; System76 wrote its own text stack (cosmic-text).

## Lessons for swaypplet
- Take: tiling as a per-workspace property visible in the shell, which swayfx supports via `layout` and could be shown as a panel control.
- Take: out-of-process applets for fault isolation, if swaypplet ever runs third-party pieces.
- Note: the closest peer project (Rust, Wayland, own design system); worth tracking its theme and blur work.

## Sources
- https://en.wikipedia.org/wiki/COSMIC_desktop — release date, architecture [verified]
- https://www.theregister.com/2025/11/03/cosmic_1_before_xmas/ — Epoch 1 date [verified]
- https://blog.system76.com/post/cosmic-september-new-window-swapping-mode/ — tiling features [verified]
- https://lwn.net/Articles/984638/ — iced choice, cosmic-text [verified]
