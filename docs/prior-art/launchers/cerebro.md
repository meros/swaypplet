---
name: Cerebro
slug: cerebro
domain: launchers
kind: product
platform: cross
vendor: Alexandr Subbotin and community
years: 2016–c. 2021
status: failed
tags: [electron, npm plugins, previews, abandoned]
relevance: low
---

## What it is
An open-source Electron launcher for macOS, Windows and Linux with plugins published as npm packages and rich previews rendered as React components.

## What was new
Plugin install from npm inside the launcher, with any web UI as a result preview (maps, GIFs, currency charts).

## What went wrong / limits
- Electron: hundreds of MB resident and a visible cold start for a surface that must appear in one frame.
- npm plugins ran with full Node rights; no review.
- Maintenance dropped after 2020.

## Lessons for swaypplet
- Avoid web runtimes and unreviewed package registries in the launcher.

## Sources
- https://github.com/cerebroapp/cerebro — project, plugins [memory]
