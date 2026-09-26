---
name: Style Dictionary
slug: style-dictionary
domain: theming
kind: project
platform: cross
vendor: Amazon (now community, with Tokens Studio)
years: 2017–present
status: current
tags: [tokens, build-system, transforms, platforms, codegen, tokens-studio]
relevance: medium
---

## What it is
A build system that takes token JSON and emits platform files (CSS variables, SCSS, iOS Swift, Android XML, Compose, JS). v4 (2024) added first-class DTCG support; types come from `$type` rather than the older category/type/item (CTI) naming.

## What was new
- One source, many targets via composable transforms (name casing, colour format, px→rem) and formats per platform.
- Made "tokens as source of truth, generated per platform" standard practice in corporate design systems.

- Its companion Tokens Studio (Figma plugin, 2020–) popularised tokens-in-Git edited from the design tool and themes as ordered token-set selections, which fed DTCG's resolver module.

## What went wrong / limits
- Configuration grows complex; theming (modes, brands) needs multiple builds or plugins.
- Pre-v4, CTI naming conventions leaked into token names.

## Lessons for swaypplet
- swaypplet already has the pattern in Rust (`tokens::css`, `paint.rs`, `sway.rs`). The idea to borrow is the explicit platform list: add emitters (GTK4 libadwaita overrides, Kvantum, foot/kitty, Neovim) as formats of one generator rather than as separate scripts.

## Sources
- https://styledictionary.com/ — project [verified via search summary]
- https://tokens.studio/blog/style-dictionary-v4-plan — Tokens Studio relation [verified via search summary]
- https://v4.styledictionary.com/info/dtcg/ — DTCG support in v4 [verified via search summary]
