---
name: libadwaita
slug: libadwaita
domain: theming
kind: project
platform: GNOME
vendor: GNOME
years: 2021–present
status: current
tags: [gtk4, style-manager, named-colors, css-variables, dark-mode]
relevance: high
---

## What it is
GNOME's platform library for GTK4 apps, carrying the Adwaita stylesheet and widgets. `AdwStyleManager` follows the system colour scheme, high contrast and (since 1.6) accent colour. Since 1.6/1.7 the stylesheet uses CSS custom properties (`--accent-bg-color`, `--window-bg-color`, `--view-fg-color`).

## What was new
- Semantic named colours as the only public API for app CSS: `accent_bg_color`, `accent_color` (text-safe accent), `destructive_*`, `success_*`, `card_bg_color`, plus shade/opacity tokens.
- A separate text-safe accent (`accent_color`) derived from `accent_bg_color` per mode so that accent text passes contrast.
- Dark mode and high contrast are variants of one stylesheet, not separate themes; apps can request `prefer-dark` but the system decides by default.

## What went wrong / limits
- No public theme API; restyling is officially unsupported, which frustrated the theming community.
- GTK4 CSS custom properties arrived only in 4.16, so libadwaita < 1.6 used `@define-color`, which apps then depended on.

## Lessons for swaypplet
- swaypplet's `--accent` (text) vs `--accent-bg` split mirrors libadwaita's; keep the naming close enough that a libadwaita export is a straight mapping.
- Idea: write a `gtk-4.0/gtk.css` that sets libadwaita's public variables from swaypplet's tokens, so native GTK apps share the accent and neutrals without "theming" them.

## Sources
- https://gnome.pages.gitlab.gnome.org/libadwaita/doc/main/css-variables.html — CSS variables [memory]
- https://gnome.pages.gitlab.gnome.org/libadwaita/doc/main/styles-and-appearance.html — style manager, named colours [memory]
- https://en.wikipedia.org/wiki/Adwaita_(design_language) — history [verified via search summary]
