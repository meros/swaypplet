---
name: GitHub Primer themes (colourblind, dimmed, high contrast)
slug: github-primer
domain: theming
kind: project
platform: web
vendor: GitHub
years: 2021–present
status: current
tags: [themes, colourblind, high-contrast, functional-tokens, primitives]
relevance: high
---

## What it is
Primer Primitives defines base and functional colour tokens (`fgColor-default`, `bgColor-success-emphasis`, `borderColor-muted`) and ships many themes from them: light, dark, dark dimmed, light/dark high contrast, protanopia & deuteranopia, and tritanopia (2022). Users pick separate themes for day and night and GitHub follows the OS.

## What was new
- Colour-vision themes as first-class modes: red/green swapped for orange/blue so diff, CI and PR status stay distinguishable.
- Separate day and night theme choices, so "dark" can be dimmed or high contrast.
- Functional tokens named by role and emphasis (`-muted`, `-emphasis`), not by hue.

## What went wrong / limits
- Colourblind themes produced reports of confusing PR status in light protanopia mode, and a tritanopia user reported the first scheme as inaccessible.
- Many themes multiply testing; some combinations regress unnoticed.

## Lessons for swaypplet
- Add a colour-vision input for status: success/danger as blue/orange under a "deuteranopia" preset. The status set is fixed and small, and the categorical slots are already hue-rotatable, so the cost is a table plus the existing APCA test.
- Separate day and night choices maps onto auto mode: let the user pick which neutral or contrast each half uses.

## Sources
- https://github.blog/changelog/2022-04-19-protanopia-deuteranopia-colorblind-themes-beta/ — swap to orange/blue [verified via search summary]
- https://github.blog/changelog/2022-04-19-tritanopia-colorblind-theme-beta/ — tritanopia theme [verified via search summary]
- https://github.com/orgs/community/discussions/146487 — PR status confusion [verified via search summary]
- https://primer.style/foundations/primitives — functional tokens [memory]
