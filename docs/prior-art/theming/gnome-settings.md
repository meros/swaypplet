---
name: GNOME Settings
slug: gnome-settings
domain: theming
kind: product
platform: GNOME
vendor: GNOME
years: 2011–present
status: current
tags: [settings-app, panels, search, gsettings, tweaks]
relevance: medium
---

## What it is
GNOME's settings app (formerly Control Center), a sidebar of panels (Wi-Fi, Displays, Appearance, Accessibility …) backed by gsettings keys. Advanced options live in GNOME Tweaks or `dconf-editor`.

## What was new
- Deliberately small surface: settings that most users need; everything else is a gsettings key, still scriptable.
- Search integrated into the shell overview: typing "night light" in Activities opens the right panel.
- Appearance panel shows live previews of light/dark and accent.

## What went wrong / limits
- Minimalism pushed common options (fonts, title-bar buttons) into Tweaks, a separate app many users never find.
- Some keys have no UI at all, so "discoverable only via dconf-editor" became normal.

## Lessons for swaypplet
- swaypplet's rule "a setting earns a row when it is a matter of taste that a rebuild is too slow a loop for" is GNOME's discipline with a principled escape hatch (Nix). Keep it.
- Take search from the launcher: the omnibox should find individual rows (e.g. "night light warmth"), not just `:set`.

## Sources
- https://gitlab.gnome.org/GNOME/gnome-control-center — project [memory]
- https://wiki.nixos.org/wiki/GNOME — gsettings/dconf relation [verified via search summary]
