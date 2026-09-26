---
name: Unity Launcher and Dash
slug: unity-launcher-dash
domain: shells
kind: product
platform: other
vendor: Canonical
years: 2010–2017
status: discontinued
tags: [launcher, dash, lenses, badges, privacy, failed-idea]
relevance: medium
---

## What it is
Unity was Ubuntu's shell from 11.04 to 17.10: a left Launcher of pinned and running apps, a top panel with a global menu and indicators, and the Dash, a full-screen search overlay extended by Lenses (later Scopes) for apps, files, music and online sources. Canonical ended Unity in April 2017 and moved Ubuntu 18.04 to GNOME.

## What was new
- Launcher icons carried badges (counts), progress bars and quicklists that apps set over a D-Bus API (LauncherEntry), a standard later adopted by Plank, Dash to Dock and KDE.
- Dash scopes: pluggable search providers in one field.

## What went wrong / limits
- The Amazon shopping lens (Ubuntu 12.10, 2012) sent desktop searches to Canonical and Amazon by default; the home lens was the natural way to search local files, so this disclosed local queries. Disabled by default only in 2016.
- Tied to Canonical's convergence plan (Unity 8, Mir, phone); when the phone failed, the desktop went with it.
- The launcher could not be moved to the bottom until 16.04.

## Lessons for swaypplet
- Avoid: sending local launcher queries to the network, even as an option on by default.
- Take: progress and count as data an app publishes and the shell renders in its own style (the LauncherEntry idea), if a task-progress mark is ever wanted.

## Sources
- https://en.wikipedia.org/wiki/Unity_(user_interface) — launcher, scopes, Amazon lens, end of Unity [verified]
