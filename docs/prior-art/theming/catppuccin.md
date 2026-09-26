---
name: Catppuccin
slug: catppuccin
domain: theming
kind: project
platform: cross
vendor: Catppuccin community
years: 2021–present
status: current
tags: [palette, community, ports, flavors, pastel, nix]
relevance: medium
---

## What it is
A pastel palette in four flavours (Latte light; Frappé, Macchiato, Mocha dark), each 26 named colours (14 accents such as Rosewater, Mauve, Peach, plus text, subtext, overlay, surface, base, mantle, crust). Ported by the community to a few hundred applications, with an official `catppuccin/nix` module.

## What was new
- Named, role-described colours with a published style guide per port, so ports agree on what "surface0" or "overlay1" means.
- Organisation as product: a GitHub org with per-port repos, review rules, a palette package (JSON, npm, crates), and a Nix module that themes apps declaratively.
- Four flavours as a contrast ladder: users pick how dark, not only dark vs light.

## What went wrong / limits
- Pastel accents on Latte have weak contrast for text; ports vary in quality.
- Consistency relies on reviewers, not tooling.

## Lessons for swaypplet
- A neutral preset named "catppuccin" (Mocha anchors) is a cheap, popular addition; the generator keeps its identity through the anchors as with gruvbox.
- The "flavours as a darkness ladder" idea fits swaypplet's neutral input.

## Sources
- https://github.com/catppuccin/catppuccin — project, flavours [verified via search summary]
- https://catppuccin.com/palette/ — palette [verified via search summary]
- catppuccin/nix module [memory]
