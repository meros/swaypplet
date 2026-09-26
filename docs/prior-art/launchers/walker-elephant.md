---
name: walker and elephant
slug: walker-elephant
domain: launchers
kind: project
platform: Wayland
vendor: Andrej Benz (abenz1267)
years: 2023–present
status: current
tags: [gtk4, rust, daemon, protobuf, unix socket, providers]
relevance: high
---

## What it is
walker is a GTK4/Rust launcher; elephant is its separate backend daemon (Go), which serves providers over a Unix socket with Protocol Buffers. swaypplet's launcher uses elephant directly.

## What was new
- **Backend as a daemon**: providers (desktop apps with launch history, files, clipboard with images, runner, symbols, calc/units via qalc, custom menus, websearch, bookmarks, todo, windows, snippets, Bluetooth, Arch packages, 1Password) live in one resident process that any front end can query.
- Front end and backend release separately; a second front end (swaypplet) gets every provider without writing one.

## What went wrong / limits
- Two processes to keep in step; protocol changes between elephant releases can break front ends.
- Provider ranking and history live in elephant, so a front end with its own frecency (swaypplet) has two ranking layers to reconcile.

## Lessons for swaypplet
- Keep elephant as the plugin boundary: out of process, crash-isolated, maintained upstream.
- Decide one owner for usage history; either feed swaypplet's launches back to elephant or ignore elephant's history, so the two do not fight.

## Sources
- https://github.com/abenz1267/elephant — providers, socket, protobuf [verified]
- https://github.com/abenz1267/walker — GTK4/Rust, requires elephant [verified]
