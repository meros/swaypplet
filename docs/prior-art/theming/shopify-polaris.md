---
name: Shopify Polaris tokens
slug: shopify-polaris
domain: theming
kind: project
platform: web
vendor: Shopify
years: 2017–present
status: current
tags: [tokens, dark-mode, deferral, admin-ui]
relevance: low
---

## What it is
Shopify's design system for the merchant admin. Polaris tokens (`--p-color-bg-surface`, `--p-space-400`) were reconciled in v12 (2023) alongside a visual refresh; Polaris later moved to web components.

## What was new
- Numeric token names by multiple of the base (`space-400` = 16 px) so scales can be extended in between without renames.
- Public, versioned token package with migration tooling (`polaris-migrator`).

## What went wrong / limits
- Dark mode was planned repeatedly (a 2019 colour-system proposal, a dark-mode coordination issue) and deferred out of the v12 token reconciliation; the admin shipped for years without it.
- Lesson in cost: once components hard-code assumptions about a light surface, adding a mode later is a rewrite.

## Lessons for swaypplet
- swaypplet built modes into the generator from the start (principle 8); Polaris shows why that is the cheap order.

## Sources
- https://github.com/Shopify/polaris-react/issues/9894 — v12 token reconciliation, dark mode out of scope [verified via search summary]
- https://github.com/Shopify/polaris/issues/2042 — dark mode coordination [verified via search summary]
