---
name: anyrun
slug: anyrun
domain: launchers
kind: project
platform: Wayland
vendor: Kirottu and anyrun-org
years: 2023–present
status: current
tags: [rust, gtk4, dylib plugins, abi_stable, krunner-like]
relevance: medium
---

## What it is
A Wayland "KRunner-like" runner in Rust and GTK4 whose plugins are shared libraries loaded with `abi_stable`. Official plugins: applications, symbols, rink calculator, shell, translate, kidex files, randr, dictionary, websearch, stdin.

## What was new
- Rust plugins with a stable ABI, so a plugin built against one release loads in the next.
- Every plugin's results shown in its own section, streamed as they arrive.

## What went wrong / limits
- `.so` plugins in `~/.config/anyrun/plugins/` run in process with the user's rights: a crash or a malicious plugin takes the launcher, and there is no review.
- Configuration in RON; the project changed maintainers.

## Lessons for swaypplet
- Avoid dlopen plugins in the shell process; swaypplet runs the bar, lock-adjacent UI and notifications in the same process.
- Take rink for units: a Rust library, no process per query, which could replace the qalc fallback.

## Sources
- https://github.com/anyrun-org/anyrun — plugin model, abi_stable, plugin list [verified]
