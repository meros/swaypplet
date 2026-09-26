---
name: Settings search on Android and ChromeOS
slug: settings-search
domain: theming
kind: pattern
platform: Android
vendor: Google
years: 2017–present
status: current
tags: [search, indexing, deep-link, highlight, settings-app]
relevance: high
---

## What it is
Android's Settings search (SettingsIntelligence, Android 8+) indexes every preference through `SearchIndexablesProvider`s: titles, summaries and extra keywords, each with a key. A result opens the screen and highlights the row. ChromeOS Settings (OS Settings split in 2020) has the same pattern with a search box that scrolls to and flashes the matching control, and launcher search returns settings rows directly.

## What was new
- Indexing is declared next to the setting (keywords, unique key), so search stays complete as settings are added.
- Results land on the exact control with a highlight pulse rather than on a page.
- Launcher-level search (ChromeOS launcher, Pixel launcher) treats a setting as a first-class result.

## What went wrong / limits
- Keyword quality varies; synonyms ("dark mode" vs "dark theme") are missed unless added by hand.
- Dynamic settings (per-device) need custom providers and are often missing.

## Lessons for swaypplet
- High value: have each settings row register a key, title and keywords; the omnibox then returns rows ("warmth", "dark mode", "lock after") and opens the pane scrolled to the row with a `--motion-state` highlight.
- Let `settings-defaults.json` keys double as the index keys, so the CLI path and the search path agree.

## Sources
- https://source.android.com/docs/automotive/hmi/car_settings/search_indexing — SearchIndexablesProvider, keys, keywords [verified via search summary]
- https://android.googlesource.com/platform/packages/apps/SettingsIntelligence/ — implementation [verified via search summary]
- ChromeOS settings search highlight [memory]
