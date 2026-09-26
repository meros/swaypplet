---
name: Qt styles, qt5ct/qt6ct and Kvantum
slug: qt-styles-kvantum
domain: theming
kind: project
platform: cross
vendor: Qt / community (Pedram Pourang for Kvantum)
years: 2001–present
status: current
tags: [qt, qstyle, platform-theme, svg-theme, consistency]
relevance: low
---

## What it is
Qt draws widgets through a `QStyle` plugin (Fusion, Breeze, Windows) and reads colours and fonts through a platform theme plugin. Outside Plasma, `qt5ct`/`qt6ct` set style, palette and fonts; Kvantum is an SVG-based style engine that imitates GTK and other looks.

## What was new
- Separation of drawing (style) from environment (platform theme), so a desktop can supply palette and fonts without owning the widget code.
- Kvantum's SVG themes let designers restyle Qt without C++.

## What went wrong / limits
- Non-KDE desktops rely on env vars (`QT_QPA_PLATFORMTHEME`) that are easy to miss; Qt apps under GNOME/sway often look unstyled.
- `QPalette` has no accent-vs-text-accent split or status roles; colour schemes flatten into it.
- QML/Quick apps ignore QStyle, fragmenting again.

## Lessons for swaypplet
- Qt apps are the weak link of any Wayland shell theme. A generated Kvantum or qt6ct palette from the tokens (like Stylix does) is the cheapest way to cover them.

## Sources
- https://doc.qt.io/qt-6/qstyle.html — QStyle [memory]
- https://github.com/tsujan/Kvantum — Kvantum [memory]
