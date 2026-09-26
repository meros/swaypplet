---
name: Hyprland plugin API and hyprland-plugins
slug: hyprland-plugin-api
domain: wayland
kind: feature
platform: Wayland
vendor: hyprwm
years: 2023–present
status: current
tags: [plugins, abi, hyprpm, hyprexpo, overview, fragility]
relevance: medium
---

## What it is
Hyprland loads C++ shared objects into the compositor process. `hyprpm` fetches plugin repos, builds them against the headers of the running Hyprland and loads them. The official `hyprland-plugins` repo carries hyprexpo (workspace grid overview), hyprbars (title bars), hyprtrails, borders-plus-plus and others.

## What was new
- Lets third parties change rendering itself: hyprexpo renders every workspace into an off-screen framebuffer and composites a live grid with animations, something no layer-shell client can do.
- Function hooks into compositor internals, not just a stable API.

## What went wrong / limits
- No stable ABI: an `abiHash` derived from the Hyprland commit and library versions gates loading, and every Hyprland update forces a rebuild of every plugin. Authors pin plugin commits per Hyprland release.
- Internal methods "may be changed, removed or added without any prior notice"; hooks break silently.
- hyprexpo was retired from the official repo and lives on in community forks chasing releases.
- A plugin crash is a compositor crash.

## Lessons for swaypplet
- Avoid in-process compositor extension beyond a small, owned patch; the overview (roadmap item 4) should stay a client over `ext-image-copy-capture`, as planned.
- If swayfx patches grow, give them one narrow, versioned interface rather than scattered hooks.

## Sources
- https://wiki.hypr.land/Plugins/Development/Plugin-Guidelines/ — API vs internal stability [verified via search summary]
- https://deepwiki.com/hyprwm/Hyprland/9-plugin-system — abiHash [verified via search summary]
- https://github.com/sandwichfarm/hyprexpo — retired upstream, fork continues [verified via search summary]
