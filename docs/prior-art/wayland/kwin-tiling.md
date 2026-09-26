---
name: KWin custom tiling and tiling scripts (Bismuth, Polonium, Karousel)
slug: kwin-tiling
domain: wayland
kind: feature
platform: KDE
vendor: KDE; script authors
years: 2020–present
status: current
tags: [kwin, tiling, scripts, zones, plasma]
relevance: low
---

## What it is
Plasma 5.27 added a custom tiling editor (Meta+T): the user draws a zone layout per screen and Shift-drags windows into zones. Full auto-tiling lives in KWin scripts: Bismuth (Plasma 5), Polonium (its Plasma 6 successor) and Karousel (scrollable tiling).

## What was new
- Zones as a user-drawn layout, not a tiling algorithm; windows stay floating until dropped.
- KWin blur via `org_kde_kwin_blur` for years, then Xaver Hugl's `ext-background-effect-v1`, which KWin 6.7 implements.

## What went wrong / limits
- Bismuth died with the Plasma 6 port: it relied on KWin internals that the new scripting API removed [memory]. Script-based tiling is always one API change from breakage.

## Lessons for swaypplet
- Tiling added on top of a compositor via its scripting surface is fragile; policies that matter belong in the compositor or a stable protocol.

## Sources
- https://github.com/peterfajdiga/karousel — Karousel [verified via search summary]
- https://www.phoronix.com/news/Wayland-Background-Effect — ext-background-effect author, KWin support [verified via search summary]
