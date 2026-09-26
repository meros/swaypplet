---
name: ext-foreign-toplevel-list and wlr-foreign-toplevel-management
slug: foreign-toplevel
domain: wayland
kind: protocol
platform: Wayland
vendor: wlroots; wayland-protocols
years: 2019–present
status: current
tags: [protocol, taskbar, window-list, activate, close]
relevance: high
---

## What it is
`zwlr_foreign_toplevel_manager_v1` gives a taskbar every window's title, app id and state (maximised, minimised, activated, fullscreen) and lets it activate, close, minimise, maximise and fullscreen them. `ext-foreign-toplevel-list-v1` is the frozen, vendor-neutral successor to the read-only half: identity, title, app id and lifecycle, plus a stable identifier that other protocols (image capture) take.

## What was new
- ext's stable toplevel identifier lets one protocol name a window for another, which is how per-window capture works.

## What went wrong / limits
- The ext list cannot act: activating or closing from a dock still needs the wlr protocol, and an ext replacement for the management half has not landed [memory].
- Compositors that advertise only one of the two leave half the tools broken; Hyprland had a mapping protocol answering `failed` when ext-list was absent.
- No skip-taskbar hint, so utility windows appear in lists.

## Lessons for swaypplet
- swaypplet uses the ext list; for activate/close it can use `zwlr_foreign_toplevel_manager_v1` (swayfx advertises it) instead of sway IPC `[con_id]` commands, matching windows by app id and title, or keep IPC and say why.

## Sources
- https://wayland.app/protocols/wlr-foreign-toplevel-management-unstable-v1 — wlr protocol [verified via search summary]
- https://github.com/swaywm/wlr-protocols/issues/96 — skip-taskbar hint [verified via search summary]
- https://github.com/iconidentify/chonkstep/issues/77 — tools broken when ext-list missing [verified via search summary]
