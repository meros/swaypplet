---
name: mako, dunst and fnott
slug: mako-dunst-fnott
domain: wayland
kind: product
platform: Wayland
vendor: emersion (mako); dunst project; dnkl (fnott)
years: 2013–present
status: current
tags: [notifications, criteria, rules, lightweight]
relevance: medium
---

## What it is
Popup-only notification daemons. mako has INI sections with criteria (`[app-name=Slack urgency=high]`) that override style and behaviour, modes (e.g. `do-not-disturb`) toggled by `makoctl mode`, and no history UI. dunst began on X11, supports Wayland, and uses rules plus a history recalled by shortcut. fnott is a minimal daemon from foot's author.

## What was new
- mako's criteria and modes: DND is just a mode that sets `invisible=1` for matching notifications, so "quiet except calls" is one config section.
- dunst's history pop (`dunstctl history-pop`) as a keyboard way to see what you missed.

## What went wrong / limits
- No centre or grouping; users bolt on waybar modules to count unread.
- Config reload needed for rule changes; no per-app settings UI.

## Lessons for swaypplet
- Adapt mako's modes for roadmap item 3: "presenting" is a mode whose rules decide which notifications still pop (calls, critical), not a single on/off DND.

## Sources
- https://github.com/emersion/mako — criteria and modes [memory]
- https://dunst-project.org/documentation/ — rules, history [memory]
