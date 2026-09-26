---
name: Flow Launcher
slug: flow-launcher
domain: launchers
kind: product
platform: Windows
vendor: Flow Launcher community
years: 2020–present
status: current
tags: [wox fork, plugin store, json-rpc, everything integration, community]
relevance: medium
---

## What it is
A community fork of Wox for Windows, written in C#. It searches apps, files (through Everything or Windows Search), runs shell commands and hosts a plugin store with hundreds of plugins.

## What was new
- Plugins in C# (in process) or in Python/JS/any language over JSON-RPC (out of process), all installable from an in-app store (`pm install`).
- Uses Everything as the file backend instead of building its own index.
- Query history and "last opened" ranking boosts.

## What went wrong / limits
- C# plugins share the process; no sandbox. Quality varies across the store.
- Visual integration with Windows 11 is theme-based rather than native.

## Lessons for swaypplet
- Take "reuse the best index, do not build one": elephant's `files` provider and a locate-style backend beat a new crawler.
- A fork with a fresh maintainer group saved a stalled project: keep swaypplet's launcher backend swappable behind `services::elephant`.

## Sources
- https://github.com/Flow-Launcher/Flow.Launcher — plugins, Everything, JSON-RPC [memory]
