---
name: Unity HUD
slug: unity-hud
domain: launchers
kind: feature
platform: other
vendor: Canonical
years: 2012–2017
status: failed
tags: [menu search, dbusmenu, global menu, intent, ubuntu]
relevance: medium
---

## What it is
In Ubuntu 12.04 (2012), tapping Alt opened a search over the focused application's menu bar, exported over D-Bus (dbusmenu/appmenu). Shuttleworth pitched it as the eventual replacement for menus.

## What was new
- **Search the app's commands from the shell**, not from inside the app: one keystroke, any app with a menu bar.
- Fuzzy matching across the full menu path ("Filters > Blur > Gaussian").
- Learned from choices per app.

## What went wrong / limits
- Needed apps to export menus; GTK3/GNOME apps moved to header bars and dropped menu bars, so there was less and less to search.
- Tied to Unity 7; Unity 8 and the convergence plan were cancelled in 2017 and Ubuntu moved to GNOME.
- Replacing menus entirely lost the discovery that browsing a menu gives.

## Lessons for swaypplet
- An idea worth adapting only where a command source exists: GTK4 apps expose `GActionGroup` on D-Bus (`org.gtk.Actions`), and KDE apps have KCommandBar. A "commands of the focused app" source is feasible for those, not universally.
- Avoid making it the only path to a feature.

## Sources
- https://techcrunch.com/2012/01/25/mark-shuttleworth-unveils-new-head-up-display-for-ubuntu-12-04 — HUD pitch [verified via search summary]
- https://en.wikipedia.org/wiki/Unity_(user_interface) — Unity end in 2017 [verified via search summary]
