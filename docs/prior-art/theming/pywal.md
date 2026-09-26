---
name: pywal
slug: pywal
domain: theming
kind: project
platform: cross
vendor: Dylan Araps
years: 2017–2024
status: discontinued
tags: [wallpaper, palette, terminal, ricing, templates, imagemagick]
relevance: high
---

## What it is
A Python tool that generates a 16-colour terminal palette from a wallpaper (via ImageMagick and other backends), applies it to open terminals by escape sequences, and writes templates (`~/.cache/wal/colors.css`, `.json`, `.Xresources`) that other programs import. Archived by its owner on 26 April 2024.

## What was new
- One command re-themes the whole ricing stack (terminals, bar, launcher, editor) from the wallpaper, live, including already-open terminals.
- A cache of templates as the integration surface: any app that can read a file can follow.

## What went wrong / limits
- Takes lightness from the image: dark wallpapers give low-contrast colour slots, light ones unreadable foregrounds; no contrast checking.
- Colours are assigned to ANSI slots by order, not by hue, so "red" can be green.
- Unmaintained since 2021, archived in 2024; forks (pywal16) and successors (wallust, matugen) took over.

## Lessons for swaypplet
- Avoid: taking lightness from the image, and slot assignment by extraction order. swaypplet's design doc already cites this as the reason for hue-only rules.
- Take: the template cache as an integration surface. A `$XDG_CACHE_HOME/swaypplet/colors.{json,css}` export would let terminals and editors follow the shell.

## Sources
- https://github.com/dylanaraps/pywal — project, archived [memory; archive date verified via https://crates.io/crates/wallust search summary]
- https://alternativeto.net/software/pywal/about — status [verified via search summary]
