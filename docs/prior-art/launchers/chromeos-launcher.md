---
name: ChromeOS launcher and the Everything button
slug: chromeos-launcher
domain: launchers
kind: product
platform: other
vendor: Google
years: 2011–present
status: current
tags: [launcher key, bubble launcher, continue section, settings search, dedicated key]
relevance: medium
---

## What it is
ChromeOS replaced Caps Lock with a Search/Launcher key from the first Chromebooks (2011). In 2022 the full-screen launcher became a compact "bubble" beside the shelf, and in 2023 Google rebranded the key as the "Everything Button" on new keyboards. Search covers apps, settings, files, Drive, help articles, Play Store, and web.

## What was new
- A dedicated hardware key for search, not a chord.
- **Continue section**: recent files and tabs at the top of the empty launcher, above the app grid.
- Settings pages and help answers as results, with inline answers (calculator, dictionary).

## What went wrong / limits
- Web and Drive results mixed with local ones; ranking across them is opaque.
- The key's meaning changed name twice (Search, Launcher, Everything), then Quick Insert took another key in 2024.

## Lessons for swaypplet
- Take "continue" on the empty query: recent files (`recently-used.xbel`) next to frecent apps.
- Super already is the dedicated key; keep a tap of it bound to the launcher.

## Sources
- https://support.google.com/chromebook/answer/1047364 — launcher and search [memory]
- https://blog.google/products/chromebooks/ — 2022 launcher redesign, 2023 Everything Button [memory]
