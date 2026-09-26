---
name: Albert
slug: albert
domain: launchers
kind: product
platform: cross
vendor: Manuel Schneider
years: 2014–present
status: current
tags: [qt, c++, python plugins, triggers, fallbacks, usage ranking]
relevance: medium
---

## What it is
A Qt/C++ keyboard launcher for Linux (and macOS) with native C++ and Python plugins: apps, files, calculator, clipboard, snippets, websearch, and many community extensions.

## What was new
- **Triggers**: each plugin can be global or only answer behind a trigger string, configurable per plugin.
- **Fallback items** shown when nothing matches (web search, run in terminal), with order set by the user.
- Usage-weighted ranking mixed with match score; a "global query" mode that merges all plugins with a time budget.

## What went wrong / limits
- Plugin ABI changes across major versions broke third-party Python plugins repeatedly.
- One core maintainer; distribution packaging lagged.

## Lessons for swaypplet
- Take per-source trigger vs global setting (same as [powertoys-run](powertoys-run.md)).
- Take a time budget for the merged query: providers that miss it fill in later, never block the first frame.

## Sources
- https://albertlauncher.github.io/ — plugins, triggers, fallbacks [memory]
