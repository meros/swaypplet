---
name: IBM Carbon themes and layering tokens
slug: ibm-carbon-themes
domain: theming
kind: pattern
platform: web
vendor: IBM
years: 2018–present
status: current
tags: [themes, layering, layer-01, gray-scale, tokens]
relevance: medium
---

## What it is
Carbon ships four themes (White, Gray 10, Gray 90, Gray 100) over one token set. v11 introduced layering tokens: `$layer-01`, `$layer-02`, `$layer-03` plus `-hover`, `-active`, `-selected`, and a `Layer` component that increments the level for everything inside it.

## What was new
- Nesting as a token dimension: a component asks for "my layer" and gets the right grey for its depth; light themes alternate White/Gray 10, dark themes step lighter (Gray 100 → 90 → 80).
- Contextual tokens resolved by DOM position rather than by the component author.

## What went wrong / limits
- Many near-identical token names (`$layer-accent-01`, `$field-02`) confuse contributors; v10→v11 token renames were a heavy migration.
- Greys only; no tint or accent in surfaces.

## Lessons for swaypplet
- swaypplet's fills are `currentColor` mixes over glass, which already stack (a fill in a fill gets darker); Carbon's explicit layer counter is not needed while the principle "one glass layer per surface" holds.
- Take the "renames are migrations" warning: keep token names stable and lint for removed names.

## Sources
- https://carbondesignsystem.com/elements/themes/overview/ — four themes [verified via search summary]
- https://github.com/carbon-design-system/carbon/discussions/7743 — layering model [verified via search summary]
