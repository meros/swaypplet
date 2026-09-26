---
name: ext-workspace-v1
slug: ext-workspace
domain: wayland
kind: protocol
platform: Wayland
vendor: wayland-protocols (COSMIC, labwc and KDE contributors)
years: 2025–present
status: current
tags: [protocol, workspaces, taskbar, groups]
relevance: high
---

## What it is
A staging protocol giving taskbars the list of workspaces, grouped by output (workspace groups), with name, coordinates and state (active, urgent, hidden), and requests to activate, deactivate, assign to a group, create and remove. It replaced years of the unmerged `ext-workspace-unstable-v1` draft that Waybar supported. wlroots 0.20 and sway 1.12 implement it; labwc and COSMIC did earlier.

## What was new
- A compositor-neutral workspace list, so a bar's workspace module no longer needs one backend per compositor.
- Urgent as a protocol state, matching i3's urgent hint.

## What went wrong / limits
- Took about five years in draft; every bar wrote per-compositor code meanwhile (Waybar has modules for sway, Hyprland, niri, river, dwl and ext).
- No window-to-workspace mapping; a bar still needs IPC or foreign-toplevel for that.

## Lessons for swaypplet
- Once swayfx is on wlroots 0.20, the workspace list could come from ext-workspace, keeping sway IPC only for task metadata and marks. Low urgency; the payoff is portability.

## Sources
- https://wayland.app/protocols/ext-workspace-v1 — protocol purpose [verified via search summary]
- https://www.phoronix.com/news/wlroots-0.20-Sway-1.12-rc1 — sway 1.12 support [verified via search summary]
