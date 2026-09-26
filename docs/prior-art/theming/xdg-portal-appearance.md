---
name: XDG desktop portal appearance settings
slug: xdg-portal-appearance
domain: theming
kind: protocol
platform: Wayland
vendor: freedesktop.org / Flatpak
years: 2021–present
status: current
tags: [portal, color-scheme, accent-color, contrast, reduced-motion, cross-desktop]
relevance: high
---

## What it is
`org.freedesktop.portal.Settings` exposes a small, read-only set of standardised host preferences under the `org.freedesktop.appearance` namespace: `color-scheme` (0 none, 1 dark, 2 light), `accent-color` (sRGB triple in [0,1]), `contrast` (0 normal, 1 higher) and `reduced-motion` (0/1). Backends (GNOME, KDE, and `xdg-desktop-portal-gtk` reading gsettings) implement it; GTK4, libadwaita, Qt 6.5+, Firefox, Chromium and Electron read it and emit change signals.

## What was new
- A desktop-neutral contract: one dark/light preference that browsers turn into `prefers-color-scheme`, toolkits into their dark variant.
- Accent as a real colour, not a name, so desktops with arbitrary accents interoperate.
- `SettingChanged` signal allows live switches without restart.

## What went wrong / limits
- "No preference" (0) is common on minimal Wayland sessions, so apps default to light unless someone writes the setting.
- Only four keys; fonts, icon theme and cursor stay in gsettings/XSettings.

## Lessons for swaypplet
- Today the nixos repo pins apps dark (`users/modules/scaling.nix`: `color-scheme = "prefer-dark"`, `gtk-application-prefer-dark-theme = true`), so in light mode the shell turns light while every app stays dark.
- High value, small: when auto mode flips, set `org.gnome.desktop.interface color-scheme` (read by `xdg-desktop-portal-gtk`/`-gnome`), `accent-color`, and contrast. Then browsers, libadwaita and Qt follow the sun with the shell, and reduced motion follows the Motion setting.
- Apply them in the same deferred step as the token reload so the whole screen flips at once.

## Sources
- https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Settings.html — keys and values [verified]
- https://github.com/flatpak/xdg-desktop-portal/blob/main/data/org.freedesktop.portal.Settings.xml — interface [verified via search summary]
- Toolkit adoption (GTK4, Qt 6.5, Firefox, Chromium) [memory]
