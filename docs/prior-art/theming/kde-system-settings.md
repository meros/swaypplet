---
name: KDE System Settings
slug: kde-system-settings
domain: theming
kind: product
platform: KDE
vendor: KDE
years: 2008–present
status: current
tags: [settings-app, kcm, highlight-changed, search, defaults]
relevance: high
---

## What it is
Plasma's settings app built from KCM modules, one per area, with a sidebar and KRunner-integrated search. Also usable per module (`kcmshell6 kcm_colors`).

## What was new
- "Highlight changed settings": a toggle that marks every row differing from the default, and a per-page Defaults button. It answers "what did I change?" at a glance.
- Every setting is available, including power-user ones, organised by modules; KRunner finds modules by keyword.
- Global themes bundle colour scheme, icons, cursors, window decorations and splash.

## What went wrong / limits
- Breadth creates depth: many pages and dense forms; historic inconsistency between modules.
- Plasma 5.x rewrites (QML KCMs) moved options around between releases.

## Lessons for swaypplet
- Take "highlight changed settings": swaypplet already knows exactly which sections the user file overrides; marking those rows (and offering per-row reset) is small and directly useful.
- Per-module entry point (`kcmshell`) maps to deep links into a tab.

## Sources
- https://userbase.kde.org/System_Settings — overview [memory]
- Highlight changed settings (Plasma 5.20+) [memory]
