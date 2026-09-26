---
name: GNOME Shell search providers
slug: gnome-shell-search-providers
domain: launchers
kind: protocol
platform: GNOME
vendor: GNOME
years: 2011–present
status: current
tags: [dbus, SearchProvider2, overview, per-app results, subsearch]
relevance: high
---

## What it is
The D-Bus interface `org.gnome.Shell.SearchProvider2` that lets apps (Files, Calendar, Settings, Characters, Calculator) answer searches typed in the Activities overview. Registered by an ini file in `$(datadir)/gnome-shell/search-providers/`.

## What was new
- **Five methods**: GetInitialResultSet, GetSubsearchResultSet (narrow the previous IDs as the user types more), GetResultMetas (name, icon, description for visible IDs only), ActivateResult, LaunchSearch (open the app's own search).
- **The app is the provider**: the same binary serves in-app search and shell search, so results match.
- Results grouped under the app icon, at most three per app; clicking the icon opens the full search in the app.

## What went wrong / limits
- Every enabled provider is D-Bus-activated on the first keystroke; a slow or badly written provider delays the overview, and users disable providers to get speed back.
- Grouped-by-app layout means no global ranking: the best hit can sit under the third app.

## Lessons for swaypplet
- Take subsearch: narrow the last answer locally before asking again; swaypplet already does this for elephant rows.
- Take "metas only for visible rows" to keep icon loads off the path.
- Consider being a SearchProvider2 *client*: then GNOME apps' providers become elephant-like sources for free.

## Sources
- https://developer.gnome.org/documentation/tutorials/search-provider.html — methods, ini file, three per app [verified]
