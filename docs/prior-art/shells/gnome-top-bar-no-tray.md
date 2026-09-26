---
name: GNOME top bar without a system tray
slug: gnome-top-bar-no-tray
domain: shells
kind: pattern
platform: GNOME
vendor: GNOME Project
years: 2011–present
status: current
tags: [top-bar, tray, statusnotifier, minimalism, background-apps]
relevance: medium
---

## What it is
GNOME's top bar holds only Activities (a workspace indicator since 45), the clock in the centre, and the system status area. Legacy XEmbed tray icons went to a hidden bottom "message tray" in 3.x and were removed in 3.26 (2017). StatusNotifierItem/AppIndicator is not supported; most distributions ship an extension for it.

## What was new
A deliberate position: the bar shows system state only, apps do not get a permanent slot. Background apps are meant to use notifications and, since GNOME 44, the background-apps list in quick settings.

## What went wrong / limits
- Apps that rely on a tray (chat, sync clients, VPNs) lose their only UI; Ubuntu, Fedora variants and others patch it back with the AppIndicator extension.
- The centred clock opens notifications and calendar together, mixing two roles.

## Lessons for swaypplet
- Take: filtering the tray by `Status` (only `NeedsAttention` on the bar, the rest in the panel) is the middle path between GNOME's removal and KDE's everything-visible.
- Take: background apps listed in the panel as a first-class section.

## Sources
- https://en.wikipedia.org/wiki/GNOME_Shell — tray removal in 3.26 [memory]
- https://extensions.gnome.org/extension/615/appindicator-support/ — AppIndicator extension [memory]
