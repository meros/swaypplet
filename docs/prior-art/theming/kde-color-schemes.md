---
name: KDE colour schemes
slug: kde-color-schemes
domain: theming
kind: feature
platform: KDE
vendor: KDE
years: 2008–present
status: current
tags: [color-scheme, kdeglobals, color-sets, qt, gtk-bridge]
relevance: medium
---

## What it is
`.colors` files (INI, in `kdeglobals` when applied) defining colour sets per context: View, Window, Button, Selection, Tooltip, Complementary, Header. Each set has Background Normal/Alternate, Foreground Normal/Inactive/Active/Link/Visited/Negative/Neutral/Positive and Decoration Focus/Hover. Breeze Light/Dark are the defaults.

## What was new
- A context × role matrix: the same "negative foreground" is defined per set, so a red on a selection and a red on a view can differ.
- Status roles (Positive, Neutral, Negative) are first-class in the scheme, not app-defined.
- Plasma writes matching GTK settings (via `kde-gtk-config`) so GTK apps follow the scheme.

## What went wrong / limits
- About 70 colour slots per scheme with no contrast validation; community schemes often fail on Inactive or Visited text.
- Colours are absolute sRGB per mode; there is no generator, so a light and dark scheme are two unrelated files.

## Lessons for swaypplet
- swaypplet's generated modes avoid KDE's "two unrelated files" problem; keep it.
- Idea: export a `.colors` file from the tokens so Qt/KDE apps under swaypplet follow the neutral, accent and status sets.

## Sources
- https://develop.kde.org/docs/plasma/ — Plasma developer docs [memory]
- KColorScheme set/role enumeration (`KColorScheme::ColorSet`, `ForegroundRole`) [memory]
