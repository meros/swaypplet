---
name: GNOME 47 accent colours
slug: gnome-accent-colors
domain: theming
kind: feature
platform: GNOME
vendor: GNOME
years: 2024–present
status: current
tags: [accent, gsettings, portal, curated-palette]
relevance: high
---

## What it is
GNOME 47 added a system accent colour chosen from nine fixed options (blue, teal, green, yellow, orange, red, pink, purple, slate) in Settings → Appearance. It is stored in `org.gnome.desktop.interface accent-color` and exposed to sandboxed apps via the portal's `accent-color` key; libadwaita 1.6 apps and GNOME Shell follow it.

## What was new
- A curated set instead of a free picker: each option was tuned so that text-safe variants pass contrast in both modes.
- Accent travels through a cross-desktop portal key, which KDE Plasma 6 also implements, so apps follow either desktop.

## What went wrong / limits
- Nine colours only, no custom colour and no wallpaper-derived option; extensions add those.
- Apps that are not libadwaita (GTK3, Qt, Electron) mostly ignore it.

## Lessons for swaypplet
- swaypplet's six curated accent pairs follow the same philosophy; publish the chosen accent through `gsettings accent-color` (nearest enum) and the portal RGB so GTK4/libadwaita apps match the shell.
- Curated plus a wallpaper tint is already more than GNOME ships; no need for a free picker.

## Sources
- https://www.phoronix.com/news/GNOME-Shell-Accent-Color-Merged — merge, portal honoured [verified via search summary]
- https://www.osnews.com/story/140769/gnome-47-released-with-accent-colours-and-completely-new-open-save-file-dialogs/ — release [verified via search summary]
- https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Settings.html — accent-color key [verified]
