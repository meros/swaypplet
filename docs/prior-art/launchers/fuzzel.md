---
name: fuzzel
slug: fuzzel
domain: launchers
kind: product
platform: Wayland
vendor: Daniel Eklöf (dnkl)
years: 2020–present
status: current
tags: [fast, fcft, launch count, dmenu, icons]
relevance: medium
---

## What it is
A Wayland-native app launcher and dmenu replacement from the author of the foot terminal. Renders text with fcft/pixman, no toolkit.

## What was new
- Opens in a few milliseconds with icons, from a small C binary.
- Ranks by **launch count** kept in a cache file, then by match.
- Fuzzy matching with Levenshtein-style tolerance for typos (configurable).

## What went wrong / limits
- Launch counts never decay: last year's app stays on top.
- Apps and dmenu only; no plugins or async sources by design.

## Lessons for swaypplet
- Take typo tolerance as a last-resort tier: when no row matches, retry with one edit allowed rather than show nothing.
- swaypplet's decaying score is already better than raw counts; keep it.

## Sources
- https://codeberg.org/dnkl/fuzzel — features, cache [memory]
