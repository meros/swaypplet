---
name: StatusNotifierItem and the Plasma System Tray
slug: statusnotifier-status-filter
domain: shells
kind: protocol
platform: KDE
vendor: KDE, freedesktop.org
years: 2009–present
status: current
tags: [tray, statusnotifier, dbus, needs-attention, overflow]
relevance: high
---

## What it is
StatusNotifierItem (SNI) is a D-Bus replacement for XEmbed tray icons, designed by KDE and used by Canonical's AppIndicators. Each item exposes an icon, tooltip, menu and a `Status` of `Passive`, `Active` or `NeedsAttention`. Plasma's System Tray shows `Active` and `NeedsAttention` items and moves `Passive` ones into an expander; users can pin each item to shown, hidden or automatic.

## What was new
The host, not the app, decides placement, and the app declares importance. Icons are data (names or pixmaps), so the tray can recolour and resize them to fit the theme.

## What went wrong / limits
- Apps set `Active` permanently, so the status is inflated and the visible tray fills anyway.
- Not a freedesktop standard in practice (the spec stayed a draft); GNOME never adopted it.
- Menus over D-Bus (`com.canonical.dbusmenu`) are a separate, bug-prone protocol.

## Lessons for swaypplet
- Take: BAR_VISION already filters the bar to `NeedsAttention`; this is the right cut given apps inflate `Active`.
- Adapt: allow a per-item override (always show, always hide) in the panel, as Plasma does, for apps that misuse the status.

## Sources
- https://www.freedesktop.org/wiki/Specifications/StatusNotifierItem/ — spec, Status values [memory]
- https://userbase.kde.org/Plasma/SystemTray — tray behaviour and per-item visibility [memory]
