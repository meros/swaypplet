---
name: PowerToys Run
slug: powertoys-run
domain: launchers
kind: product
platform: Windows
vendor: Microsoft (open source)
years: 2020–present (superseded)
status: discontinued
tags: [alt+space, action keywords, in-process plugins, dotnet]
relevance: medium
---

## What it is
The Alt+Space launcher in Microsoft's open-source PowerToys (v0.18, 2020), derived from Wox. Plugins for apps, calculator, shell, registry, services, window walker, unit converter; each has an action keyword prefix.

## What was new
- An OS vendor shipping a Wox-style launcher as open source, with per-plugin **action keywords** (`=` calc, `>` shell, `<` window walker, `?` search).
- Global-results vs keyword-only plugins: each plugin can be in the unprefixed query or only behind its keyword.

## What went wrong / limits
- Third-party plugins are .NET assemblies loaded in process: one bad plugin crashes or slows the launcher, and there is no sandbox or store.
- Microsoft froze it in favour of Command Palette in 2025 rather than retrofit isolation.

## Lessons for swaypplet
- Take the per-source switch "in the plain query / only behind its prefix"; swaypplet's Launcher settings could offer both states per source.
- Avoid in-process plugins (see [plugin-security-models](plugin-security-models.md)).

## Sources
- https://learn.microsoft.com/en-us/windows/powertoys/run — plugins, action keywords [memory]
- https://learn.microsoft.com/en-us/windows/powertoys/command-palette/overview — successor, shared calculator [verified]
