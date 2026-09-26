---
name: Open Props
slug: open-props
domain: theming
kind: project
platform: web
vendor: Adam Argyle
years: 2021–present
status: niche
tags: [css-variables, sub-atomic, adaptive, easing, normalize]
relevance: low
---

## What it is
A library of CSS custom properties: colours, sizes, radii, shadows, easings, gradients, animations, and a `normalize.css` with "adaptive props" that switch light/dark via `prefers-color-scheme` or `.light`/`.dark` switch classes.

## What was new
- Tokens without a framework: just variables, imported piecemeal.
- A curated easing set (`--ease-1`…`--ease-5`, spring, elastic) and named animations, treating motion as tokens.
- Adaptive props: one name (`--surface-1`) whose value flips with the scheme.

## What went wrong / limits
- Numbered scales (`--size-3`) carry no meaning; teams still need a semantic layer on top.
- Niche adoption compared with Tailwind.

## Lessons for swaypplet
- swaypplet's motion tokens named by meaning (`--motion-enter`) are ahead of Open Props' numbered easings; keep them.
- Adaptive props are what swaypplet's semantic tier already is.

## Sources
- https://open-props.style/ — library [verified via search summary]
- https://github.com/argyleink/open-props/ — adaptive props, normalize [verified via search summary]
