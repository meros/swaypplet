---
name: The GTK theme era (GTK2 engines to GTK3 CSS)
slug: gtk-theme-era
domain: theming
kind: pattern
platform: GNOME
vendor: GNOME and theme community
years: 2002–2021
status: discontinued
tags: [gtk2, gtk3, css, themes, gnome-look, breakage]
relevance: low
---

## What it is
GTK2 themes were `gtkrc` files plus C "engines" (Clearlooks, Murrine). GTK3 (2011) replaced them with CSS stylesheets. Thousands of themes (Arc, Numix, Adapta, Materia) lived on gnome-look.org and in distro defaults.

## What was new
- CSS made themes approachable without C; a stylesheet could restyle every app on the desktop.
- `@define-color` names (`theme_bg_color`, `theme_selected_bg_color`) became a de-facto semantic palette that apps and other toolkits read.

## What went wrong / limits
- GTK3's CSS node names and selectors changed across minor releases (3.14, 3.20 especially), breaking most themes with each GNOME release.
- No test suite: themes shipped unreadable combinations (dark text on dark header bars) that apps inherited.
- Led directly to the 2019 letter and libadwaita's locked stylesheet.

## Lessons for swaypplet
- Avoid: a stylesheet that is the extension point. swaypplet's lint (semantic tier only) and the APCA test are the two things the theme era lacked.
- The named-colour vocabulary survived its themes; stable token names matter more than any single palette.

## Sources
- https://en.wikipedia.org/wiki/Adwaita_(design_language) — GTK3 CSS history [verified via search summary]
- GTK 3.20 CSS node breakage [memory]
