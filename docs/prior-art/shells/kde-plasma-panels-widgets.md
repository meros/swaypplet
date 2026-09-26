---
name: KDE Plasma panels and widgets
slug: kde-plasma-panels-widgets
domain: shells
kind: product
platform: KDE
vendor: KDE
years: 2008–present
status: current
tags: [panel, widgets, plasmoids, customization, qml]
relevance: medium
---

## What it is
Plasma's desktop is built from panels and widgets ("plasmoids", QML since Plasma 5). Any panel on any edge can hold any widget (launcher, task manager, system tray, clock, pager); widgets can also sit on the desktop. Plasma 6 (February 2024) moved to Qt 6 and made Wayland the default session.

## What was new
Everything is a widget in a container, so the default layout is one configuration among many; users rebuild Windows-, macOS- or GNOME-like layouts with the same parts. Global themes bundle a layout with colours and icons.

## What went wrong / limits
- Configuration depth produces bugs and inconsistency: panel and widget settings dialogs differ, and layouts break after upgrades.
- Third-party widgets from the KDE Store can break at major versions (Plasma 6 required ports).
- Default experience is Windows-like, and the flexibility is rarely used by most users.

## Lessons for swaypplet
- Avoid: generic widget containers. swaypplet's fixed slots (BAR_VISION P4) are a deliberate refusal of this model, and the cost Plasma pays in QA supports that.
- Take: global theme as one bundle (layout + colours + material) matches swaypplet's single-input token set.

## Sources
- https://kde.org/announcements/megarelease/6/ — Plasma 6 release [verified]
- https://en.wikipedia.org/wiki/KDE_Plasma — architecture, plasmoids [memory]
