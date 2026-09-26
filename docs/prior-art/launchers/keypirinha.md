---
name: Keypirinha
slug: keypirinha
domain: launchers
kind: product
platform: Windows
vendor: Jean-Charles Lefebvre
years: 2016–c. 2021
status: niche
tags: [portable, embedded python, catalog, speed, dormant]
relevance: low
---

## What it is
A portable, keyboard-only launcher for Windows with an embedded Python 3 for plugins ("packages"), configured through INI files. Built for speed and low memory.

## What was new
- **Catalog + item arguments**: items are cataloged ahead of time; after choosing one, Tab lets you type arguments for it (a search term for a web search item, a path for a command).
- Fuzzy matching with learned history, sub-millisecond on a large catalog.

## What went wrong / limits
- One developer; releases stopped around 2020–21, leaving it usable but dormant.
- INI-only configuration kept it to power users.

## Lessons for swaypplet
- Take "item then Tab for arguments" as a second use of Tab on command-like rows (web search, `ssh host`).

## Sources
- https://keypirinha.com/ — features, packages [memory]
