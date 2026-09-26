---
name: KDE Activities
slug: kde-activities
domain: notifications-wm
kind: feature
platform: KDE
vendor: KDE
years: 2010–present
status: niche
tags: [activities, contexts, virtual-desktops, scope]
relevance: medium
---

## What it is
Separate desktop contexts introduced in the Plasma 4 era: each Activity has its own wallpaper, widgets, open windows and recent files, orthogonal to virtual desktops. In 2023 KDE developers proposed removing them for Plasma 6; users objected, and they stayed with an overhaul, moved out of Frameworks and lost features such as per-Activity power settings.

## What was new
A task context that carries state beyond windows: files, recent documents, widget layout.

## What went wrong / limits
Unclear scope: some settings and app state are per-Activity and others are not, so users cannot predict what switching changes. Two orthogonal axes (Activities by virtual desktops) created a combinatorial explosion of states and bugs. Few users, but passionate ones.

## Lessons for swaypplet
- Avoid: a second grouping axis next to workspaces. Task identity should hang off workspaces (as the bar's task board does), not a parallel concept.
- Take: if a context switches anything, it must be obvious which things; partial scope is the failure.

## Sources
- https://invent.kde.org/plasma/plasma-workspace/-/issues/35 — proposal to deprecate and remove. [verified via search summary]
- https://pointieststick.com/2024/02/06/whats-going-on-with-activities-in-plasma-6/ — outcome in Plasma 6. [verified via search summary]
- https://pointieststick.com/2023/07/26/what-we-plan-to-remove-in-plasma-6/ — removal plans. [verified via search summary]
