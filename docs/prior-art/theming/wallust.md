---
name: wallust
slug: wallust
domain: theming
kind: project
platform: cross
vendor: explosion-mental
years: 2022–present
status: current
tags: [wallpaper, palette, rust, contrast, kmeans, oklab, templates]
relevance: medium
---

## What it is
"A better pywal" in Rust. Backends (full, resized, wal, kmeans, fast_resize) extract colours, a colour space (lab, labmixed, lch, OkLab and others) groups them, and a palette mode (dark, light, softdark, harddark, ansidark ...) orders them into 16 slots, with a contrast check that nudges foregrounds. Templates use a Jinja-like engine.

## What was new
- Pipeline split into backend → colour space → palette → contrast, each configurable.
- Contrast checking step (`check_contrast`) that fixes pywal's worst failure.
- Native, fast, cache keyed by image and settings.

## What went wrong / limits
- Still a 16-slot terminal model: no semantic roles, so apps must guess which slot is an accent.
- Many knobs; results vary a lot between backends and colour spaces, which moves the tuning burden to users.

## Lessons for swaypplet
- The staged pipeline is a good shape for `wallpaper.rs`; swaypplet's version has fewer knobs and more guarantees, which is right for a shell.
- Offer an ANSI-16 export derived from the categorical slots for terminals, so the wallpaper tint reaches them without a second tool.

## Sources
- https://codeberg.org/explosion-mental/wallust — project [verified via search summary]
- https://explosion-mental.codeberg.page/wallust/ — backends, palettes [verified via search summary]
- https://deepwiki.com/explosion-mental/wallust — pipeline stages [verified via search summary]
