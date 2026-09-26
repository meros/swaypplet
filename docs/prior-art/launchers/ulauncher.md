---
name: Ulauncher
slug: ulauncher
domain: launchers
kind: product
platform: GNOME
vendor: Ulauncher project
years: 2015–present
status: current
tags: [python, gtk, extensions, websocket, long beta]
relevance: low
---

## What it is
A GTK launcher for Linux written in Python with a public extension gallery. Extensions are separate Python processes that talk to the launcher.

## What was new
- Extensions out of process (over a local socket), each with a keyword, installed from a URL in the gallery.
- Fuzzy app search with learned selection order.

## What went wrong / limits
- v6, a large rewrite with a new extension API, has been in beta for years (beta34 in July 2026) while v5 only gets fixes; extension authors wait on an unstable API.
- X11 heritage: global hotkeys and positioning are awkward on Wayland.

## Lessons for swaypplet
- Avoid long-lived rewrite branches that freeze the provider API; change the contract in small compatible steps.

## Sources
- https://newreleases.io/project/github/Ulauncher/Ulauncher/release/v6.0.0-beta34 — beta34 [verified via search summary]
- https://docs.ulauncher.io/ — extension API v2 [verified via search summary]
