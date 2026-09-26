---
name: Launcher plugin security models
slug: plugin-security-models
domain: launchers
kind: pattern
platform: cross
vendor: Raycast, PowerToys, anyrun, Alfred, KRunner, GNOME, elephant
years: 2003–present
status: current
tags: [plugins, sandbox, out of process, review, supply chain]
relevance: high
---

## What it is
The ways launchers let third parties add results, ordered from least to most isolated.

## What was new
- **In process, native** (anyrun `.so`, KRunner C++ runners, PowerToys Run .NET): fastest; a crash or malicious plugin owns the launcher.
- **Script per call** (Alfred Script Filters, rofi script mode, dmenu): process isolation for free, full user rights, one fork per step.
- **Resident child process** (Raycast Node with a V8 isolate per extension, Ulauncher, Flow JSON-RPC, elephant): crash isolation, no permission model.
- **OS-mediated out of process** (GNOME SearchProvider2 and KRunner D-Bus runners, PowerToys Command Palette COM extensions): the app is the provider; trust is the app's install trust.
- **Review as the security layer**: Raycast requires open source and PR review with auto-updates; Alfred's Gallery is curated.

## What went wrong / limits
- None of them sandbox filesystem or network access in practice; review is the real control, and side-loading bypasses it.
- Unreviewed registries (Cerebro on npm) are an open supply-chain path.

## Lessons for swaypplet
- Keep the rule: no third-party code in the swaypplet process. Sources are elephant providers or D-Bus services.
- If a script source is ever added, run it as the dmenu chassis does, as a child process with a timeout, never in the GTK thread.

## Sources
- https://developers.raycast.com/information/security — Raycast model [verified]
- https://github.com/anyrun-org/anyrun — dylib plugins [verified]
- https://developer.gnome.org/documentation/tutorials/search-provider.html — D-Bus providers [verified]
