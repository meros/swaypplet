---
name: wlr-output-management
slug: wlr-output-management
domain: wayland
kind: protocol
platform: Wayland
vendor: wlroots project
years: 2019–present
status: current
tags: [protocol, displays, test-apply, hotplug]
relevance: high
---

## What it is
`zwlr_output_manager_v1` lists every output head with modes, position, scale, transform and adaptive sync, and applies a new configuration for all heads in one request. A client can `test` a configuration before `apply`, and gets `succeeded`, `failed` or `cancelled`. kanshi, shikane, way-displays, wlr-randr and nwg-displays use it.

## What was new
- Atomic multi-head apply with a dry run, where X11's xrandr applied outputs one by one and could leave a half-configured desk.
- A serial on every configuration, so a stale apply after hotplug is cancelled rather than applied to the wrong heads.

## What went wrong / limits
- Still wlr-namespaced; KWin and Mutter use their own protocols, so display tools split by desktop.
- Mode matching across monitors with the same model and no serial is ambiguous; shikane exists largely to fix kanshi's matching.

## Lessons for swaypplet
- Roadmap item 2 already chooses this; add `test` before `apply` and show the result in the UI before committing.
- Handle `cancelled` by re-reading heads, never by retrying the old config.

## Sources
- https://wayland.app/protocols/wlr-output-management-unstable-v1 — protocol [memory]
- https://github.com/hw0lff/shikane — requires protocol v3+ [verified via search summary]
