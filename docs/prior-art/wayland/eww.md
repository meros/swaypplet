---
name: eww (ElKowar's Wacky Widgets)
slug: eww
domain: wayland
kind: product
platform: Wayland
vendor: ElKowar
years: 2020–present
status: niche
tags: [widgets, rust, gtk3, yuck, polling, declarative]
relevance: medium
---

## What it is
A Rust widget system, independent of the WM, configured in yuck (an S-expression language) and styled with SCSS. Data comes from `defpoll` (run a command every N seconds) and `deflisten` (read a command's stdout lines), and widgets bind to those variables.

## What was new
- Made arbitrary desktop widgets (dashboards, bars, sidebars) scriptable by non-programmers; drove the 2021–23 r/unixporn look.
- `deflisten` as a push model: one long-lived process emits JSON, widgets redraw on change.

## What went wrong / limits
- `defpoll` encourages spawning shells every second; many published configs cost measurable CPU.
- GTK3 only, and development slowed as the author's time went elsewhere; forks (ewwii) appeared [memory].
- Yuck is a language to learn with no types; errors surface at runtime.

## Lessons for swaypplet
- Avoid: a DSL plus exec-based data is flexible but turns every user into a performance engineer.
- Take `deflisten`'s shape for any external data feed: one long-lived producer, change-only updates.

## Sources
- https://elkowar.github.io/eww/ — docs, defpoll/deflisten [verified via search summary]
- https://github.com/elkowar/eww — project [verified via search summary]
