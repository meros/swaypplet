---
name: W3C Design Tokens Community Group format
slug: dtcg-design-tokens
domain: theming
kind: protocol
platform: cross
vendor: W3C Design Tokens Community Group
years: 2019–present
status: current
tags: [tokens, json, interchange, aliases, theming, oklch]
relevance: medium
---

## What it is
A JSON format for exchanging design tokens between tools. Tokens have `$value`, `$type` (color, dimension, fontFamily, duration, cubicBezier, shadow, typography, ...), `$description`, and `{alias.references}`. The first stable version, 2025.10, was announced on 28 October 2025, with a separate resolver/theming module for modes and brands.

## What was new
- A vendor-neutral file format backed by Adobe, Figma, Google, Microsoft, Shopify, Salesforce and others; supported by Penpot, Figma, Sketch, Framer, Style Dictionary.
- Colour values as objects with an explicit colour space (sRGB, Display P3, OKLCH), not only hex.
- Composite types (typography, shadow, transition) and aliases give a tiered system a standard shape.

## What went wrong / limits
- Six years from first draft to stable; tools implemented incompatible drafts in the meantime.
- It is static data: it cannot express generation rules (a scale from an anchor, a contrast solve), only their output.

## Lessons for swaypplet
- swaypplet's generator is richer than DTCG can express; but emitting a DTCG JSON beside `tokens.css` (per mode and tint) makes the palette consumable by editors, terminals and Figma/Penpot, and gives a machine-readable diff of token changes.
- Keep generation in Rust; use DTCG as an output, not a source.

## Sources
- https://www.w3.org/community/design-tokens/2025/10/28/design-tokens-specification-reaches-first-stable-version/ — stable release [verified via search summary]
- https://www.designtokens.org/tr/drafts/format/ — format module [verified via search summary]
