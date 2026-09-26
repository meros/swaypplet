---
name: Synapse (and Zeitgeist)
slug: synapse-zeitgeist
domain: launchers
kind: product
platform: GNOME
vendor: Synapse project; Zeitgeist project
years: 2010–c. 2014
status: failed
tags: [vala, activity log, zeitgeist, relevance, abandoned]
relevance: medium
---

## What it is
A Vala/GTK launcher that ranked results with Zeitgeist, a desktop-wide activity log daemon that recorded which files, apps and sites were used when. Unity's Dash used Zeitgeist too.

## What was new
- **Shared activity log as the ranking signal**: one daemon recorded events from many apps, so a launcher could rank a document by when you last edited it anywhere.
- Relevance queries like "files used with this app" or "used at this time of day".

## What went wrong / limits
- Zeitgeist needed every app to log events; few did beyond GNOME core and Unity. Its SQLite log grew, and privacy-minded users disabled it.
- When Unity ended (2017) the main consumer vanished; Synapse stopped around its 0.2.99 series.

## Lessons for swaypplet
- Avoid a desktop-wide activity database; swaypplet's own launch log plus `recently-used.xbel` is enough.
- Adapt the insight: rank files by recency of use, not by match alone; `recently-used.xbel` is already written by GTK apps.

## Sources
- https://launchpad.net/synapse-project — history, releases [memory]
- https://en.wikipedia.org/wiki/Zeitgeist_(framework) — activity log, Unity [memory]
