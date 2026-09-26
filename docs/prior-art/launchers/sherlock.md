---
name: Sherlock
slug: sherlock
domain: launchers
kind: project
platform: Wayland
vendor: Skxxtz
years: 2024–present
status: current
tags: [rust, gtk4, aliases, async widgets, fallback.json]
relevance: medium
---

## What it is
A Rust/GTK4 launcher for Wayland configured by `fallback.json`, a list of "launchers" (apps, calculator, web search, custom commands, widgets), each with an alias and priority.

## What was new
- **Alias + space scopes the query** to one launcher, shown as a chip.
- **Async widgets**: a launcher can call an API and render a live tile inside the results (music player, weather, meetings).

## What went wrong / limits
- JSON configuration for everything; ordering by hand-set priority rather than use.

## Lessons for swaypplet
- Take "prefix becomes a visible chip": after `=` or `>` is typed, show the scope as a chip with Backspace to leave it.
- Avoid live network tiles in the launcher (the Rejected list already bans weather polling).

## Sources
- https://github.com/Skxxtz/sherlock — launchers, aliases, async widgets [verified]
