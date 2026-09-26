---
name: Atlassian design tokens and dark mode migration
slug: atlassian-tokens
domain: theming
kind: pattern
platform: web
vendor: Atlassian
years: 2021–present
status: current
tags: [tokens, migration, lint, codemod, dark-mode]
relevance: medium
---

## What it is
Atlassian introduced semantic tokens (`color.text.subtle`, `elevation.surface.raised`) to ship dark mode across Jira, Confluence and Marketplace apps. Tokens are consumed via a `token()` function with fallbacks; themes switch via `data-color-mode` and `data-theme` attributes.

## What was new
- Enforcement tooling as part of the system: an ESLint plugin and a Stylelint plugin that flag hard-coded colours and auto-fix to tokens, plus codemods (`css-to-design-tokens`, `theme-to-design-tokens`).
- Third-party apps embedded in Jira inherit the host's theme through the same tokens.

## What went wrong / limits
- Codemods only suggest tokens; manual review was needed at scale, and dark mode took years to cover the products.
- Developer-community reports of the published dark tokens CSS drifting from Jira's.

## Lessons for swaypplet
- swaypplet's lint (`rust-ui-class`, semantic tier only) is the same idea and is what makes a system hold; extend it to Cairo drawing code (no literal RGB in `paint.rs` callers).

## Sources
- https://atlassian.design/tokens/migrate-to-tokens/ — lint plugins, codemods [verified via search summary]
- https://community.developer.atlassian.com/t/dark-mode-tokens-css-file-out-of-sync-with-jira/97499 — drift [verified via search summary]
