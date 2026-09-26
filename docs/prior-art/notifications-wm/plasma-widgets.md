---
name: KDE Plasma widgets (plasmoids)
slug: plasma-widgets
domain: notifications-wm
kind: feature
platform: KDE
vendor: KDE
years: 2008–present
status: current
tags: [plasmoids, panel, desktop, qml, extensibility, security]
relevance: medium
---

## What it is
Everything in Plasma, panel items included, is a widget (plasmoid), written in QML/JavaScript, placeable on panels or the desktop, and installable from the KDE Store. In March 2024 a user lost personal files to a third-party global theme; KDE warned that third-party themes and widgets "can and will run arbitrary code".

## What was new
Total composability: the shell is a container of the same widget type, so users rebuild the panel from parts.

## What went wrong / limits
Store content runs with user rights, unreviewed. The combinatorial space of layouts and widgets makes bugs hard to reproduce. Widget-level configuration spreads settings across many dialogs.

## Lessons for swaypplet
- Avoid: a plugin surface for third-party code in the shell; swaypplet's fixed spatial grammar (BAR_VISION P4) is the deliberate opposite.
- Take: when something must be extensible, make it data (settings.json, tokens) rather than code.

## Sources
- https://discuss.kde.org/t/warning-global-themes-and-widgets-created-by-3rd-party-developers-for-plasma-can-and-will-run-arbitrary-code-you-are-encouraged-to-exercise-extreme-caution-when-using-these-products/12714 — the warning. [verified via search summary]
- https://en.wikipedia.org/wiki/KDE_Plasma_6 — Plasma structure. [verified via search summary]
